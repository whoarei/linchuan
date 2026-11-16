use linchuan_core::{App, Bootstrap, Options};
use tauri::{Manager, State};
mod desktop;
mod receiving;
mod sending;
struct Runtime(tokio::sync::OnceCell<App>);
#[tauri::command]
fn desktop_window_controls() -> bool {
    cfg!(any(target_os = "windows", target_os = "linux"))
}
#[tauri::command]
async fn check_release(
    app: tauri::AppHandle,
) -> Result<linchuan_core::updates::ReleaseInfo, String> {
    use linchuan_core::updates::Package;
    let package = if cfg!(target_os = "android") {
        Package::Android
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Package::Windows
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        match tauri::utils::platform::bundle_type() {
            Some(tauri::utils::config::BundleType::AppImage) => Package::AppImage,
            Some(tauri::utils::config::BundleType::Deb) => Package::Deb,
            _ => Package::Manual,
        }
    } else {
        Package::Manual
    };
    linchuan_core::updates::check(&app.package_info().version.to_string(), package).await
}
#[tauri::command]
async fn prepare_update(state: State<'_, Runtime>) -> Result<(), String> {
    if let Some(core) = state.0.get() {
        if core.has_active_work().await {
            return Err("有文件正在准备或传输，请完成后再安装更新".into());
        }
    }
    Ok(())
}
async fn ensure_core(app: &tauri::AppHandle) -> Result<App, String> {
    let state = app.state::<Runtime>();
    let core = state
        .0
        .get_or_try_init(|| async {
            let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let core = App::start(Options::new(directory))
                .await
                .map_err(|e| e.to_string())?;
            let initialized = async {
                desktop::initialize(app, &core).await?;
                receiving::start(app.clone(), core.clone()).await
            }
            .await;
            if let Err(error) = initialized {
                core.stop();
                return Err(error);
            }
            Ok::<_, String>(core)
        })
        .await?;
    Ok(core.clone())
}
#[tauri::command]
async fn bootstrap(app: tauri::AppHandle) -> Result<Bootstrap, String> {
    Ok(ensure_core(&app).await?.bootstrap())
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(receiving::plugin());
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _| {
        if !args.iter().any(|arg| arg == "--autostart") {
            let _ = desktop::show(app);
        }
    }));
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder
        .manage(desktop::DesktopRuntime::default())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("邻传")
                .arg("--autostart")
                .build(),
        )
        .setup(|app| {
            desktop::setup(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            desktop::on_window_event(window, event);
            sending::on_window_event(window, event);
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());
    let mut context = tauri::generate_context!();
    desktop::configure_context(&mut context);
    builder
        .manage(Runtime(tokio::sync::OnceCell::new()))
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            desktop_window_controls,
            check_release,
            prepare_update,
            sending::choose_send_files,
            desktop::desktop_settings,
            desktop::desktop_set_autostart,
            desktop::desktop_set_close_to_tray,
            desktop::desktop_quit,
            receiving::receive_settings,
            receiving::choose_receive_directory,
            receiving::reset_receive_directory,
            receiving::received_files,
            receiving::save_received_file,
            receiving::relocate_received_file,
            receiving::open_received_file,
            receiving::open_receive_directory
        ])
        .build(context)
        .expect("邻传启动失败")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(core) = app.state::<Runtime>().0.get() {
                    core.stop();
                }
            }
        });
}
