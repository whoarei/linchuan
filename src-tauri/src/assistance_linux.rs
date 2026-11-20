use tauri::{ipc::Response, AppHandle};

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("linux-screen-assistance")
        .setup(|app, _| {
            #[cfg(target_os = "linux")]
            {
                use tauri::Manager;
                app.manage(platform::Transport::default());
            }
            #[cfg(not(target_os = "linux"))]
            let _ = app;
            Ok(())
        })
        .build()
}

#[tauri::command]
pub async fn assistance_linux_start(
    app: AppHandle,
    token: String,
    peer_id: String,
) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    return platform::start(app, token, peer_id).await;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, token, peer_id);
        Err("当前平台不使用 Linux 屏幕共享接口".into())
    }
}

#[tauri::command]
pub async fn assistance_linux_join(
    app: AppHandle,
    token: String,
    id: String,
    sdp: String,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return platform::join(app, token, id, sdp).await;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, token, id, sdp);
        Err("当前平台不使用 Linux 屏幕共享接口".into())
    }
}

#[tauri::command]
pub fn assistance_linux_stop(app: AppHandle, token: Option<String>) {
    #[cfg(target_os = "linux")]
    platform::stop(&app, token.as_deref());
    #[cfg(not(target_os = "linux"))]
    let _ = (app, token);
}

#[tauri::command]
pub fn assistance_linux_status(app: AppHandle, token: String) -> String {
    #[cfg(target_os = "linux")]
    return platform::status(&app, &token).into();
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, token);
        "ended".into()
    }
}

#[tauri::command]
pub fn assistance_linux_frame(app: AppHandle, token: String) -> Response {
    #[cfg(target_os = "linux")]
    return Response::new(platform::frame(&app, &token));
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, token);
        Response::new(Vec::<u8>::new())
    }
}

