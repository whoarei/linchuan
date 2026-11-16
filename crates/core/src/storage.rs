//! Reserve only bytes that have not reached disk yet, grouped by filesystem.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock, Weak,
    },
};

pub const DISK_HEADROOM: u64 = 2 * 1024 * 1024 * 1024;
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
// JSON numbers must survive browser round trips, and SQLite uses signed integers.
pub const MAX_EXACT_SIZE: u64 = (1u64 << 53) - 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiskSpace {
    pub volume: String,
    pub available: u64,
    pub path: PathBuf,
}
type ProbeFuture = Pin<Box<dyn Future<Output = Result<DiskSpace, String>> + Send>>;
pub type SpaceProbe = Arc<dyn Fn(String) -> ProbeFuture + Send + Sync>;

pub fn disk_space(path: &Path) -> Result<DiskSpace, String> {
    let mut ancestor = std::path::absolute(path).map_err(|e| e.to_string())?;
    while !ancestor.is_dir() {
        if !ancestor.pop() {
            return Err("无法找到目标磁盘".into());
        }
    }
    let available =
        fs2::available_space(&ancestor).map_err(|e| format!("无法读取磁盘剩余空间：{e}"))?;
    #[cfg(unix)]
    let volume = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "unix:{}",
            std::fs::metadata(&ancestor)
                .map_err(|e| e.to_string())?
                .dev()
        )
    };
    #[cfg(windows)]
    let volume = {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
        };
        let path: Vec<u16> = ancestor.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut mount = vec![0u16; 32768];
        let mut name = vec![0u16; 32768];
        // The input and output buffers are NUL-terminated and stay live for both calls.
        unsafe {
            if GetVolumePathNameW(path.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if GetVolumeNameForVolumeMountPointW(
                mount.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
            ) == 0
            {
                // UNC shares need not expose a volume GUID; their mount path is stable.
                name = mount;
            }
        }
        String::from_utf16_lossy(&name[..name.iter().position(|&v| v == 0).unwrap_or(name.len())])
            .to_lowercase()
    };
    Ok(DiskSpace {
        volume,
        available,
        path: ancestor,
    })
}

#[derive(Clone)]
struct Charge {
    tag: String,
    location: String,
    disk: DiskSpace,
    bytes: u64,
    written: Arc<AtomicU64>,
}
impl Charge {
    fn remaining(&self) -> u64 {
        self.bytes
            .saturating_sub(self.written.load(Ordering::SeqCst))
    }
}
struct BudgetInner {
    charges: Mutex<HashMap<String, Vec<Charge>>>,
    gate: tokio::sync::Mutex<()>,
    probe: RwLock<SpaceProbe>,
}
#[derive(Clone)]
pub struct SpaceBudget(Arc<BudgetInner>);
impl Default for SpaceBudget {
    fn default() -> Self {
        Self(Arc::new(BudgetInner {
            charges: Mutex::new(HashMap::new()),
            gate: tokio::sync::Mutex::new(()),
            probe: RwLock::new(Arc::new(|path| {
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || disk_space(Path::new(&path)))
                        .await
                        .map_err(|e| e.to_string())?
                })
            })),
        }))
    }
}
struct Lease {
    id: String,
    budget: Weak<BudgetInner>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(budget) = self.budget.upgrade() {
            budget.charges.lock().unwrap().remove(&self.id);
        }
    }
}
#[derive(Clone)]
pub struct Reservation(Arc<Lease>);
#[derive(Clone)]
pub struct Progress {
    charge: Charge,
    reservation: Reservation,
}
impl Progress {
    pub fn set(&self, bytes: u64) {
        self.charge
            .written
            .store(bytes.min(self.charge.bytes), Ordering::SeqCst);
    }
    pub fn check(&self) -> Result<(), String> {
        self.reservation.check()
    }
}
impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiskProgress")
    }
}
impl Reservation {
    pub fn finished(&self) -> bool {
        self.0.budget.upgrade().is_none_or(|budget| {
            budget
                .charges
                .lock()
                .unwrap()
                .get(&self.0.id)
                .is_none_or(|rows| rows.iter().all(|c| c.remaining() == 0))
        })
    }
    pub fn progress(&self, tag: &str, location: &str) -> Option<Progress> {
        let budget = self.0.budget.upgrade()?;
        let rows = budget.charges.lock().unwrap();
        rows.get(&self.0.id)?
            .iter()
            .find(|c| c.tag == tag && c.location == location)
            .cloned()
            .map(|charge| Progress {
                charge,
                reservation: self.clone(),
            })
    }
    pub fn release_tag(&self, tag: &str) {
        if let Some(budget) = self.0.budget.upgrade() {
            if let Some(rows) = budget.charges.lock().unwrap().get(&self.0.id) {
                for c in rows.iter().filter(|c| c.tag == tag) {
                    c.written.store(c.bytes, Ordering::SeqCst);
                }
            }
        }
    }
    pub fn check(&self) -> Result<(), String> {
        let budget = self.0.budget.upgrade().ok_or("空间预留已结束")?;
        let (remaining, _) = snapshot(&budget);
        let own = budget
            .charges
            .lock()
            .unwrap()
            .get(&self.0.id)
            .cloned()
            .unwrap_or_default();
        for c in own {
            let free = fs2::available_space(&c.disk.path)
                .map_err(|e| format!("无法读取磁盘剩余空间：{e}"))?;
            ensure_space(
                &c.disk.path,
                free,
                *remaining.get(&c.disk.volume).unwrap_or(&0),
            )?;
        }
        Ok(())
    }
}

