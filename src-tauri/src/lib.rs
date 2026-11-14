use linchuan_core::{App, Bootstrap, Options};
use tauri::{Manager, State};
struct Runtime(tokio::sync::OnceCell<App>);
#[tauri::command]
fn desktop_window_controls() -> bool {
    cfg!(any(target_os = "windows", target_os = "linux"))
}
#[tauri::command]
async fn bootstrap(state: State<'_, Runtime>, app: tauri::AppHandle) -> Result<Bootstrap, String> {
    let core = state
        .0
        .get_or_try_init(|| async {
            let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
            App::start(Options::new(directory))
                .await
                .map_err(|e| e.to_string())
        })
        .await?;
    Ok(core.bootstrap())
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }));
    builder
        .manage(Runtime(tokio::sync::OnceCell::new()))
        .invoke_handler(tauri::generate_handler![bootstrap, desktop_window_controls])
        .build(tauri::generate_context!())
        .expect("邻传启动失败")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(core) = app.state::<Runtime>().0.get() {
                    core.stop();
                }
            }
        });
}
