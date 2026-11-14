use crate::{model::now_ms, App};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SavedFile {
    pub message_id: String,
    pub file_id: String,
    pub name: String,
    pub directory: String,
    pub location: String,
    pub status: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct SaveJob {
    pub message_id: String,
    pub file_id: String,
    pub source: PathBuf,
    pub name: String,
    pub mime: String,
    pub directory: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SaveLocation {
    pub name: String,
    pub location: String,
}

impl App {
    pub async fn configure_receiving(&self, default: String) -> anyhow::Result<()> {
        self.0
            .store
            .run(move |c, _| {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT OR IGNORE INTO app_settings(key,value) VALUES('receive_directory',?1)",
                    [default],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO app_settings(key,value) VALUES('receive_since',?1)",
                    [now_ms().to_string()],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }
    pub async fn receive_directory(&self) -> anyhow::Result<String> {
        self.0
            .store
            .run(|c, _| {
                Ok(c.query_row(
                    "SELECT value FROM app_settings WHERE key='receive_directory'",
                    [],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default())
            })
            .await
    }
    pub async fn set_receive_directory(&self, directory: String) -> anyhow::Result<()> {
        anyhow::ensure!(!directory.is_empty(), "请选择接收目录");
        self.0.store.run(move |c, _| {
            let tx = c.transaction()?;
            tx.execute("INSERT INTO app_settings(key,value) VALUES('receive_directory',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[directory])?;
            tx.execute("INSERT OR IGNORE INTO app_settings(key,value) VALUES('receive_since',?1)", [now_ms().to_string()])?;
            tx.execute("UPDATE saved_files SET status='pending',error=NULL,updated_at=?1 WHERE status='failed'",[now_ms()])?;
            tx.commit()?;
            Ok(())
        }).await?;
        self.notify();
        Ok(())
    }
    pub async fn queue_save_file(&self, transfer_id: &str, file_id: &str) -> anyhow::Result<()> {
        self.cache_transfer(transfer_id).await?;
        let state = self.0.state.lock().await;
        let transfer = state
            .transfers
            .get(transfer_id)
            .filter(|t| {
                t.view.sender_id == self.0.device_id
                    || (t.view.recipient_id == self.0.device_id && t.view.status == "completed")
            })
            .ok_or_else(|| anyhow::anyhow!("该附件不属于本机或尚未接收完成"))?;
        let file = transfer
            .files
            .iter()
            .find(|f| f.meta.id == file_id && f.complete && f.path.is_file())
            .ok_or_else(|| anyhow::anyhow!("应用内副本已清理，无法重新保存"))?;
        let (message, id, transfer_id, name) = (
            transfer.view.message_id.clone(),
            file.meta.id.clone(),
            transfer.view.id.clone(),
            file.meta.name.clone(),
        );
        self.0.store.run(move |c, _| {
            c.execute("INSERT INTO saved_files(message_id,file_id,transfer_id,name,status,updated_at) VALUES(?1,?2,?3,?4,'pending',?5) ON CONFLICT(message_id,file_id) DO UPDATE SET status='pending',error=NULL,updated_at=excluded.updated_at",params![message,id,transfer_id,name,now_ms()])?;
            Ok(())
        }).await?;
        drop(state);
        self.notify();
        Ok(())
    }
    pub async fn saved_files(&self, messages: Vec<String>) -> anyhow::Result<Vec<SavedFile>> {
        anyhow::ensure!(messages.len() <= 200, "一次读取的消息过多");
        self.0.store.run(move |c, _| {
            let mut q = c.prepare("SELECT message_id,file_id,name,directory,location,status,error FROM saved_files WHERE message_id=?1")?;
            let mut files=Vec::new();
            for message in messages {
                files.extend(q.query_map([message], |r| Ok(SavedFile {message_id:r.get(0)?,file_id:r.get(1)?,name:r.get(2)?,directory:r.get(3)?,location:r.get(4)?,status:r.get(5)?,error:r.get(6)?}))?.collect::<rusqlite::Result<Vec<_>>>()?);
            }
            Ok(files)
        }).await
    }
    async fn receive_jobs(&self) -> anyhow::Result<Vec<SaveJob>> {
        self.0.store.run(|c, root| {
            let directory: String = c.query_row("SELECT value FROM app_settings WHERE key='receive_directory'",[],|r|r.get(0)).optional()?.unwrap_or_default();
            let mut q = c.prepare("SELECT s.message_id,s.file_id,a.name,a.mime,b.relative_path FROM saved_files s JOIN attachments a ON a.message_id=s.message_id AND a.file_id=s.file_id JOIN delivery_files df ON df.delivery_id=s.transfer_id AND df.file_id=s.file_id JOIN file_blobs b ON b.id=df.blob_id WHERE s.status='pending' ORDER BY s.updated_at LIMIT 32")?;
            let jobs = q.query_map([], |r| { let path:String=r.get(4)?; Ok(SaveJob {message_id:r.get(0)?,file_id:r.get(1)?,name:safe_name(&r.get::<_,String>(2)?),mime:r.get(3)?,source:root.join(path),directory:directory.clone()}) })?.collect::<rusqlite::Result<_>>()?;
            Ok(jobs)
        }).await
    }
    pub async fn run_receive_saver<F, Fut>(&self, mut save: F)
    where
        F: FnMut(SaveJob) -> Fut,
        Fut: Future<Output = Result<SaveLocation, String>>,
    {
        let mut events = self.0.events.subscribe();
        let mut stop = self.0.stop.subscribe();
        let mut completed: HashMap<(String, String), (SaveLocation, String)> = HashMap::new();
        loop {
            if *stop.borrow() {
                break;
            }
            if let Ok(jobs) = self.receive_jobs().await {
                for mut job in jobs {
                    if *stop.borrow() {
                        break;
                    }
                    let key = (job.message_id.clone(), job.file_id.clone());
                    // Count disk export as active work so an update cannot interrupt a copy.
                    let Ok(_permit) = self.0.streams.acquire().await else {
                        return;
                    };
                    let result = if let Some((value, directory)) = completed.get(&key) {
                        job.directory = directory.clone();
                        Ok(value.clone())
                    } else {
                        if let Ok(directory) = self.receive_directory().await {
                            job.directory = directory;
                        }
                        let result = save(job.clone()).await;
                        if let Ok(value) = &result {
                            completed.insert(key.clone(), (value.clone(), job.directory.clone()));
                        }
                        result
                    };
                    let success = result.is_ok();
                    let outcome = self.0.store.run(move |c, _| {
                        match result {
                            Ok(value) => { c.execute("UPDATE saved_files SET status='saved',name=?1,directory=?2,location=?3,error=NULL,updated_at=?4 WHERE message_id=?5 AND file_id=?6",params![value.name,job.directory,value.location,now_ms(),job.message_id,job.file_id])?; }
                            Err(error) => { c.execute("UPDATE saved_files SET status='failed',error=?1,updated_at=?2 WHERE message_id=?3 AND file_id=?4",params![error,now_ms(),job.message_id,job.file_id])?; }
                        }
                        Ok(())
                    }).await;
                    if outcome.is_ok() {
                        completed.remove(&key);
                        self.notify();
                    } else if success {
                        break;
                    }
                }
            }
            tokio::select! {
                _ = stop.changed() => break,
                _ = events.recv() => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
            }
        }
    }
}

pub fn safe_name(value: &str) -> String {
    let value: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "/\\<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut name = value.trim_end_matches([' ', '.']).to_string();
    if name.is_empty() {
        name = "file".into();
    }
    let extension = Path::new(&name)
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| s.len() <= 24)
        .map(str::to_string);
    if name.len() > 180 {
        while name.len() > 150 {
            name.pop();
        }
        if let Some(ext) = extension {
            name.push('.');
            name.push_str(&ext);
        }
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        name.insert(0, '_');
    }
    name
}

pub fn save_to_folder(job: SaveJob) -> Result<SaveLocation, String> {
    let save = || -> anyhow::Result<SaveLocation> {
        anyhow::ensure!(!job.directory.is_empty(), "请选择接收目录");
        let directory = Path::new(&job.directory);
        std::fs::create_dir_all(directory)?;
        let name = safe_name(&job.name);
        let mut temporary = tempfile::Builder::new()
            .prefix(".linchuan-")
            .suffix(".part")
            .tempfile_in(directory)?;
        let mut source = std::fs::File::open(job.source)?;
        std::io::copy(&mut source, temporary.as_file_mut())?;
        temporary.as_file_mut().sync_all()?;
        let path = Path::new(&name);
        let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("file");
        let extension = path.extension().and_then(|v| v.to_str());
        for number in 0..10000 {
            let candidate = if number == 0 {
                name.clone()
            } else {
                format!(
                    "{stem} ({number}){}",
                    extension.map(|e| format!(".{e}")).unwrap_or_default()
                )
            };
            let target = directory.join(&candidate);
            match temporary.persist_noclobber(&target) {
                Ok(file) => {
                    file.sync_all()?;
                    return Ok(SaveLocation {
                        name: candidate,
                        location: target.to_string_lossy().into_owned(),
                    });
                }
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    temporary = error.file;
                }
                Err(error) => return Err(error.error.into()),
            }
        }
        anyhow::bail!("同名文件过多，请更换接收目录")
    };
    save().map_err(|e| e.to_string())
}
