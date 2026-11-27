use crate::Runtime;
use linchuan_core::{
    receive_conflicts::{ConflictChoices, FileConflict},
    receiving::{SaveJob, SaveLocation, SavedFile},
    App,
};
use serde::Serialize;
use tauri::{Manager, State};

#[cfg(target_os = "android")]
struct AndroidStorage(tauri::plugin::PluginHandle<tauri::Wry>);

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("receiving")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            app.manage(AndroidStorage(api.register_android_plugin(
                "io.github.whoarei.linchuan",
                "ReceivingPlugin",
            )?));
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
pub(crate) async fn mobile<T: serde::de::DeserializeOwned + Send + 'static>(
    app: tauri::AppHandle,
    command: &'static str,
    args: serde_json::Value,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AndroidStorage>()
            .0
            .run_mobile_plugin::<T>(command, args)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn default_directory(app: &tauri::AppHandle) -> String {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        "@downloads".into()
    }
    #[cfg(not(target_os = "android"))]
    {
        app.path()
            .download_dir()
            .or_else(|_| app.path().home_dir().map(|p| p.join("Downloads")))
            .map(|p| p.join("邻传").to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

pub async fn start(app: tauri::AppHandle, core: App) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use linchuan_core::files::{PlatformFiles, PlatformOutput};
        use std::os::fd::FromRawFd;
        use std::sync::Arc;
        #[derive(serde::Deserialize)]
        struct Descriptor {
            fd: i32,
            location: String,
        }
        #[derive(serde::Deserialize)]
        struct Input {
            fd: i32,
        }
        let inspect_app = app.clone();
        let open_app = app.clone();
        let begin_app = app.clone();
        let finish_app = app.clone();
        let abort_app = app.clone();
        core.set_platform_files(PlatformFiles {
            open: Arc::new(move |location| {
                let app = open_app.clone();
                Box::pin(async move {
                    let input: Input =
                        mobile(app, "openInput", serde_json::json!({"location":location})).await?;
                    Ok(unsafe { std::fs::File::from_raw_fd(input.fd) })
                })
            }),
            inspect: Arc::new(move |directory, name| {
                let app = inspect_app.clone();
                Box::pin(async move {
                    #[derive(serde::Deserialize)]
                    struct Inspection { entry: Option<linchuan_core::receive_conflicts::ExistingEntry> }
                    Ok(mobile::<Inspection>(app, "inspectFile", serde_json::json!({"directory": directory, "name":name})).await?.entry)
                })
            }),
            begin: Arc::new(move |target, mime| {
                let app = begin_app.clone();
                Box::pin(async move {
                    let output: Descriptor = mobile(
                        app,
                        "beginFile",
                        serde_json::json!({"directory":target.directory,"name":target.name,"mime":mime,"strict":true,"replace":target.replace}),
                    )
                    .await?;
                    Ok(PlatformOutput {
                        file: unsafe { std::fs::File::from_raw_fd(output.fd) },
                        location: output.location,
                    })
                })
            }),
            finish: Arc::new(move |location, target| {
                let app = finish_app.clone();
                Box::pin(async move {
                    let saved: SaveLocation = mobile(
                        app,
                        "completeFile",
                        serde_json::json!({"location":location,"directory":target.directory,"name":target.name,"replace":target.replace}),
                    )
                    .await?;
                    Ok(saved)
                })
            }),
            abort: Arc::new(move |location, target| {
                let app = abort_app.clone();
                Box::pin(async move {
                    mobile::<serde_json::Value>(
                        app,
                        "abortFile",
                        serde_json::json!({"location":location,"name":target.as_ref().map_or("", |t| t.name.as_str()),"replace":target.as_ref().and_then(|t| t.replace.as_deref())}),
                    )
                    .await?;
                    Ok(())
                })
            }),
        });
        mobile::<serde_json::Value>(app.clone(), "recoverReplacements", serde_json::json!({}))
            .await?;
        let handle = app.clone();
        core.set_space_probe(std::sync::Arc::new(move |directory| {
            let handle = handle.clone();
            Box::pin(async move {
                if directory == "@downloads" || directory.starts_with("content://") {
                    mobile::<linchuan_core::storage::DiskSpace>(
                        handle,
                        "storageInfo",
                        serde_json::json!({"directory":directory}),
                    )
                    .await
                } else {
                    tauri::async_runtime::spawn_blocking(move || {
                        linchuan_core::storage::disk_space(std::path::Path::new(&directory))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                }
            })
        }));
    }
    core.configure_receiving(default_directory(&app))
        .await
        .map_err(|e| e.to_string())?;
    core.recover_receive_outputs().await?;
    tauri::async_runtime::spawn(async move {
        core.run_receive_saver(move |job| save(app.clone(), job))
            .await;
    });
    Ok(())
}

async fn save(app: tauri::AppHandle, job: SaveJob) -> Result<SaveLocation, String> {
    #[cfg(target_os = "android")]
    {
        let input = if job.source.to_string_lossy().starts_with("content://") {
            use std::os::fd::FromRawFd;
            #[derive(serde::Deserialize)]
            struct Input {
                fd: i32,
            }
            let source: Input = mobile(
                app.clone(),
                "openInput",
                serde_json::json!({"location":job.source}),
            )
            .await?;
            unsafe { std::fs::File::from_raw_fd(source.fd) }
        } else {
            std::fs::File::open(&job.source).map_err(|e| e.to_string())?
        };
        save_android(app, job, input).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        tauri::async_runtime::spawn_blocking(move || linchuan_core::receiving::save_to_folder(job))
            .await
            .map_err(|e| e.to_string())?
    }
}

#[cfg(target_os = "android")]
async fn save_android(
    app: tauri::AppHandle,
    job: SaveJob,
    input: std::fs::File,
) -> Result<SaveLocation, String> {
    use std::os::fd::FromRawFd;
    #[derive(serde::Deserialize)]
    struct Output {
        fd: i32,
        location: String,
    }
    let target = job.target.as_ref().filter(|t| t.directory == job.directory);
    let name = target.map_or(job.name.as_str(), |t| t.name.as_str());
    let replace = target.and_then(|t| t.replace.as_deref());
    let mut args = serde_json::json!({"name":name,"mime":job.mime,"directory":job.directory,"strict":job.target.is_some(),"replace":replace});
    let output: Output = mobile(app.clone(), "beginFile", args.clone()).await?;
    // Kotlin detaches ownership of this descriptor to Rust in beginFile.
    let target = unsafe { std::fs::File::from_raw_fd(output.fd) };
    let result = tauri::async_runtime::spawn_blocking(move || {
        linchuan_core::receiving::copy_stream(input, target, job.progress.as_ref())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|v| v);
    args["location"] = serde_json::json!(output.location);
    let value = match result {
        Ok(()) => mobile(app.clone(), "completeFile", args.clone()).await,
        Err(error) => Err(error),
    };
    if value.is_err() {
        let _ = mobile::<serde_json::Value>(app, "abortFile", args).await;
    }
    value
}

async fn delete_location(app: tauri::AppHandle, location: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        mobile::<serde_json::Value>(app, "deleteFile", serde_json::json!({"location":location}))
            .await
            .map(|_| ())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        tauri::async_runtime::spawn_blocking(move || {
            std::fs::remove_file(location).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

async fn pick_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        #[derive(serde::Deserialize)]
        struct Pick {
            directory: Option<String>,
        }
        mobile::<Pick>(app, "pickDirectory", serde_json::json!({}))
            .await
            .map(|v| v.directory)
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_dialog::DialogExt;
        tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_title("选择目标目录")
                .blocking_pick_folder()
                .map(|file| {
                    file.into_path()
                        .map(|p| p.to_string_lossy().into_owned())
                        .map_err(|e| e.to_string())
                })
        })
        .await
        .map_err(|e| e.to_string())?
        .transpose()
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum ReceiveResult {
    Done(bool),
    Conflicts {
        directory: String,
        conflicts: Vec<FileConflict>,
    },
}

#[tauri::command]
pub async fn receive_transfer_as(
    app: tauri::AppHandle,
    transfer_id: String,
    file_ids: Option<Vec<String>>,
    directory: Option<String>,
    choices: Option<ConflictChoices>,
    runtime: State<'_, Runtime>,
) -> Result<ReceiveResult, String> {
    let core = runtime.0.get().ok_or("服务尚未就绪")?;
    let directory = match directory {
        Some(d) => d,
        None => match pick_directory(app).await? {
            Some(d) => d,
            None => return Ok(ReceiveResult::Done(false)),
        },
    };
    match core
        .accept_files_with_choices(
            &transfer_id,
            directory.clone(),
            file_ids,
            choices.unwrap_or_default(),
        )
        .await
    {
        Ok(()) => Ok(ReceiveResult::Done(true)),
        Err(e) if !e.conflicts().is_empty() => Ok(ReceiveResult::Conflicts {
            directory,
            conflicts: e.conflicts().to_vec(),
        }),
        Err(e) => Err(e.to_string()),
    }
}

static FILE_OPERATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tauri::command]
pub async fn relocate_received_file(
    app: tauri::AppHandle,
    message_id: String,
    file_id: String,
    transfer_id: String,
    move_file: bool,
    runtime: State<'_, Runtime>,
) -> Result<Option<SaveLocation>, String> {
    let _operation = FILE_OPERATIONS.lock().await;
    let core = runtime.0.get().ok_or("服务尚未就绪")?;
    let source = core
        .saved_files(vec![message_id.clone()])
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|r| r.file_id == file_id && r.status == "saved");
    if move_file && source.is_none() {
        return Err("文件尚未保存完成".into());
    }
    let Some(directory) = pick_directory(app.clone()).await? else {
        return Ok(None);
    };
    let _permit = core.active_file_operation().await?;
    let mut job = if let Some(record) = &source {
        SaveJob {
            target: None,
            message_id: message_id.clone(),
            file_id: file_id.clone(),
            source: record.location.clone().into(),
            name: linchuan_core::receiving::safe_name(&record.name),
            mime: record.mime.clone(),
            directory: directory.clone(),
            size: record.size,
            progress: None,
        }
    } else {
        let mut job = core.cached_file(&transfer_id, &file_id).await?;
        if job.message_id != message_id {
            return Err("文件不属于此消息".into());
        }
        job.directory = directory.clone();
        job
    };
    #[cfg(target_os = "android")]
    let input = {
        use std::os::fd::FromRawFd;
        #[derive(serde::Deserialize)]
        struct Input {
            fd: i32,
        }
        let value: Input = mobile(
            app.clone(),
            "openInput",
            serde_json::json!({"location":job.source.to_string_lossy()}),
        )
        .await?;
        // openInput detaches the descriptor; this File now owns and closes it.
        unsafe { std::fs::File::from_raw_fd(value.fd) }
    };
    #[cfg(not(target_os = "android"))]
    let input =
        std::fs::File::open(&job.source).map_err(|e| format!("文件已移动、删除或无法读取：{e}"))?;
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    if metadata.is_file() {
        job.size = metadata.len();
    }
    let size = job.size;
    let copy_app = app.clone();
    let copy = move |progress| async move {
        job.progress = progress;
        #[cfg(target_os = "android")]
        {
            save_android(copy_app, job, input).await
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = copy_app;
            // Hold the original open until the operation finishes; the folder
            // helper reads the same source path and never overwrites a target.
            let _input = input;
            tauri::async_runtime::spawn_blocking(move || {
                linchuan_core::receiving::save_to_folder(job)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    };
    let value = if let Some(record) = source {
        core.relocate_saved_file(record, size, directory, move_file, copy, move |location| {
            delete_location(app.clone(), location)
        })
        .await?
    } else {
        let tag = uuid_tag();
        let progress = core.reserve_copy(tag.clone(), directory, size).await?;
        let result = copy(progress).await;
        core.finish_copy(&tag);
        result?
    };
    Ok(Some(value))
}

fn uuid_tag() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("copy:{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

#[derive(Serialize)]
pub struct ReceiveSettings {
    directory: String,
    label: String,
}
async fn settings(app: tauri::AppHandle, core: &App) -> Result<ReceiveSettings, String> {
    let directory = core.receive_directory().await.map_err(|e| e.to_string())?;
    #[cfg(target_os = "android")]
    let label = {
        #[derive(serde::Deserialize)]
        struct Label {
            label: String,
        }
        mobile::<Label>(
            app,
            "describeDirectory",
            serde_json::json!({"directory":directory}),
        )
        .await
        .map(|v| v.label)
        .unwrap_or_else(|_| directory.clone())
    };
    #[cfg(not(target_os = "android"))]
    let label = {
        let _ = app;
        directory.clone()
    };
    Ok(ReceiveSettings { directory, label })
}
#[tauri::command]
pub async fn receive_settings(
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
) -> Result<ReceiveSettings, String> {
    settings(app, runtime.0.get().ok_or("服务尚未就绪")?).await
}
#[tauri::command]
pub async fn choose_receive_directory(
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
) -> Result<ReceiveSettings, String> {
    let core = runtime.0.get().ok_or("服务尚未就绪")?;
    #[cfg(target_os = "android")]
    let selected = {
        #[derive(serde::Deserialize)]
        struct Pick {
            directory: Option<String>,
        }
        mobile::<Pick>(app.clone(), "pickDirectory", serde_json::json!({}))
            .await?
            .directory
    };
    #[cfg(not(target_os = "android"))]
    let selected = {
        use tauri_plugin_dialog::DialogExt;
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            handle
                .dialog()
                .file()
                .set_title("选择接收目录")
                .blocking_pick_folder()
                .map(|file| file.into_path().map(|p| p.to_string_lossy().into_owned()))
        })
        .await
        .map_err(|e| e.to_string())?
        .transpose()
        .map_err(|e| e.to_string())?
    };
    if let Some(directory) = selected {
        core.set_receive_directory(directory)
            .await
            .map_err(|e| e.to_string())?;
    }
    settings(app, core).await
}
#[tauri::command]
pub async fn reset_receive_directory(
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
) -> Result<ReceiveSettings, String> {
    let core = runtime.0.get().ok_or("服务尚未就绪")?;
    core.set_receive_directory(default_directory(&app))
        .await
        .map_err(|e| e.to_string())?;
    settings(app, core).await
}
#[tauri::command]
pub async fn received_files(
    messages: Vec<String>,
    runtime: State<'_, Runtime>,
) -> Result<Vec<SavedFile>, String> {
    runtime
        .0
        .get()
        .ok_or("服务尚未就绪")?
        .saved_files(messages)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn save_received_file(
    transfer_id: String,
    file_id: String,
    runtime: State<'_, Runtime>,
) -> Result<(), String> {
    runtime
        .0
        .get()
        .ok_or("服务尚未就绪")?
        .queue_save_file(&transfer_id, &file_id)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn open_received_file(
    app: tauri::AppHandle,
    message_id: String,
    file_id: String,
    folder: bool,
    runtime: State<'_, Runtime>,
) -> Result<(), String> {
    let core = runtime.0.get().ok_or("服务尚未就绪")?;
    let record = core
        .saved_files(vec![message_id.clone()])
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|f| f.file_id == file_id && f.status == "saved");
    let record = if let Some(record) = record {
        record
    } else {
        let job = core
            .outgoing_file(message_id.clone(), file_id.clone())
            .await?;
        SavedFile {
            message_id,
            file_id,
            name: job.name,
            directory: job.directory,
            location: job.source.to_string_lossy().into_owned(),
            status: "saved".into(),
            error: None,
            mime: job.mime,
            size: job.size,
        }
    };
    #[cfg(target_os = "android")]
    {
        let _: serde_json::Value = mobile(
            app,
            if folder { "openDirectory" } else { "openFile" },
            serde_json::json!({"location":record.location,"directory":record.directory}),
        )
        .await?;
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_opener::OpenerExt;
        let path = std::path::PathBuf::from(record.location);
        if !path.is_file() {
            return Err("文件已在外部移动或删除，原位置已失效".into());
        }
        if folder {
            app.opener()
                .reveal_item_in_dir(path)
                .map_err(|e| e.to_string())
        } else {
            app.opener()
                .open_path(path.to_string_lossy(), None::<&str>)
                .map_err(|e| e.to_string())
        }
    }
}
#[tauri::command]
pub async fn open_receive_directory(
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
) -> Result<(), String> {
    let directory = runtime
        .0
        .get()
        .ok_or("服务尚未就绪")?
        .receive_directory()
        .await
        .map_err(|e| e.to_string())?;
    open_directory(app, directory).await
}

#[tauri::command]
pub async fn open_scan_directory(
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
    id: String,
) -> Result<(), String> {
    let directory = runtime
        .0
        .get()
        .ok_or("服务尚未就绪")?
        .scan_receive_directory(&id)?;
    open_directory(app, directory).await
}

async fn open_directory(app: tauri::AppHandle, directory: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let _: serde_json::Value = mobile(
            app,
            "openDirectory",
            serde_json::json!({"directory":directory,"location":""}),
        )
        .await?;
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_opener::OpenerExt;
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        app.opener()
            .open_path(directory, None::<&str>)
            .map_err(|e| e.to_string())
    }
}
