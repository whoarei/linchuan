use linchuan_core::assistance::Session;
use tauri::AppHandle;

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("screen-assistance")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                app.manage(android::Capture(api.register_android_plugin(
                    "io.github.whoarei.linchuan",
                    "ScreenAssistancePlugin",
                )?));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

#[tauri::command]
pub async fn assistance_session(app: AppHandle) -> Result<Option<Session>, String> {
    Ok(crate::ensure_core(&app).await?.assistance_session().await)
}

#[tauri::command]
pub async fn assistance_invite(
    app: AppHandle,
    peer_id: String,
    sdp: String,
) -> Result<String, String> {
    crate::ensure_core(&app)
        .await?
        .invite_to_screen_share(&peer_id, sdp)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assistance_join(app: AppHandle, id: String, sdp: String) -> Result<(), String> {
    crate::ensure_core(&app)
        .await?
        .join_screen_share(&id, sdp)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assistance_end(app: AppHandle, id: String) -> Result<(), String> {
    crate::assistance_input::revoke(&app, Some(&id));
    crate::ensure_core(&app).await?.end_screen_share(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn assistance_browser_https_setup(
    app: AppHandle,
    host: String,
    port: u16,
    certificate_pem: String,
    key_pem: String,
) -> Result<(), String> {
    crate::ensure_core(&app)
        .await?
        .save_browser_https(host, port, certificate_pem, key_pem)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    use tauri::Manager;
    if window.label() != "main" || !matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
        return;
    }
    stop_desktop(window.app_handle());
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn stop_desktop(app: &AppHandle) {
    use tauri::{Emitter, Manager};
    crate::assistance_input::revoke(app, None);
    crate::assistance_linux::assistance_linux_stop(app.clone(), None);
    let _ = app.emit("linchuan-assistance-stop", ());
    if let Some(core) = app.state::<crate::Runtime>().0.get().cloned() {
        tauri::async_runtime::spawn(async move { core.end_active_screen_share().await });
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct CaptureStatus {
    state: String,
    control: String,
    needs_accessibility: bool,
}

#[tauri::command]
pub async fn assistance_android_start(
    app: AppHandle,
    peer_id: String,
    token: String,
    entire_display: bool,
) -> Result<String, String> {
    #[cfg(target_os = "android")]
    return android::start(app, peer_id, token, entire_display).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, peer_id, token, entire_display);
        Err("当前系统无需 Android 屏幕共享服务".into())
    }
}

#[tauri::command]
pub async fn assistance_android_stop(app: AppHandle, token: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    android::stop(&app, &token).await;
    #[cfg(not(target_os = "android"))]
    let _ = (app, token);
    Ok(())
}

#[tauri::command]
pub async fn assistance_android_status(
    app: AppHandle,
    token: String,
) -> Result<CaptureStatus, String> {
    #[cfg(target_os = "android")]
    return android::state(&app, &token).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, token);
        Ok(CaptureStatus {
            state: "ended".into(),
            control: "unavailable".into(),
            needs_accessibility: false,
        })
    }
}

#[tauri::command]
pub async fn assistance_android_capabilities(
    app: AppHandle,
) -> Result<AndroidCapabilities, String> {
    #[cfg(target_os = "android")]
    return android::capabilities(&app).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(AndroidCapabilities::default())
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct AndroidCapabilities {
    single_app: bool,
    top: f64,
    bottom: f64,
    left: f64,
    right: f64,
}

#[tauri::command]
pub async fn assistance_android_control(
    app: AppHandle,
    token: String,
    action: String,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    return android::control(&app, &token, &action).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, token, action);
        Ok(())
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::time::{Duration, Instant};
    use tauri::Manager;

    pub struct Capture(pub tauri::plugin::PluginHandle<tauri::Wry>);

    async fn mobile<T: serde::de::DeserializeOwned + Send + 'static>(
        app: &AppHandle,
        command: &'static str,
        args: Value,
    ) -> Result<T, String> {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Capture>()
                .0
                .run_mobile_plugin(command, args)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn stop(app: &AppHandle, token: &str) {
        let _ = mobile::<Value>(app, "stopCapture", json!({"token": token})).await;
    }

    pub async fn state(app: &AppHandle, token: &str) -> Result<CaptureStatus, String> {
        mobile(app, "captureState", json!({"token": token})).await
    }

    pub async fn capabilities(app: &AppHandle) -> Result<AndroidCapabilities, String> {
        mobile(app, "captureCapabilities", json!({})).await
    }

    pub async fn control(app: &AppHandle, token: &str, action: &str) -> Result<(), String> {
        mobile::<Value>(
            app,
            "controlAction",
            json!({"token": token, "action": action}),
        )
        .await?;
        Ok(())
    }

    pub async fn start(
        app: AppHandle,
        peer_id: String,
        token: String,
        entire_display: bool,
    ) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Offer {
            sdp: String,
        }
        let core = crate::ensure_core(&app).await?;
        if core.assistance_session().await.is_some() {
            return Err("请先结束当前屏幕共享会话".into());
        }
        let allow_control = core
            .screen_share_control_available(&peer_id)
            .await
            .map_err(|error| error.to_string())?;
        let offer: Offer = mobile(
            &app,
            "startCapture",
            json!({"token": token, "entireDisplay": entire_display, "allowControl": allow_control}),
        )
        .await?;
        let id = match core.invite_to_screen_share(&peer_id, offer.sdp).await {
            Ok(id) => id,
            Err(error) => {
                stop(&app, &token).await;
                return Err(error.to_string());
            }
        };
        let session_id = id.clone();
        tauri::async_runtime::spawn(async move {
            monitor(&app, &core, &session_id, &token).await;
            stop(&app, &token).await;
            core.end_screen_share(&session_id).await;
        });
        Ok(id)
    }

    async fn monitor(app: &AppHandle, core: &linchuan_core::App, id: &str, token: &str) {
        let mut answered = false;
        let mut deadline = None;
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            let Some(session) = core
                .assistance_session()
                .await
                .filter(|session| session.id == id)
            else {
                return;
            };
            if let Some(sdp) = session.answer.filter(|_| !answered) {
                if mobile::<Value>(app, "setAnswer", json!({"token": token, "sdp": sdp}))
                    .await
                    .is_err()
                {
                    return;
                }
                answered = true;
            }
            let Ok(status) = state(app, token).await else {
                return;
            };
            match status.state.as_str() {
                "waiting" => {}
                "connected" => deadline = None,
                "connecting" => {
                    if *deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(15))
                        <= Instant::now()
                    {
                        return;
                    }
                }
                _ => return,
            }
        }
    }
}
