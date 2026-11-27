use crate::{api::Error, model::FileMeta, receiving::safe_name, App};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::ErrorKind,
    path::Path,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExistingEntry {
    pub location: Option<String>,
    pub stamp: String,
    pub size: Option<u64>,
    pub replaceable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileConflict {
    pub id: String,
    pub name: String,
    pub key: String,
    pub existing_size: Option<u64>,
    pub replaceable: bool,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    Replace,
    KeepBoth,
}
pub type ConflictChoices = HashMap<String, ConflictChoice>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReceiveTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_location: Option<String>,
    pub directory: String,
    pub name: String,
    pub replace: Option<String>,
}

pub(crate) fn inspect_local(path: &Path) -> Result<Option<ExistingEntry>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let stamp = format!(
                "{:?}/{:?}/{}",
                meta.created().ok(),
                meta.modified().ok(),
                meta.len()
            );
            Ok(Some(ExistingEntry {
                location: None,
                stamp,
                size: meta.is_file().then_some(meta.len()),
                replaceable: meta.is_file() && !meta.permissions().readonly(),
            }))
        }
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => Ok(None),
        Err(e) => Err(format!("无法检查目标文件：{e}")),
    }
}

pub(crate) fn name_key(name: &str) -> String {
    if cfg!(any(windows, target_os = "android", target_os = "macos")) {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

pub(crate) fn numbered(name: &str, index: usize) -> String {
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    format!("{stem} ({index}){ext}")
}

impl App {
    pub(crate) async fn inspect_entry(
        &self,
        directory: &str,
        name: &str,
    ) -> Result<Option<ExistingEntry>, Error> {
        if directory == "@downloads" || directory.starts_with("content://") {
            let io = self
                .0
                .file_io
                .read()
                .unwrap()
                .clone()
                .ok_or_else(|| Error::bad("接收目录不可用"))?;
            (io.inspect)(directory.into(), name.into())
                .await
                .map_err(Error::bad)
        } else {
            let path = Path::new(directory).join(name);
            tokio::task::spawn_blocking(move || inspect_local(&path))
                .await
                .map_err(|e| Error::bad(e.to_string()))?
                .map_err(Error::bad)
        }
    }

    pub(crate) async fn plan_receive(
        &self,
        directory: &str,
        files: &[&FileMeta],
        active: &[ReceiveTarget],
        choices: &ConflictChoices,
    ) -> Result<HashMap<String, ReceiveTarget>, Error> {
        let mut claimed: HashSet<_> = active
            .iter()
            .filter(|t| t.directory == directory)
            .map(|t| name_key(&t.name))
            .collect();
        // A generated copy name must not take another attachment's original name.
        let originals: HashSet<_> = files
            .iter()
            .map(|f| name_key(&safe_name(&f.name)))
            .collect();
        let mut conflicts = Vec::new();
        let mut targets = HashMap::new();
        for file in files {
            let mut name = safe_name(&file.name);
            let occupied = !claimed.insert(name_key(&name));
            let existing = self.inspect_entry(directory, &name).await?;
            let mut replace = None;
            if occupied || existing.is_some() {
                let stamp = existing.as_ref().map(|e| e.stamp.as_str()).unwrap_or("");
                let key = crate::identity::token_hash(&format!(
                    "{directory}\0{}\0{name}\0{occupied}\0{stamp}",
                    file.id
                ));
                let replaceable = !occupied && existing.as_ref().is_some_and(|e| e.replaceable);
                match choices.get(&key) {
                    Some(ConflictChoice::Replace) if replaceable => {
                        replace = Some(stamp.to_owned())
                    }
                    Some(ConflictChoice::KeepBoth) => {
                        let mut index = 1;
                        loop {
                            if index >= 10000 {
                                return Err(Error::bad("同名文件过多，请更换接收目录"));
                            }
                            let candidate = numbered(&name, index);
                            let key = name_key(&candidate);
                            if !originals.contains(&key)
                                && !claimed.contains(&key)
                                && self.inspect_entry(directory, &candidate).await?.is_none()
                            {
                                claimed.insert(key);
                                name = candidate;
                                break;
                            }
                            index += 1;
                        }
                    }
                    _ => conflicts.push(FileConflict {
                        id: file.id.clone(),
                        name: name.clone(),
                        key,
                        replaceable,
                        existing_size: existing.as_ref().and_then(|e| e.size),
                        reason: if occupied {
                            "另一个待接收文件使用了此名称"
                        } else if replaceable {
                            "已有同名文件"
                        } else {
                            "同名项目无法覆盖"
                        },
                    }),
                }
            }
            targets.insert(
                file.id.clone(),
                ReceiveTarget {
                    previous_location: if replace.is_some() {
                        existing.and_then(|e| e.location)
                    } else {
                        None
                    },
                    directory: directory.into(),
                    name,
                    replace,
                },
            );
        }
        if !conflicts.is_empty() {
            return Err(Error::file_conflicts(conflicts));
        }
        Ok(targets)
    }
}
