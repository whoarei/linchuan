use crate::Runtime;
use linchuan_core::{
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
async fn mobile<T: serde::de::DeserializeOwned + Send + 'static>(
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
    core.configure_receiving(default_directory(&app))
        .await
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(async move {
        core.run_receive_saver(move |job| save(app.clone(), job))
            .await;
    });
    Ok(())
}

async fn save(app: tauri::AppHandle, job: SaveJob) -> Result<SaveLocation, String> {
    #[cfg(target_os = "android")]
    {
        mobile(
            app,
            "saveFile",
            serde_json::json!({
                "sourcePath": job.source.to_string_lossy(),
                "name": job.name,
                "mime": job.mime,
                "directory": job.directory,
            }),
        )
        .await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        tauri::async_runtime::spawn_blocking(move || linchuan_core::receiving::save_to_folder(job))
            .await
            .map_err(|e| e.to_string())?
    }
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
    let record = runtime
        .0
        .get()
        .ok_or("服务尚未就绪")?
        .saved_files(vec![message_id])
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|f| f.file_id == file_id && f.status == "saved")
        .ok_or("文件尚未保存完成")?;
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
            return Err("文件已移动或删除，可点击保存副本重新保存".into());
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
