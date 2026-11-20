use linchuan_core::App;
use serde::Serialize;
use tauri::AppHandle;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use tauri::Manager;

#[derive(Serialize)]
pub struct DesktopSettings {
    supported: bool,
    autostart: bool,
    close_to_tray: bool,
    tray_available: bool,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod platform {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    use tauri_plugin_autostart::ManagerExt;

    pub struct DesktopRuntime {
        pub close_to_tray: AtomicBool,
        pub tray_available: AtomicBool,
        pub quitting: AtomicBool,
    }
    impl Default for DesktopRuntime {
        fn default() -> Self {
            Self {
                close_to_tray: AtomicBool::new(true),
                tray_available: AtomicBool::new(false),
                quitting: AtomicBool::new(false),
            }
        }
    }

    pub fn autostart_requested() -> bool {
        std::env::args_os().any(|value| value == "--autostart")
    }
    pub fn show(app: &AppHandle) -> Result<(), String> {
        let window = app.get_webview_window("main").ok_or("主窗口不可用")?;
        window.show().map_err(|e| e.to_string())?;
        window.unminimize().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())
    }
    pub fn quit(app: &AppHandle) {
        app.state::<DesktopRuntime>()
            .quitting
            .store(true, Ordering::SeqCst);
        app.exit(0);
    }
    fn create_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        let icon = app.default_window_icon().ok_or("应用图标不可用")?;
        let show = MenuItem::with_id(app, "show-main", "打开邻传", true, None::<&str>)?;
        let stop = MenuItem::with_id(app, "stop-assistance", "停止远程协助", true, None::<&str>)?;
        let exit = MenuItem::with_id(app, "quit-app", "退出邻传", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &stop, &exit])?;
        let tray = TrayIconBuilder::with_id("linchuan-main")
            .icon(icon.clone())
            .tooltip("邻传 · 正在运行")
            .menu(&menu)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "show-main" => {
                    let _ = super::show(app);
                }
                "stop-assistance" => crate::assistance::stop_desktop(app),
                "quit-app" => quit(app),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                ) {
                    let _ = super::show(tray.app_handle());
                }
            });
        #[cfg(target_os = "windows")]
        let tray = tray.show_menu_on_left_click(false);
        tray.build(app)?;
        Ok(())
    }
    pub fn setup(app: &mut tauri::App) -> tauri::Result<()> {
        let handle = app.handle().clone();
        let available = match create_tray(&handle) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("托盘创建失败，将使用普通窗口：{error}");
                false
            }
        };
        handle
            .state::<DesktopRuntime>()
            .tray_available
            .store(available, Ordering::SeqCst);
        if !available || !autostart_requested() {
            let _ = show(&handle);
        }
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::ensure_core(&handle).await {
                eprintln!("邻传后台服务启动失败：{error}");
                handle
                    .state::<DesktopRuntime>()
                    .close_to_tray
                    .store(false, Ordering::SeqCst);
                let _ = show(&handle);
            }
        });
        Ok(())
    }
    pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
        if window.label() != "main" {
            return;
        }
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            let state = window.state::<DesktopRuntime>();
            if state.close_to_tray.load(Ordering::SeqCst)
                && state.tray_available.load(Ordering::SeqCst)
                && !state.quitting.load(Ordering::SeqCst)
            {
                // Prevent closure only after hiding succeeds, so a failed hide can still exit.
                if window.hide().is_ok() {
                    api.prevent_close();
                }
            }
        }
    }
    pub async fn settings(app: &AppHandle) -> Result<DesktopSettings, String> {
        let state = app.state::<DesktopRuntime>();
        let close_to_tray = state.close_to_tray.load(Ordering::SeqCst);
        let tray_available = state.tray_available.load(Ordering::SeqCst);
        let handle = app.clone();
        let autostart = tauri::async_runtime::spawn_blocking(move || {
            handle.autolaunch().is_enabled().map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(DesktopSettings {
            supported: true,
            autostart,
            close_to_tray,
            tray_available,
        })
    }
    pub async fn set_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let manager = handle.autolaunch();
            if enabled {
                manager.enable()
            } else {
                manager.disable()
            }
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub use platform::{on_window_event, setup, show, DesktopRuntime};

pub fn configure_context(context: &mut tauri::Context<tauri::Wry>) {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if platform::autostart_requested() {
        for window in &mut context.config_mut().app.windows {
            if window.label == "main" {
                window.visible = false;
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let _ = context;
}
pub async fn initialize(app: &AppHandle, core: &App) -> Result<(), String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    app.state::<DesktopRuntime>().close_to_tray.store(
        core.close_to_tray().await.map_err(|e| e.to_string())?,
        std::sync::atomic::Ordering::SeqCst,
    );
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let _ = (app, core);
    Ok(())
}
#[tauri::command]
pub async fn desktop_settings(app: AppHandle) -> Result<DesktopSettings, String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        crate::ensure_core(&app).await?;
        platform::settings(&app).await
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = app;
        Ok(DesktopSettings {
            supported: false,
            autostart: false,
            close_to_tray: false,
            tray_available: false,
        })
    }
}
#[tauri::command]
pub async fn desktop_set_autostart(
    app: AppHandle,
    enabled: bool,
) -> Result<DesktopSettings, String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        platform::set_autostart(&app, enabled).await?;
        platform::settings(&app).await
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (app, enabled);
        Err("此平台不支持开机启动设置".into())
    }
}
#[tauri::command]
pub async fn desktop_set_close_to_tray(
    app: AppHandle,
    enabled: bool,
) -> Result<DesktopSettings, String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        let core = crate::ensure_core(&app).await?;
        core.set_close_to_tray(enabled)
            .await
            .map_err(|e| e.to_string())?;
        app.state::<DesktopRuntime>()
            .close_to_tray
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
        platform::settings(&app).await
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (app, enabled);
        Err("此平台不支持托盘设置".into())
    }
}
#[tauri::command]
pub fn desktop_quit(app: AppHandle) -> Result<(), String> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        platform::quit(&app);
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = app;
        Err("此平台不支持桌面退出命令".into())
    }
}
