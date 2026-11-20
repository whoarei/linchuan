use serde::Deserialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct Control(Mutex<Option<Grant>>);

struct Grant {
    id: String,
    lease: String,
}

pub fn revoke(app: &AppHandle, id: Option<&str>) {
    let state = app.state::<Control>();
    let mut grant = state.0.lock().unwrap();
    if id.is_none()
        || grant
            .as_ref()
            .is_some_and(|grant| Some(grant.id.as_str()) == id)
    {
        grant.take();
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(target_os = "android", allow(dead_code))]
pub enum Input {
    Click {
        x: f64,
        y: f64,
        button: u8,
    },
    Drag {
        x: f64,
        y: f64,
        to_x: f64,
        to_y: f64,
    },
    Scroll {
        x: f64,
        y: f64,
        lines: i32,
    },
    Key {
        key: String,
    },
    Text {
        text: String,
    },
}

async fn sharer_is_active(app: &AppHandle, id: &str) -> Result<bool, String> {
    Ok(crate::ensure_core(app)
        .await?
        .assistance_session()
        .await
        .is_some_and(|session| session.id == id && session.accepted && session.is_sharer()))
}

#[tauri::command]
pub fn assistance_input_available() -> bool {
    platform::available()
}

#[tauri::command]
pub async fn assistance_input_grant(
    app: AppHandle,
    id: String,
    lease: Option<String>,
) -> Result<(), String> {
    let Some(lease) = lease else {
        revoke(&app, Some(&id));
        return Ok(());
    };
    if lease.len() != 36 || !platform::available() || !sharer_is_active(&app, &id).await? {
        return Err("当前共享画面无法接管".into());
    }
    *app.state::<Control>().0.lock().unwrap() = Some(Grant { id, lease });
    Ok(())
}

#[tauri::command]
pub async fn assistance_input_apply(
    app: AppHandle,
    id: String,
    lease: String,
    input: Input,
) -> Result<(), String> {
    if !sharer_is_active(&app, &id).await? {
        return Err("屏幕共享已结束".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Control>();
        let grant = state.0.lock().unwrap();
        if !grant
            .as_ref()
            .is_some_and(|g| g.id == id && g.lease == lease)
        {
            return Err("控制权限已收回".into());
        }
        if !platform::available() {
            return Err("显示环境已改变，控制已停止".into());
        }
        platform::apply(input)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod platform {
    use super::Input;
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

    #[cfg(target_os = "windows")]
    pub fn available() -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};
        unsafe { GetSystemMetrics(SM_CMONITORS) == 1 }
    }

    #[cfg(target_os = "linux")]
    pub fn available() -> bool {
        use x11rb::{connection::Connection, protocol::randr::ConnectionExt};
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            && std::env::var("GDK_BACKEND").as_deref() != Ok("x11")
        {
            return false;
        }
        let Ok((connection, screen)) = x11rb::connect(None) else {
            return false;
        };
        let root = connection.setup().roots[screen].root;
        connection
            .randr_get_monitors(root, true)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.monitors.len() == 1)
    }

    pub fn apply(input: Input) -> Result<(), String> {
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        let (width, height) = enigo.main_display().map_err(|e| e.to_string())?;
        if width <= 0 || height <= 0 {
            return Err("无法读取共享画面尺寸".into());
        }
        let coords = |x: f64, y: f64| {
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return Err("坐标超出画面".to_owned());
            }
            Ok((
                (x * f64::from(width - 1)).round() as i32,
                (y * f64::from(height - 1)).round() as i32,
            ))
        };
        let move_to = |enigo: &mut Enigo, x: i32, y: i32| enigo.move_mouse(x, y, Coordinate::Abs);
        match input {
            Input::Click { x, y, button } => {
                let button = match button {
                    0 => Button::Left,
                    2 => Button::Right,
                    _ => return Ok(()),
                };
                let (x, y) = coords(x, y)?;
                move_to(&mut enigo, x, y).and_then(|_| enigo.button(button, Direction::Click))
            }
            Input::Drag { x, y, to_x, to_y } => {
                let (x, y) = coords(x, y)?;
                let (end_x, end_y) = coords(to_x, to_y)?;
                move_to(&mut enigo, x, y).map_err(|e| e.to_string())?;
                enigo
                    .button(Button::Left, Direction::Press)
                    .map_err(|e| e.to_string())?;
                let moved = (1..=8).try_for_each(|step| {
                    std::thread::sleep(std::time::Duration::from_millis(12));
                    move_to(
                        &mut enigo,
                        x + (end_x - x) * step / 8,
                        y + (end_y - y) * step / 8,
                    )
                });
                let release = enigo.button(Button::Left, Direction::Release);
                moved.and(release)
            }
            Input::Scroll { x, y, lines } => {
                let (x, y) = coords(x, y)?;
                move_to(&mut enigo, x, y)
                    .and_then(|_| enigo.scroll(lines.clamp(-6, 6), Axis::Vertical))
            }
            Input::Key { key } => {
                let key = match key.as_str() {
                    "Enter" => Key::Return,
                    "Escape" => Key::Escape,
                    "Tab" => Key::Tab,
                    "Backspace" => Key::Backspace,
                    "Delete" => Key::Delete,
                    "ArrowUp" => Key::UpArrow,
                    "ArrowDown" => Key::DownArrow,
                    "ArrowLeft" => Key::LeftArrow,
                    "ArrowRight" => Key::RightArrow,
                    _ => return Ok(()),
                };
                enigo.key(key, Direction::Click)
            }
            Input::Text { text } if text.len() <= 1600 && !text.chars().any(char::is_control) => {
                enigo.text(&text)
            }
            Input::Text { .. } => return Ok(()),
        }
        .map_err(|e| e.to_string())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod platform {
    use super::Input;
    pub fn available() -> bool {
        false
    }
    pub fn apply(_: Input) -> Result<(), String> {
        Err("当前系统不使用桌面控制接口".into())
    }
}