impl crate::App {
    pub fn finish_copy(&self, tag: &str) {
        self.release_space_tag(tag);
    }
    pub fn set_space_probe(&self, probe: SpaceProbe) {
        self.0.space.set_probe(probe);
    }
    pub fn cache_location(&self) -> String {
        self.0.directory.to_string_lossy().into_owned()
    }
    pub(crate) fn space_progress(&self, tag: &str, location: &str) -> Option<Progress> {
        self.0
            .space_leases
            .lock()
            .unwrap()
            .values()
            .find_map(|lease| lease.progress(tag, location))
    }
    pub(crate) fn release_space_tag(&self, tag: &str) {
        let mut leases = self.0.space_leases.lock().unwrap();
        for lease in leases.values() {
            lease.release_tag(tag);
        }
        leases.retain(|_, lease| !lease.finished());
    }
    pub async fn reserve_copy(
        &self,
        tag: String,
        directory: String,
        size: u64,
    ) -> Result<Option<Progress>, String> {
        if size == 0 {
            return Ok(None);
        }
        if let Some(progress) = self.space_progress(&tag, &directory) {
            return Ok(Some(progress));
        }
        self.release_space_tag(&tag);
        let lease = self
            .0
            .space
            .reserve(vec![(tag.clone(), directory.clone(), size)])
            .await?;
        let progress = lease.progress(&tag, &directory);
        self.0.space_leases.lock().unwrap().insert(tag, lease);
        Ok(progress)
    }
}
// Snapshot counters BEFORE querying free space: writes during the query then make
// the estimate conservative rather than subtracting the same bytes twice.
fn snapshot(budget: &BudgetInner) -> (HashMap<String, u64>, HashMap<String, DiskSpace>) {
    let mut remaining: HashMap<String, u64> = HashMap::new();
    let mut disks = HashMap::new();
    for c in budget.charges.lock().unwrap().values().flatten() {
        let entry = remaining.entry(c.disk.volume.clone()).or_default();
        *entry = entry.saturating_add(c.remaining());
        disks.insert(c.disk.volume.clone(), c.disk.clone());
    }
    (remaining, disks)
}
fn ensure_space(path: &Path, free: u64, needed: u64) -> Result<(), String> {
    if free < needed.saturating_add(DISK_HEADROOM) {
        return Err(format!("磁盘空间不足（{}）：可用 {:.2} GiB，任务尚需 {:.2} GiB，另预留 2 GiB。可清理缓存或选择其他磁盘。",
            path.display(), free as f64 / 1073741824.0, needed as f64 / 1073741824.0));
    }
    Ok(())
}
impl SpaceBudget {
    pub fn set_probe(&self, probe: SpaceProbe) {
        *self.0.probe.write().unwrap() = probe;
    }
    pub async fn reserve(
        &self,
        requests: Vec<(String, String, u64)>,
    ) -> Result<Reservation, String> {
        let _gate = self.0.gate.lock().await;
        let (mut remaining, _) = snapshot(&self.0);
        let probe = self.0.probe.read().unwrap().clone();
        let mut charges = Vec::new();
        let mut free_by_volume: HashMap<String, (DiskSpace, u64)> = HashMap::new();
        for (tag, location, bytes) in requests.into_iter().filter(|(_, _, size)| *size > 0) {
            let disk = probe(location.clone()).await?;
            let count = remaining.entry(disk.volume.clone()).or_default();
            *count = count.checked_add(bytes).ok_or("文件总大小超出可表示范围")?;
            let entry = free_by_volume
                .entry(disk.volume.clone())
                .or_insert((disk.clone(), disk.available));
            entry.1 = entry.1.min(disk.available);
            charges.push(Charge {
                tag,
                location,
                disk,
                bytes,
                written: Arc::new(AtomicU64::new(0)),
            });
        }
        for (volume, (disk, free)) in free_by_volume {
            ensure_space(&disk.path, free, remaining[&volume])?;
        }
        let id = crate::model::uid();
        self.0.charges.lock().unwrap().insert(id.clone(), charges);
        Ok(Reservation(Arc::new(Lease {
            id,
            budget: Arc::downgrade(&self.0),
        })))
    }
    pub async fn available(&self, path: String) -> Result<u64, String> {
        let (remaining, _) = snapshot(&self.0);
        let probe = self.0.probe.read().unwrap().clone();
        let disk = probe(path).await?;
        Ok(disk
            .available
            .saturating_sub(DISK_HEADROOM)
            .saturating_sub(*remaining.get(&disk.volume).unwrap_or(&0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GIB: u64 = 1024 * 1024 * 1024;

    fn budget(free: Arc<AtomicU64>, separate: bool) -> SpaceBudget {
        let budget = SpaceBudget::default();
        budget.set_probe(Arc::new(move |location| {
            let free = free.clone();
            Box::pin(async move {
                Ok(DiskSpace {
                    volume: if separate {
                        location.clone()
                    } else {
                        "same-disk".into()
                    },
                    available: free.load(Ordering::SeqCst),
                    path: std::env::temp_dir(),
                })
            })
        }));
        budget
    }

    #[tokio::test]
    async fn same_disk_counts_cache_and_export_together_and_other_tasks_cannot_reuse_space() {
        let free = Arc::new(AtomicU64::new(22 * GIB));
        let budget = budget(free, false);
        let lease = budget
            .reserve(vec![
                ("cache".into(), "app".into(), 10 * GIB),
                ("export".into(), "downloads".into(), 10 * GIB),
            ])
            .await
            .unwrap();
        assert!(budget
            .reserve(vec![("other".into(), "app".into(), 1)])
            .await
            .is_err());
        drop(lease);
        assert!(budget
            .reserve(vec![("other".into(), "app".into(), 20 * GIB)])
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn separate_disks_and_written_bytes_are_accounted_without_double_counting() {
        let free = Arc::new(AtomicU64::new(12 * GIB));
        let budget = budget(free.clone(), true);
        let lease = budget
            .reserve(vec![
                ("cache".into(), "app".into(), 10 * GIB),
                ("export".into(), "downloads".into(), 10 * GIB),
            ])
            .await
            .unwrap();
        lease.progress("cache", "app").unwrap().set(4 * GIB);
        free.store(8 * GIB, Ordering::SeqCst);
        assert_eq!(budget.available("app".into()).await.unwrap(), 0);
        // Six GiB remain reserved and two GiB are kept free. The four written
        // GiB have already disappeared from disk free space.
        assert!(budget
            .reserve(vec![("new".into(), "app".into(), 1)])
            .await
            .is_err());
        lease.progress("cache", "app").unwrap().set(10 * GIB);
        free.store(2 * GIB, Ordering::SeqCst);
        assert_eq!(budget.available("app".into()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn simultaneous_admission_reserves_space_atomically() {
        let budget = budget(Arc::new(AtomicU64::new(12 * GIB)), false);
        let (a, b) = tokio::join!(
            budget.reserve(vec![("a".into(), "app".into(), 6 * GIB)]),
            budget.reserve(vec![("b".into(), "app".into(), 6 * GIB)])
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    }

    #[tokio::test]
    async fn sweep_keeps_active_uploads_and_releases_idle_expired_drafts() {
        use crate::model::*;
        let directory = tempfile::tempdir().unwrap();
        let mut options = crate::Options::new(directory.path().into());
        options.discovery = false;
        let app = crate::App::start(options).await.unwrap();
        let mut state = app.0.state.lock().await;
        let path = app.0.work.join("active");
        std::fs::create_dir(&path).unwrap();
        state.drafts.insert(
            "active".into(),
            Draft {
                id: "active".into(),
                owner: app.0.device_id.clone(),
                created: now() - 7200,
                sent: false,
                files: vec![StoredFile {
                    meta: FileMeta {
                        id: "f".into(),
                        name: "f".into(),
                        size: 0,
                        mime: String::new(),
                    },
                    path: path.join("f"),
                    complete: false,
                    uploading: true,
                    storage: crate::model::FileStorage::Cache,
                    downloaded: false,
                }],
            },
        );
        drop(state);
        app.sweep().await;
        assert!(path.is_dir());
        let mut state = app.0.state.lock().await;
        assert!(state.drafts.contains_key("active"));
        state.drafts.get_mut("active").unwrap().files[0].uploading = false;
        drop(state);
        app.sweep().await;
        assert!(!path.exists());
        app.stop();
    }
}