#[tauri::command]
pub fn assistance_linux_message(app: AppHandle, token: String, message: String) {
    #[cfg(target_os = "linux")]
    platform::message(&app, &token, &message);
    #[cfg(not(target_os = "linux"))]
    let _ = (app, token, message);
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use gstreamer::{self as gst, prelude::*};
    use gstreamer_app::{AppSink, AppSinkCallbacks};
    use gstreamer_sdp::SDPMessage;
    use gstreamer_webrtc::{
        WebRTCBundlePolicy, WebRTCDataChannel, WebRTCDataChannelState, WebRTCICEGatheringState,
        WebRTCPeerConnectionState, WebRTCRTPTransceiver, WebRTCRTPTransceiverDirection,
        WebRTCSDPType, WebRTCSessionDescription,
    };
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use tauri::{Emitter, Manager};

    #[derive(Default)]
    pub struct Transport(Mutex<Option<Arc<Connection>>>);

    struct Connection {
        token: String,
        pipeline: gst::Pipeline,
        rtc: gst::Element,
        channel: Mutex<Option<WebRTCDataChannel>>,
        frame: Mutex<Vec<u8>>,
    }

    impl Drop for Connection {
        fn drop(&mut self) {
            let _ = self.pipeline.set_state(gst::State::Null);
        }
    }

    fn current(app: &AppHandle, token: &str) -> Option<Arc<Connection>> {
        let state = app.state::<Transport>();
        let active = state.0.lock().unwrap();
        active
            .as_ref()
            .filter(|connection| connection.token == token)
            .cloned()
    }

    pub fn stop(app: &AppHandle, token: Option<&str>) {
        let state = app.state::<Transport>();
        let stopped = {
            let mut active = state.0.lock().unwrap();
            if token.is_some_and(|token| active.as_ref().is_none_or(|call| call.token != token)) {
                return;
            }
            active.take()
        };
        if let Some(connection) = stopped {
            let _ = connection.pipeline.set_state(gst::State::Null);
        }
    }

    pub fn frame(app: &AppHandle, token: &str) -> Vec<u8> {
        current(app, token)
            .map(|c| std::mem::take(&mut *c.frame.lock().unwrap()))
            .unwrap_or_default()
    }

    pub fn status(app: &AppHandle, token: &str) -> &'static str {
        match current(app, token).map(|c| {
            c.rtc
                .property::<WebRTCPeerConnectionState>("connection-state")
        }) {
            Some(WebRTCPeerConnectionState::Connected) => "connected",
            Some(WebRTCPeerConnectionState::Failed | WebRTCPeerConnectionState::Closed) | None => {
                "ended"
            }
            _ => "connecting",
        }
    }

    pub fn message(app: &AppHandle, token: &str, message: &str) {
        if message.len() > 2200 {
            return;
        }
        if let Some(connection) = current(app, token) {
            let channel = connection.channel.lock().unwrap().clone();
            if let Some(channel) = channel.filter(|channel| {
                channel.ready_state() == WebRTCDataChannelState::Open
                    && channel.buffered_amount() < 8192
            }) {
                channel.send_string(Some(message));
            }
        }
    }

    #[derive(Clone, serde::Serialize)]
    struct Event {
        token: String,
        kind: &'static str,
        message: Option<String>,
    }

    fn bind_channel(app: &AppHandle, connection: &Connection, channel: WebRTCDataChannel) {
        let mut slot = connection.channel.lock().unwrap();
        if slot.is_some() || channel.label().as_deref() != Some("linchuan-control-v1") {
            channel.close();
            return;
        }
        let event = |kind| Event {
            token: connection.token.clone(),
            kind,
            message: None,
        };
        let (open_app, opened) = (app.clone(), event("open"));
        channel.connect_on_open(move |_| {
            let _ = open_app.emit("linchuan-linux-control", opened.clone());
        });
        let (close_app, closed) = (app.clone(), event("close"));
        channel.connect_on_close(move |_| {
            let _ = close_app.emit("linchuan-linux-control", closed.clone());
        });
        let (message_app, received) = (app.clone(), event("message"));
        channel.connect_on_message_string(move |_, message| {
            if let Some(message) = message.filter(|message| message.len() <= 2200) {
                let mut received = received.clone();
                received.message = Some(message.into());
                let _ = message_app.emit("linchuan-linux-control", received);
            }
        });
        if channel.ready_state() == WebRTCDataChannelState::Open {
            let _ = app.emit("linchuan-linux-control", event("open"));
        }
        *slot = Some(channel);
    }

    fn setup(
        app: &AppHandle,
        token: String,
        sharer: bool,
        allow_control: bool,
    ) -> Result<Arc<Connection>, String> {
        if token.len() != 36 {
            return Err("屏幕共享标识无效".into());
        }
        gst::init().map_err(|e| e.to_string())?;
        let pipeline = gst::Pipeline::new();
        let rtc = gst::ElementFactory::make("webrtcbin")
            .name("assistance")
            .property("bundle-policy", WebRTCBundlePolicy::MaxBundle)
            .build()
            .map_err(|e| e.to_string())?;
        pipeline.add(&rtc).map_err(|e| e.to_string())?;
        let connection = Arc::new(Connection {
            token,
            pipeline,
            rtc,
            channel: Mutex::new(None),
            frame: Mutex::new(Vec::new()),
        });
        let state = app.state::<Transport>();
        {
            let mut active = state.0.lock().unwrap();
            if active.is_some() {
                return Err("请先结束当前屏幕共享".into());
            }
            *active = Some(connection.clone());
        }
        if let Err(error) = prepare(app, &connection, sharer, allow_control) {
            stop(app, Some(&connection.token));
            return Err(error);
        }
        Ok(connection)
    }

    fn prepare(
        app: &AppHandle,
        connection: &Arc<Connection>,
        sharer: bool,
        allow_control: bool,
    ) -> Result<(), String> {
        if sharer {
            if !crate::assistance_input::assistance_input_available() {
                return Err("Linux 屏幕共享需要单屏 X11 桌面".into());
            }
            use x11rb::{connection::Connection as _, protocol::xproto::ConnectionExt};
            let (display, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
            let root = display.setup().roots[screen].root;
            let geometry = display
                .get_geometry(root)
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?;
            let width = f64::from(geometry.width.max(2));
            let height = f64::from(geometry.height.max(2));
            let scale = 1.0_f64.min(1280.0 / width).min(800.0 / height);
            let (scaled_width, scaled_height) = (
                (width * scale).round() as u32,
                (height * scale).round() as u32,
            );
            let source = format!(
                "ximagesrc use-damage=false endx={} endy={} ! video/x-raw,framerate=12/1 ! videoconvert ! videoscale ! video/x-raw,width={scaled_width},height={scaled_height} ! vp8enc deadline=1 cpu-used=8 target-bitrate=1800000 keyframe-max-dist=24 ! rtpvp8pay pt=96 ! capsfilter caps=application/x-rtp,media=video,encoding-name=VP8,payload=96",
                geometry.width.saturating_sub(1), geometry.height.saturating_sub(1)
            );
            let capture =
                gst::parse::bin_from_description(&source, true).map_err(|e| e.to_string())?;
            connection
                .pipeline
                .add(&capture)
                .map_err(|e| e.to_string())?;
            capture.link(&connection.rtc).map_err(|e| e.to_string())?;
            connection
                .pipeline
                .set_state(gst::State::Ready)
                .map_err(|e| e.to_string())?;
            if allow_control {
                let channel = connection
                    .rtc
                    .emit_by_name::<Option<WebRTCDataChannel>>(
                        "create-data-channel",
                        &[&"linchuan-control-v1", &None::<gst::Structure>],
                    )
                    .ok_or("无法创建协助控制通道")?;
                bind_channel(app, connection, channel);
            }
        } else {
            let video = gst::Caps::builder("application/x-rtp")
                .field("media", "video")
                .field("encoding-name", "VP8")
                .field("clock-rate", 90_000_i32)
                .build();
            connection.rtc.emit_by_name::<WebRTCRTPTransceiver>(
                "add-transceiver",
                &[&WebRTCRTPTransceiverDirection::Recvonly, &video],
            );
            let output = gst::parse::bin_from_description(
                "queue ! rtpvp8depay ! vp8dec ! videoconvert ! jpegenc quality=75 ! appsink name=frames max-buffers=1 drop=true sync=false", true,
            ).map_err(|e| e.to_string())?;
            let sink = output
                .by_name("frames")
                .and_then(|element| element.downcast::<AppSink>().ok())
                .ok_or("无法创建画面播放器")?;
            let weak = Arc::downgrade(connection);
            sink.set_callbacks(
                AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                        let data = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                        if let Some(connection) = weak.upgrade() {
                            *connection.frame.lock().unwrap() = data.as_slice().to_vec();
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
            connection
                .pipeline
                .add(&output)
                .map_err(|e| e.to_string())?;
            let input = output.static_pad("sink").ok_or("找不到画面播放器入口")?;
            connection.rtc.connect_pad_added(move |_, pad| {
                if pad.direction() == gst::PadDirection::Src {
                    let _ = pad.link(&input);
                }
            });
            let weak = Arc::downgrade(connection);
            let handle = app.clone();
            connection
                .rtc
                .connect("on-data-channel", false, move |values| {
                    if let (Some(connection), Ok(channel)) =
                        (weak.upgrade(), values[1].get::<WebRTCDataChannel>())
                    {
                        bind_channel(&handle, &connection, channel);
                    }
                    None
                });
        }
        connection
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn apply_description(
        connection: &Connection,
        signal: &str,
        description: &WebRTCSessionDescription,
    ) -> Result<(), String> {
        let (promise, done) = gst::Promise::new_future();
        connection
            .rtc
            .emit_by_name::<()>(signal, &[description, &promise]);
        tokio::time::timeout(Duration::from_secs(8), done)
            .await
            .map_err(|_| "连接协商超时")?
            .map_err(|e| format!("连接协商失败：{e:?}"))?;
        Ok(())
    }

    async fn local_sdp(connection: &Connection, offer: bool) -> Result<String, String> {
        let (promise, done) = gst::Promise::new_future();
        let key = if offer { "offer" } else { "answer" };
        let signal = if offer {
            "create-offer"
        } else {
            "create-answer"
        };
        connection
            .rtc
            .emit_by_name::<()>(signal, &[&None::<gst::Structure>, &promise]);
        let reply = tokio::time::timeout(Duration::from_secs(8), done)
            .await
            .map_err(|_| "连接协商超时")?
            .map_err(|e| format!("创建连接失败：{e:?}"))?
            .ok_or("缺少屏幕共享连接信息")?;
        let description = reply
            .get::<WebRTCSessionDescription>(key)
            .map_err(|e| e.to_string())?;
        apply_description(connection, "set-local-description", &description).await?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && connection
                .rtc
                .property::<WebRTCICEGatheringState>("ice-gathering-state")
                != WebRTCICEGatheringState::Complete
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let sdp = connection
            .rtc
            .property::<Option<WebRTCSessionDescription>>("local-description")
            .ok_or("缺少本地连接信息")?
            .sdp()
            .as_text()
            .map_err(|e| e.to_string())?;
        if !sdp.contains("\r\na=candidate:") {
            return Err("找不到可用的局域网连接地址".into());
        }
        Ok(sdp)
    }

    fn parse_description(
        kind: WebRTCSDPType,
        text: &str,
    ) -> Result<WebRTCSessionDescription, String> {
        SDPMessage::parse_buffer(text.as_bytes())
            .map(|sdp| WebRTCSessionDescription::new(kind, sdp))
            .map_err(|e| e.to_string())
    }

    pub async fn start(app: AppHandle, token: String, peer_id: String) -> Result<String, String> {
        let core = crate::ensure_core(&app).await?;
        let allow_control = core
            .screen_share_control_available(&peer_id)
            .await
            .map_err(|error| error.to_string())?;
        let connection = setup(&app, token.clone(), true, allow_control)?;
        let result = async {
            let sdp = local_sdp(&connection, true).await?;
            if current(&app, &token).is_none() {
                return Err("屏幕共享已取消".into());
            }
            core.invite_to_screen_share(&peer_id, sdp)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        match result {
            Ok(id) => {
                monitor(app, core, token, id.clone(), true);
                Ok(id)
            }
            Err(error) => {
                stop(&app, Some(&token));
                Err(error)
            }
        }
    }

    pub async fn join(
        app: AppHandle,
        token: String,
        id: String,
        sdp: String,
    ) -> Result<(), String> {
        let core = crate::ensure_core(&app).await?;
        let connection = setup(&app, token.clone(), false, false)?;
        let result = async {
            apply_description(
                &connection,
                "set-remote-description",
                &parse_description(WebRTCSDPType::Offer, &sdp)?,
            )
            .await?;
            let response = local_sdp(&connection, false).await?;
            if current(&app, &token).is_none() {
                return Err("屏幕共享已取消".into());
            }
            core.join_screen_share(&id, response)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        match result {
            Ok(()) => {
                monitor(app, core, token, id, false);
                Ok(())
            }
            Err(error) => {
                stop(&app, Some(&token));
                Err(error)
            }
        }
    }

    fn monitor(app: AppHandle, core: linchuan_core::App, token: String, id: String, sharer: bool) {
        tauri::async_runtime::spawn(async move {
            let mut answered = !sharer;
            let mut deadline = (!sharer).then(|| Instant::now() + Duration::from_secs(15));
            let mut ticker = tokio::time::interval(Duration::from_millis(400));
            loop {
                ticker.tick().await;
                let Some(connection) = current(&app, &token) else {
                    break;
                };
                if let Some(error) = connection
                    .pipeline
                    .bus()
                    .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]))
                {
                    if let gst::MessageView::Error(error) = error.view() {
                        eprintln!("屏幕共享媒体管线失败：{}", error.error());
                    }
                    break;
                }
                if sharer && !crate::assistance_input::assistance_input_available() {
                    break;
                }
                let Some(session) = core
                    .assistance_session()
                    .await
                    .filter(|session| session.id == id)
                else {
                    break;
                };
                if let Some(answer) = session.answer.filter(|_| !answered) {
                    let Ok(description) = parse_description(WebRTCSDPType::Answer, &answer) else {
                        break;
                    };
                    if apply_description(&connection, "set-remote-description", &description)
                        .await
                        .is_err()
                    {
                        break;
                    }
                    deadline = Some(Instant::now() + Duration::from_secs(15));
                    answered = true;
                }
                match status(&app, &token) {
                    "connected" => deadline = None,
                    "connecting" if answered => {
                        if *deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(15))
                            <= Instant::now()
                        {
                            break;
                        }
                    }
                    "ended" => break,
                    _ => (),
                }
            }
            crate::assistance_input::revoke(&app, Some(&id));
            stop(&app, Some(&token));
            core.end_screen_share(&id).await;
        });
    }
}
