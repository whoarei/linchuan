//! File ownership and platform I/O. User files never enter cache cleanup.
use crate::{
    api::Error,
    model::*,
    receive_conflicts::{ExistingEntry, ReceiveTarget},
    receiving::SaveLocation,
    App,
};
use axum::body::Body;
use futures_util::StreamExt;
use serde::Serialize;
use std::{future::Future, path::Path, pin::Pin, sync::Arc};
use tokio::io::AsyncWriteExt;

pub type IoFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;
#[derive(Clone)]
pub struct PlatformFiles {
    pub open: Arc<dyn Fn(String) -> IoFuture<std::fs::File> + Send + Sync>,
    pub inspect: Arc<dyn Fn(String, String) -> IoFuture<Option<ExistingEntry>> + Send + Sync>,
    pub begin: Arc<dyn Fn(ReceiveTarget, String) -> IoFuture<PlatformOutput> + Send + Sync>,
    pub finish: Arc<dyn Fn(String, ReceiveTarget) -> IoFuture<SaveLocation> + Send + Sync>,
    pub abort: Arc<dyn Fn(String, Option<ReceiveTarget>) -> IoFuture<()> + Send + Sync>,
}
pub struct PlatformOutput {
    pub file: std::fs::File,
    pub location: String,
}
#[derive(Clone, Serialize)]
pub struct SelectedFile {
    pub source_id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

fn modified(meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|v| v.as_millis().min(u64::MAX as u128) as u64)
}
fn identity(file: &std::fs::File) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().ok()?;
        Some(format!("{}:{}", meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return None;
        }
        Some(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
}

impl App {
    pub async fn recover_receive_outputs(&self) -> Result<(), String> {
        let rows = self
            .0
            .store
            .run(|db, _| {
                let mut q = db.prepare(
                    "SELECT delivery_id,file_id,source_json,published FROM receive_outputs",
                )?;
                let rows = q
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, bool>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .map_err(|e| e.to_string())?;
        let platform = self.0.file_io.read().unwrap().clone();
        for (id, file, json, published) in rows {
            let source: StoredFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            let FileStorage::Received { directory, name } = &source.storage else {
                continue;
            };
            if published {
                let directory = directory.clone();
                let name = name.clone();
                let location = source.path.to_string_lossy().into_owned();
                self.0.store.run(move |db,_| {
                    db.execute("INSERT OR IGNORE INTO saved_files(message_id,file_id,transfer_id,name,directory,location,status,updated_at) SELECT message_id,?1,id,?2,?3,?4,'saved',?5 FROM deliveries WHERE id=?6",rusqlite::params![file,name,directory,location,now_ms(),id])?;
                    Ok(())
                }).await.map_err(|e|e.to_string())?;
            } else {
                let location = source.path.to_string_lossy().into_owned();
                if location.starts_with("content://") {
                    let Some(io) = &platform else { continue };
                    if (io.abort)(location, source.receive_target.clone())
                        .await
                        .is_err()
                    {
                        continue;
                    }
                } else {
                    let path = &source.path;
                    let owned = path.parent() == Some(Path::new(directory))
                        && path.file_name().is_some_and(|n| {
                            let n = n.to_string_lossy();
                            n.starts_with(".linchuan-") && n.ends_with(".part")
                        });
                    if !owned {
                        continue;
                    }
                    match tokio::fs::remove_file(path).await {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => continue,
                    }
                }
                self.0.store.run(move |db,_|{db.execute("DELETE FROM receive_outputs WHERE delivery_id=?1 AND file_id=?2 AND published=0",rusqlite::params![id,file])?;Ok(())}).await.map_err(|e|e.to_string())?;
            }
        }
        Ok(())
    }
    pub fn set_platform_files(&self, files: PlatformFiles) {
        *self.0.file_io.write().unwrap() = Some(files);
    }
    pub async fn register_file(
        &self,
        location: String,
        name: String,
        mime: String,
        size: Option<u64>,
    ) -> Result<SelectedFile, String> {
        let mime = if mime.is_empty() {
            mime_guess::from_path(&name)
                .first_or_octet_stream()
                .to_string()
        } else {
            mime
        };
        let input = self.open_location(&location).await?;
        let meta = input.metadata().map_err(|e| e.to_string())?;
        if !location.starts_with("content://") && !meta.is_file() {
            return Err("请选择普通文件".into());
        }
        let size = size.unwrap_or(meta.len());
        if size > crate::storage::MAX_EXACT_SIZE {
            return Err("文件过大".into());
        }
        let id = uid();
        let file = StoredFile {
            receive_target: None,
            meta: FileMeta {
                id: id.clone(),
                name: name.clone(),
                size,
                mime: mime.clone(),
            },
            path: location.into(),
            complete: true,
            uploading: false,
            storage: FileStorage::Original {
                identity: if meta.is_file() {
                    identity(&input)
                } else {
                    None
                },
                modified: if meta.is_file() {
                    modified(&meta)
                } else {
                    None
                },
            },
            downloaded: false,
        };
        let mut sources = self.0.sources.lock().unwrap();
        sources.retain(|_, (_, at)| now().saturating_sub(*at) < 24 * 3600);
        if sources.len() >= 4096 {
            return Err("待发送的文件过多，请重新启动应用".into());
        }
        sources.insert(id.clone(), (file, now()));
        Ok(SelectedFile {
            source_id: id,
            name,
            size,
            mime,
        })
    }
    pub(crate) async fn open_location(&self, location: &str) -> Result<std::fs::File, String> {
        if location.starts_with("content://") {
            let io = self
                .0
                .file_io
                .read()
                .unwrap()
                .clone()
                .ok_or("当前平台无法读取文档 URI")?;
            (io.open)(location.into()).await
        } else {
            let location = location.to_owned();
            tokio::task::spawn_blocking(move || {
                std::fs::File::open(location)
                    .map_err(|e| format!("文件已移动、删除或无法读取：{e}"))
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
    pub(crate) async fn open_source(&self, file: &StoredFile) -> Result<tokio::fs::File, String> {
        if !file.complete || matches!(file.storage, FileStorage::Browser | FileStorage::Receiving) {
            return Err("文件来源尚未就绪".into());
        }
        let input = self.open_location(&file.path.to_string_lossy()).await?;
        Self::check_source(file, &input)?;
        Ok(tokio::fs::File::from_std(input))
    }
    pub(crate) fn check_source(file: &StoredFile, input: &std::fs::File) -> Result<(), String> {
        let meta = input.metadata().map_err(|e| e.to_string())?;
        if meta.is_file() && meta.len() != file.meta.size {
            return Err("原文件大小已变化，请重新选择发送".into());
        }
        if let FileStorage::Original {
            modified: Some(expected),
            ..
        } = file.storage
        {
            if modified(&meta) != Some(expected) {
                return Err("原文件已修改，请重新选择发送".into());
            }
        }
        if let FileStorage::Original {
            identity: Some(expected),
            ..
        } = &file.storage
        {
            if identity(input).as_ref() != Some(expected) {
                return Err("原文件已被替换，请重新选择发送".into());
            }
        }
        Ok(())
    }
    pub async fn outgoing_file(
        &self,
        message_id: String,
        file_id: String,
    ) -> Result<crate::receiving::SaveJob, String> {
        let owner = self.0.device_id.clone();
        let message = message_id.clone();
        let transfer:String=self.0.store.run(move |db,_|Ok(db.query_row("SELECT d.id FROM deliveries d JOIN messages m ON m.id=d.message_id WHERE m.id=?1 AND m.sender_id=?2 LIMIT 1",rusqlite::params![message,owner],|r|r.get(0))?)).await.map_err(|e|e.to_string())?;
        self.cached_file(&transfer, &file_id).await.map(|mut job| {
            job.message_id = message_id;
            job.directory = job
                .source
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            job
        })
    }
    pub(crate) async fn target_directory(&self) -> Result<String, Error> {
        let directory = self
            .receive_directory()
            .await
            .map_err(|e| Error::storage(e.to_string()))?;
        Ok(if directory.is_empty() {
            self.0
                .directory
                .join("received")
                .to_string_lossy()
                .into_owned()
        } else {
            directory
        })
    }
    pub(crate) async fn receive_stream(
        &self,
        id: &str,
        file: &StoredFile,
        body: Body,
    ) -> Result<StoredFile, Error> {
        let target = file
            .receive_target
            .clone()
            .ok_or_else(|| Error::conflict("接收任务尚未就绪"))?;
        let directory = target.directory.clone();
        let name = target.name.clone();
        let platform = self.0.file_io.read().unwrap().clone();
        let mobile = directory == "@downloads" || directory.starts_with("content://");
        let (output, location) = if mobile {
            let io = platform
                .as_ref()
                .ok_or_else(|| Error::bad("接收目录不可用"))?;
            let output = (io.begin)(target.clone(), file.meta.mime.clone())
                .await
                .map_err(Error::bad)?;
            (output.file, output.location)
        } else {
            let dir = directory.clone();
            tokio::task::spawn_blocking(move || -> Result<_, Error> {
                std::fs::create_dir_all(&dir)?;
                let temp = tempfile::Builder::new()
                    .prefix(".linchuan-")
                    .suffix(".part")
                    .tempfile_in(dir)?;
                let (file, path) = temp.keep().map_err(|e| Error::bad(e.to_string()))?;
                Ok((file, path.to_string_lossy().into_owned()))
            })
            .await
            .map_err(|e| Error::bad(e.to_string()))??
        };
        let mut saved = file.clone();
        saved.path = location.clone().into();
        saved.storage = FileStorage::Received {
            directory: if mobile && target.replace.is_some() && !location.starts_with("content://")
            {
                saved
                    .path
                    .parent()
                    .ok_or_else(|| Error::bad("临时文件位置无效"))?
                    .to_string_lossy()
                    .into_owned()
            } else {
                directory.clone()
            },
            name: name.clone(),
        };
        saved.complete = false;
        saved.uploading = false;
        if let Err(error) = self.output_journal(id, &saved, false).await {
            drop(output);
            if mobile {
                if let Some(io) = &platform {
                    let _ = (io.abort)(location, file.receive_target.clone()).await;
                }
            } else {
                let _ = tokio::fs::remove_file(location).await;
            }
            return Err(error);
        }
        let mut output = tokio::fs::File::from_std(output);
        let result = async {
            let space = self.space_progress(&format!("receive:{id}:{}", file.meta.id), &directory);
            let mut stream = body.into_data_stream();
            let mut count = 0u64;
            let mut updated = std::time::Instant::now();
            let mut deadline = tokio::time::Instant::now() + crate::storage::IDLE_TIMEOUT;
            loop {
                let chunk = tokio::time::timeout_at(deadline, stream.next())
                    .await
                    .map_err(|_| Error::bad("连续 5 分钟未收到文件数据，请重新发送"))?;
                let Some(chunk) = chunk else { break };
                let chunk = chunk.map_err(|_| Error::bad("文件传输中断"))?;
                if chunk.is_empty() {
                    continue;
                }
                deadline = tokio::time::Instant::now() + crate::storage::IDLE_TIMEOUT;
                count = count
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| Error::bad("文件过大"))?;
                if count > file.meta.size {
                    return Err(Error::bad("上传内容超过声明的大小"));
                }
                output.write_all(&chunk).await?;
                if let Some(space) = &space {
                    space.set(count);
                }
                if updated.elapsed().as_millis() >= 200 {
                    if let Some(space) = &space {
                        space.check().map_err(Error::conflict)?;
                    }
                    self.stream_progress(id, &file.meta.id, count).await?;
                    updated = std::time::Instant::now();
                }
            }
            if count != file.meta.size {
                return Err(Error::bad("文件不完整，请重新发送"));
            }
            self.stream_progress(id, &file.meta.id, count).await?;
            output.flush().await?;
            if output.metadata().await?.is_file() {
                output.sync_all().await?;
            }
            Ok(())
        }
        .await;
        drop(output);
        if let Err(error) = result {
            if mobile {
                if let Some(io) = &platform {
                    let _ = (io.abort)(location, file.receive_target.clone()).await;
                }
            } else {
                let _ = tokio::fs::remove_file(&location).await;
            }
            self.release_space_tag(&format!("receive:{id}:{}", file.meta.id));
            return Err(error);
        }
        let _publication = if target.replace.is_some() {
            Some(self.0.publishing_replacement.lock().await)
        } else {
            None
        };
        let (location, actual_name) = if mobile {
            let io = platform.as_ref().unwrap();
            match (io.finish)(location.clone(), target).await {
                Ok(saved) => (saved.location, saved.name),
                Err(error) => {
                    let _ = (io.abort)(location, file.receive_target.clone()).await;
                    return Err(Error::bad(error));
                }
            }
        } else {
            tokio::task::spawn_blocking(move || {
                let temp =
                    tempfile::TempPath::try_from_path(&location).map_err(|e| e.to_string())?;
                let saved = crate::receiving::publish_file(temp, &target)?;
                Ok((saved.location, saved.name))
            })
            .await
            .map_err(|e| Error::bad(e.to_string()))?
            .map_err(|e: String| Error::bad(e))?
        };
        saved.path = location.into();
        saved.storage = FileStorage::Received {
            directory,
            name: actual_name,
        };
        saved.complete = true;
        // Checkpoint the published location before acknowledging the network request.
        // Recovery preserves this user file even if the following delivery commit fails.
        self.output_journal(id, &saved, true).await?;
        if !mobile {
            if let FileStorage::Received { directory, .. } = &saved.storage {
                crate::sync_directory(directory.into()).await?;
            }
        }
        self.release_space_tag(&format!("receive:{id}:{}", file.meta.id));
        Ok(saved)
    }
    async fn output_journal(
        &self,
        id: &str,
        file: &StoredFile,
        published: bool,
    ) -> Result<(), Error> {
        let id = id.to_owned();
        let file_id = file.meta.id.clone();
        let json = serde_json::to_string(file).map_err(|e| Error::bad(e.to_string()))?;
        let replaced = file
            .receive_target
            .as_ref()
            .filter(|t| published && t.replace.is_some())
            .map(|t| {
                t.previous_location
                    .clone()
                    .unwrap_or_else(|| file.path.to_string_lossy().into_owned())
            });
        self.0.store.run(move |db, _| {
            let tx = db.transaction()?;
            if let Some(location) = replaced { crate::store::mark_replaced(&tx, &location)?; }
            tx.execute("INSERT INTO receive_outputs VALUES(?1,?2,?3,?4) ON CONFLICT(delivery_id,file_id) DO UPDATE SET source_json=excluded.source_json,published=excluded.published", rusqlite::params![id,file_id,json,published])?;
            tx.commit()?;
            Ok(())
        }).await.map_err(|e| Error::storage(e.to_string()))
    }
    pub(crate) async fn stream_progress(
        &self,
        id: &str,
        file: &str,
        bytes: u64,
    ) -> Result<(), Error> {
        let mut state = self.0.state.lock().await;
        let t = state.transfers.get_mut(id).ok_or_else(Error::missing)?;
        if t.view.terminal() {
            return Err(Error::conflict("传输已结束"));
        }
        t.stream_bytes.insert(file.into(), bytes);
        t.refresh_stream_bytes(false);
        t.view.created = now();
        drop(state);
        self.notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn abandoned_browser_source_times_out_and_releases_destination_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let mut options = crate::Options::new(directory.path().into());
        options.discovery = false;
        let app = App::start(options).await.unwrap();
        let id = uid();
        let browser = format!("browser-{}", uid());
        let file = uid();
        let lease = app
            .0
            .space
            .reserve(vec![(
                format!("receive:{id}:{file}"),
                app.cache_location(),
                1,
            )])
            .await
            .unwrap();
        app.0
            .space_leases
            .lock()
            .unwrap()
            .insert(format!("incoming:{id}"), lease);
        let mut state = app.0.state.lock().await;
        state.profiles.insert(
            browser.clone(),
            Device {
                id: browser.clone(),
                name: "网页".into(),
                kind: "browser".into(),
                host_device_id: app.0.device_id.clone(),
                ..Default::default()
            },
        );
        let meta = FileMeta {
            id: file,
            name: "file.bin".into(),
            size: 1,
            mime: String::new(),
        };
        state.transfers.insert(
            id.clone(),
            Transfer {
                view: TransferView {
                    id: id.clone(),
                    message_id: uid(),
                    group_id: None,
                    created_at: now_ms(),
                    sender_id: browser,
                    sender_name: "网页".into(),
                    recipient_id: app.0.device_id.clone(),
                    recipient_name: "本机".into(),
                    text: String::new(),
                    files: vec![meta.clone()],
                    status: "accepted".into(),
                    can_select_files: false,
                    selected_file_ids: None,
                    bytes: 0,
                    total: 1,
                    created: now() - 301,
                    error: None,
                },
                files: vec![StoredFile {
                    receive_target: None,
                    meta,
                    path: Default::default(),
                    complete: false,
                    uploading: false,
                    storage: FileStorage::Browser,
                    downloaded: false,
                }],
                stream_bytes: Default::default(),
                peer_token: None,
                incoming_remote: false,
                reserved: 0,
            },
        );
        drop(state);
        assert!(app.has_active_work().await);
        app.sweep().await;
        assert_eq!(
            app.0.state.lock().await.transfers[&id].view.status,
            "interrupted"
        );
        assert!(!app.has_active_work().await);
        assert!(app.0.space_leases.lock().unwrap().is_empty());
        app.stop();
    }
}
