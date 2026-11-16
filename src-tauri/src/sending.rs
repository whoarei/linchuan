use crate::Runtime;
use linchuan_core::files::SelectedFile;
use tauri::State;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use tauri::{Emitter, Manager};

#[tauri::command]
pub async fn choose_send_files(
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
) -> Result<Vec<SelectedFile>, String> {
    let core = runtime.0.get().ok_or("服务尚未就绪")?.clone();
    #[cfg(target_os = "android")]
    {
        #[derive(serde::Deserialize)]
        struct Selection {
            location: String,
            name: String,
            size: u64,
            mime: String,
        }
        #[derive(serde::Deserialize)]
        struct Picks {
            #[serde(default)]
            files: Vec<Selection>,
        }
        let picks: Picks =
            crate::receiving::mobile(app, "pickFiles", serde_json::json!({})).await?;
        let mut files = Vec::new();
        for f in picks.files {
            files.push(
                core.register_file(f.location, f.name, f.mime, Some(f.size))
                    .await?,
            );
        }
        Ok(files)
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_dialog::DialogExt;
        let selected = tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_title("选择要发送的文件")
                .blocking_pick_files()
        })
        .await
        .map_err(|e| e.to_string())?;
        register_paths(
            &core,
            selected
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.into_path().map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }
}

#[cfg(not(target_os = "android"))]
async fn register_paths(
    core: &linchuan_core::App,
    paths: Vec<std::path::PathBuf>,
) -> Result<Vec<SelectedFile>, String> {
    if paths.len() > 64 {
        return Err("一次最多发送 64 个文件".into());
    }
    let mut files = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .ok_or("文件名无效")?
            .to_string_lossy()
            .into_owned();
        files.push(
            core.register_file(
                path.to_string_lossy().into_owned(),
                name,
                String::new(),
                None,
            )
            .await?,
        );
    }
    Ok(files)
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::DragDrop(event) = event {
        let _ = window.emit(
            "linchuan-dragging",
            matches!(
                event,
                tauri::DragDropEvent::Enter { .. } | tauri::DragDropEvent::Over { .. }
            ),
        );
        if let tauri::DragDropEvent::Drop { paths, .. } = event {
            let app = window.app_handle().clone();
            let paths = paths.clone();
            tauri::async_runtime::spawn(async move {
                let result = async {
                    let core = crate::ensure_core(&app).await?;
                    register_paths(&core, paths).await
                }
                .await;
                match result {
                    Ok(files) => {
                        let _ = app.emit("linchuan-files", files);
                    }
                    Err(error) => {
                        let _ = app.emit("linchuan-file-error", error);
                    }
                }
            });
        }
    }
}
