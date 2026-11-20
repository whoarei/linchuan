use crate::{api, identity, model::*, App};
use axum::{
    extract::{ConnectInfo, Path, State as AxState},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

pub const CAPABILITY: &str = "screen-share-v1";
const WEB_VIEW: &str = "screen-view-web-v1";
const WEB_CONTROL: &str = "screen-control-web-v1";
const INVITE_LIFETIME: Duration = Duration::from_secs(75);
const BROWSER_LEASE: Duration = Duration::from_secs(90);
const SIGNAL_LIFETIME: u64 = 90;

pub(crate) fn browser_capabilities(enabled: bool, secure: bool) -> Vec<String> {
    let mut capabilities = crate::capabilities();
    if enabled {
        capabilities.push(WEB_VIEW.into());
        if secure {
            capabilities.push(WEB_CONTROL.into());
        }
    }
    capabilities
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Sharer,
    Viewer,
}

#[derive(Clone, Serialize)]
pub struct Session {
    pub id: String,
    peer_id: String,
    peer_name: String,
    role: Role,
    pub accepted: bool,
    offer: Option<String>,
    pub answer: Option<String>,
}

impl Session {
    pub fn is_sharer(&self) -> bool {
        self.role == Role::Sharer
    }
}

struct Active {
    session: Session,
    peer: Device,
    deadline: Instant,
    browser: Option<BrowserViewer>,
}

struct BrowserViewer {
    offer: Option<String>,
    tab: Option<uuid::Uuid>,
    control: bool,
    expires: u64,
}

impl Active {
    fn new(id: String, peer: Device, role: Role, offer: Option<String>) -> Self {
        let session = Session {
            id,
            peer_id: peer.id.clone(),
            peer_name: peer.name.clone(),
            role,
            accepted: false,
            offer,
            answer: None,
        };
        Self {
            session,
            peer,
            deadline: Instant::now() + INVITE_LIFETIME,
            browser: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct Sessions {
    active: Option<Active>,
    seen_invites: HashMap<String, u64>,
}

impl Sessions {
    fn expire(&mut self) {
        if self.active.as_ref().is_some_and(|active| {
            ((!active.session.accepted || active.browser.is_some())
                && active.deadline <= Instant::now())
                || active
                    .browser
                    .as_ref()
                    .is_some_and(|browser| browser.expires <= now())
        }) {
            self.active = None;
        }
    }

    fn remove(&mut self, id: &str) -> Option<Active> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.session.id == id)
        {
            return self.active.take();
        }
        None
    }
}

#[derive(Serialize, Deserialize)]
struct Signal {
    protocol: String,
    id: String,
    recipient: String,
    sent_at: u64,
    #[serde(flatten)]
    action: SignalAction,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SignalAction {
    Invite { sdp: String },
    Accept { sdp: String },
    End,
}

fn validate_sdp(sdp: &str) -> api::Result<()> {
    if sdp.len() > 32 * 1024 || !sdp.starts_with("v=0\r\n") {
        return Err(api::Error::bad("屏幕共享连接信息无效"));
    }
    Ok(())
}

fn has_data_channel(sdp: &str) -> bool {
    sdp.lines().any(|line| line.starts_with("m=application "))
}

impl App {
    pub async fn screen_share_control_available(&self, peer_id: &str) -> api::Result<bool> {
        if peer_id.starts_with("browser-") {
            return self
                .0
                .state
                .lock()
                .await
                .sessions
                .values()
                .find(|session| session.id == peer_id && session.online())
                .map(|session| session.secure)
                .ok_or_else(|| api::Error::bad("浏览器已断开连接，请重新配对"));
        }
        self.assistance_peer(peer_id).await?;
        Ok(true)
    }

    async fn assistance_peer(&self, id: &str) -> api::Result<Device> {
        self.0
            .state
            .lock()
            .await
            .profiles
            .get(id)
            .filter(|peer| {
                peer.kind == "app"
                    && peer.host_device_id == id
                    && !peer.public_key.is_empty()
                    && !peer.address.is_empty()
            })
            .cloned()
            .ok_or_else(|| api::Error::bad("请先在双方的邻传客户端中连接对方设备"))
    }

    async fn send_assistance(
        &self,
        peer: &Device,
        id: &str,
        action: SignalAction,
    ) -> api::Result<()> {
        let address = api::local_address(&peer.address)?;
        let payload = serde_json::to_value(Signal {
            protocol: CAPABILITY.into(),
            id: id.into(),
            recipient: peer.id.clone(),
            sent_at: now(),
            action,
        })
        .map_err(|_| api::Error::bad("无法创建屏幕共享请求"))?;
        let signed = self.signed(payload).await;
        let response = self
            .0
            .client
            .post(format!("{address}/api/peer/assistance"))
            .timeout(Duration::from_secs(5))
            .json(&signed)
            .send()
            .await
            .map_err(|_| api::Error::bad("无法连接对方设备"))?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::CONFLICT => Err(api::Error::conflict("对方正在共享，或邀请已失效")),
            StatusCode::FORBIDDEN => Err(api::Error::bad(
                "对方尚未识别此设备，请先在双方客户端中连接",
            )),
            _ => Err(api::Error::bad("对方未能处理屏幕共享请求")),
        }
    }

    pub async fn assistance_session(&self) -> Option<Session> {
        let mut sessions = self.0.assistance.as_ref()?.lock().await;
        sessions.expire();
        sessions
            .active
            .as_ref()
            .map(|active| active.session.clone())
    }

    pub async fn invite_to_screen_share(&self, peer_id: &str, sdp: String) -> api::Result<String> {
        let sessions = self.0.assistance.as_ref().ok_or_else(api::Error::missing)?;
        validate_sdp(&sdp)?;
        if peer_id.starts_with("browser-") {
            let state = self.0.state.lock().await;
            let browser = state
                .sessions
                .values()
                .find(|session| session.id == peer_id && session.online())
                .ok_or_else(|| api::Error::bad("浏览器已断开连接，请重新配对"))?;
            let peer = state
                .profiles
                .get(peer_id)
                .ok_or_else(api::Error::missing)?;
            let control = has_data_channel(&sdp);
            if control && !browser.secure {
                return Err(api::Error::bad("浏览器控制需要通过 HTTPS 配对"));
            }
            let mut sessions = sessions.lock().await;
            sessions.expire();
            if sessions.active.is_some() {
                return Err(api::Error::conflict("请先结束当前屏幕共享会话"));
            }
            let id = uid();
            let mut active = Active::new(id.clone(), peer.clone(), Role::Sharer, None);
            active.browser = Some(BrowserViewer {
                offer: Some(sdp),
                tab: None,
                control,
                expires: browser.expires,
            });
            sessions.active = Some(active);
            self.notify();
            return Ok(id);
        }
        let peer = self.assistance_peer(peer_id).await?;
        if !peer
            .capabilities
            .iter()
            .any(|capability| capability == CAPABILITY)
        {
            return Err(api::Error::bad("对方客户端暂不支持屏幕共享"));
        }
        let id = uid();
        {
            let mut sessions = sessions.lock().await;
            sessions.expire();
            if sessions.active.is_some() {
                return Err(api::Error::conflict("请先结束当前屏幕共享会话"));
            }
            sessions.active = Some(Active::new(id.clone(), peer.clone(), Role::Sharer, None));
        }
        if let Err(error) = self
            .send_assistance(&peer, &id, SignalAction::Invite { sdp })
            .await
        {
            sessions.lock().await.remove(&id);
            return Err(error);
        }
        Ok(id)
    }

    pub async fn join_screen_share(&self, id: &str, sdp: String) -> api::Result<()> {
        let sessions = self.0.assistance.as_ref().ok_or_else(api::Error::missing)?;
        validate_sdp(&sdp)?;
        let peer = {
            let mut sessions = sessions.lock().await;
            sessions.expire();
            let active = sessions
                .active
                .as_mut()
                .filter(|a| {
                    a.session.id == id && a.session.role == Role::Viewer && !a.session.accepted
                })
                .ok_or_else(|| api::Error::conflict("邀请已失效"))?;
            active.session.accepted = true;
            active.session.offer = None;
            active.peer.clone()
        };
        if let Err(error) = self
            .send_assistance(&peer, id, SignalAction::Accept { sdp })
            .await
        {
            sessions.lock().await.remove(id);
            return Err(error);
        }
        Ok(())
    }

    pub async fn end_screen_share(&self, id: &str) {
        let Some(sessions) = &self.0.assistance else {
            return;
        };
        let ended = sessions.lock().await.remove(id);
        if let Some(active) = ended {
            if active.browser.is_some() {
                self.notify();
            } else {
                let _ = self
                    .send_assistance(&active.peer, id, SignalAction::End)
                    .await;
            }
        }
    }

    pub(crate) async fn disconnect_browser_viewer(&self, peer_id: &str) {
        if let Some(sessions) = &self.0.assistance {
            let mut sessions = sessions.lock().await;
            if sessions
                .active
                .as_ref()
                .is_some_and(|active| active.peer.id == peer_id && active.browser.is_some())
            {
                sessions.active = None;
                self.notify();
            }
        }
    }

    pub async fn end_active_screen_share(&self) {
        if let Some(session) = self.assistance_session().await {
            self.end_screen_share(&session.id).await;
        }
    }
}

pub(crate) async fn browser_session(
    AxState(app): AxState<App>,
    headers: HeaderMap,
) -> api::Result<Json<Option<Session>>> {
    let browser = app.principal(api::token(&headers)?).await?;
    if browser.owner {
        return Err(api::Error::forbidden());
    }
    let Some(sessions) = &app.0.assistance else {
        return Ok(Json(None));
    };
    let name = app.0.config.lock().await.name.clone();
    let mut sessions = sessions.lock().await;
    sessions.expire();
    Ok(Json(sessions.active.as_ref().and_then(|active| {
        let target = active
            .browser
            .as_ref()
            .filter(|_| active.peer.id == browser.id)?;
        Some(Session {
            id: active.session.id.clone(),
            peer_id: app.0.device_id.clone(),
            peer_name: name,
            role: Role::Viewer,
            accepted: active.session.accepted,
            offer: target.offer.clone(),
            answer: None,
        })
    })))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum BrowserAction {
    Join { sdp: String, tab: uuid::Uuid },
    Heartbeat { tab: uuid::Uuid },
    End { tab: Option<uuid::Uuid> },
}

pub(crate) async fn browser_action(
    AxState(app): AxState<App>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(action): Json<BrowserAction>,
) -> api::Result<Json<serde_json::Value>> {
    let browser = app.principal(api::token(&headers)?).await?;
    if browser.owner {
        return Err(api::Error::forbidden());
    }
    let sessions = app.0.assistance.as_ref().ok_or_else(api::Error::missing)?;
    let mut sessions = sessions.lock().await;
    sessions.expire();
    let active = sessions
        .active
        .as_mut()
        .filter(|active| active.session.id == id && active.peer.id == browser.id)
        .ok_or_else(api::Error::missing)?;
    let target = active.browser.as_mut().ok_or_else(api::Error::missing)?;
    match action {
        BrowserAction::Join { sdp, tab } if target.tab.is_none() => {
            validate_sdp(&sdp)?;
            if !target.control && has_data_channel(&sdp) {
                return Err(api::Error::bad("当前共享未启用控制"));
            }
            target.tab = Some(tab);
            target.offer = None;
            active.session.accepted = true;
            active.session.answer = Some(sdp);
            active.deadline = Instant::now() + BROWSER_LEASE;
            app.notify();
        }
        BrowserAction::Heartbeat { tab } if target.tab == Some(tab) => {
            active.deadline = Instant::now() + BROWSER_LEASE;
        }
        BrowserAction::End { tab } if target.tab.is_none() || target.tab == tab => {
            sessions.active = None;
            app.notify();
        }
        _ => return Err(api::Error::conflict("共享已在其他页面打开，或邀请已失效")),
    }
    Ok(Json(serde_json::json!({"ok": true})))
}

pub(crate) async fn receive(
    AxState(app): AxState<App>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(envelope): Json<SignedEnvelope>,
) -> api::Result<StatusCode> {
    let sessions = app.0.assistance.as_ref().ok_or_else(api::Error::missing)?;
    api::rate_limit(
        &mut *app.0.state.lock().await,
        format!("assistance:{}", address.ip()),
        40,
    )?;
    let peer = app
        .assistance_peer(&envelope.host_id)
        .await
        .map_err(|_| api::Error::forbidden())?;
    if peer.public_key != envelope.public_key || identity::verify(&envelope).is_err() {
        return Err(api::Error::forbidden());
    }
    let signal: Signal = serde_json::from_value(envelope.payload)
        .map_err(|_| api::Error::bad("屏幕共享请求无效"))?;
    let now = now();
    if signal.protocol != CAPABILITY
        || signal.recipient != app.0.device_id
        || uuid::Uuid::parse_str(&signal.id)
            .ok()
            .is_none_or(|id| id.to_string() != signal.id)
        || now.abs_diff(signal.sent_at) > SIGNAL_LIFETIME
    {
        return Err(api::Error::bad("屏幕共享请求无效或已过期"));
    }
    let mut sessions = sessions.lock().await;
    sessions.expire();
    match signal.action {
        SignalAction::Invite { sdp } => {
            validate_sdp(&sdp)?;
            sessions.seen_invites.retain(|_, expires| *expires >= now);
            if sessions.active.is_some()
                || sessions.seen_invites.len() >= 256
                || sessions.seen_invites.contains_key(&signal.id)
            {
                return Err(api::Error::conflict("已有会话或邀请已失效"));
            }
            sessions
                .seen_invites
                .insert(signal.id.clone(), signal.sent_at + SIGNAL_LIFETIME);
            sessions.active = Some(Active::new(signal.id, peer, Role::Viewer, Some(sdp)));
        }
        SignalAction::Accept { sdp } => {
            validate_sdp(&sdp)?;
            let active = sessions
                .active
                .as_mut()
                .filter(|active| {
                    active.session.id == signal.id
                        && active.peer.id == peer.id
                        && active.session.role == Role::Sharer
                        && !active.session.accepted
                })
                .ok_or_else(|| api::Error::conflict("邀请已失效"))?;
            active.session.accepted = true;
            active.session.answer = Some(sdp);
        }
        SignalAction::End => {
            if sessions
                .active
                .as_ref()
                .is_some_and(|a| a.peer.id == peer.id)
            {
                sessions.remove(&signal.id);
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDP: &str = "v=0\r\ns=fixture\r\n";

    struct Host {
        app: App,
        _directory: tempfile::TempDir,
    }

    impl Host {
        async fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            Self::start(directory).await
        }

        async fn start(directory: tempfile::TempDir) -> Self {
            let mut options = crate::Options::new(directory.path().into());
            options.discovery = false;
            options.screen_sharing = true;
            Self {
                app: App::start(options).await.unwrap(),
                _directory: directory,
            }
        }

        async fn remember(&self, peer: &Self) {
            let device = self
                .app
                .inspect_peer(&peer.app.bootstrap().address, None)
                .await
                .unwrap();
            self.app.remember_peer(device).await.unwrap();
        }

        async fn signed_invite(&self, id: &str, sent_at: u64, peer: &Self) -> SignedEnvelope {
            self.app
                .signed(
                    serde_json::to_value(Signal {
                        protocol: CAPABILITY.into(),
                        id: id.into(),
                        recipient: peer.app.0.device_id.clone(),
                        sent_at,
                        action: SignalAction::Invite { sdp: SDP.into() },
                    })
                    .unwrap(),
                )
                .await
        }

        async fn post(&self, envelope: &SignedEnvelope) -> StatusCode {
            self.app
                .0
                .client
                .post(format!(
                    "{}/api/peer/assistance",
                    self.app.bootstrap().address
                ))
                .json(envelope)
                .send()
                .await
                .unwrap()
                .status()
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.app.stop();
        }
    }

    #[tokio::test]
    async fn paired_browser_can_only_join_its_own_invite_and_save_one_tab_lease() {
        use serde_json::{json, Value};

        let directory = tempfile::tempdir().unwrap();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        std::fs::write(directory.path().join("certificate.pem"), cert.pem()).unwrap();
        std::fs::write(
            directory.path().join("key.pem"),
            signing_key.serialize_pem(),
        )
        .unwrap();
        std::fs::write(
            directory.path().join("browser-https.json"),
            r#"{
            "host": "localhost", "certificate": "certificate.pem", "key": "key.pem"
        }"#,
        )
        .unwrap();
        let host = Host::start(directory).await;
        let bootstrap = host.app.bootstrap();
        let https = bootstrap.browser_https.unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(reqwest::Certificate::from_pem(cert.pem().as_bytes()).unwrap())
            .build()
            .unwrap();
        let pair = |address: &str, code: &str| {
            client
                .post(format!("{address}/api/pair"))
                .json(&json!({"name": "网页测试", "code": code}))
                .send()
        };
        assert_eq!(
            pair(&https.address, &bootstrap.pairing_code)
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            pair(&bootstrap.address, &https.pairing_code)
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let secure: Value = pair(&https.address, &https.pairing_code)
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plain: Value = pair(&bootstrap.address, &bootstrap.pairing_code)
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let other: Value = pair(&https.address, &https.pairing_code)
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let plain_id = plain["id"].as_str().unwrap();
        let secure_id = secure["id"].as_str().unwrap();
        let secure_token = secure["token"].as_str().unwrap();
        let other_token = other["token"].as_str().unwrap();
        let channel_sdp = format!("{SDP}m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n");

        assert!(!host
            .app
            .screen_share_control_available(plain_id)
            .await
            .unwrap());
        assert!(host
            .app
            .screen_share_control_available(secure_id)
            .await
            .unwrap());
        assert!(host
            .app
            .invite_to_screen_share(plain_id, channel_sdp.clone())
            .await
            .is_err());
        let http_id = host
            .app
            .invite_to_screen_share(plain_id, SDP.into())
            .await
            .unwrap();
        let plain_join = client
            .post(format!(
                "{}/api/browser/assistance/{http_id}",
                bootstrap.address
            ))
            .bearer_auth(plain["token"].as_str().unwrap());
        assert_eq!(
            plain_join
                .try_clone()
                .unwrap()
                .json(&json!({"action": "join", "sdp": channel_sdp, "tab": uid()}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(plain_join
            .json(&json!({"action": "join", "sdp": SDP, "tab": uid()}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        host.app.end_screen_share(&http_id).await;

        let id = host
            .app
            .invite_to_screen_share(secure_id, channel_sdp.clone())
            .await
            .unwrap();
        let viewer = format!("{}/api/browser/assistance", https.address);
        assert_eq!(
            client
                .get(format!("{}/api/browser/assistance", bootstrap.address))
                .header("X-Forwarded-Proto", "https")
                .bearer_auth(secure_token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert!(client
            .get(&viewer)
            .bearer_auth(other_token)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
            .is_null());
        let invite: Value = client
            .get(&viewer)
            .bearer_auth(secure_token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(invite["id"], id);
        assert_eq!(invite["offer"], channel_sdp);
        assert!(host.app.assistance_session().await.unwrap().offer.is_none());

        let tab = uid();
        let action = |credential: &str, payload: Value| {
            client
                .post(format!("{viewer}/{id}"))
                .bearer_auth(credential)
                .json(&payload)
                .send()
        };
        assert_eq!(
            action(other_token, json!({"action": "end"}))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert!(action(
            secure_token,
            json!({"action": "join", "sdp": channel_sdp, "tab": tab})
        )
        .await
        .unwrap()
        .status()
        .is_success());
        assert!(host.app.assistance_session().await.unwrap().accepted);
        assert_eq!(
            action(
                secure_token,
                json!({"action": "join", "sdp": channel_sdp, "tab": uid()})
            )
            .await
            .unwrap()
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            action(secure_token, json!({"action": "end"}))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        assert!(
            action(secure_token, json!({"action": "heartbeat", "tab": tab}))
                .await
                .unwrap()
                .status()
                .is_success()
        );
        assert!(client
            .post(format!("{}/api/session/logout", https.address))
            .bearer_auth(secure_token)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        assert!(host.app.assistance_session().await.is_none());
    }

    #[tokio::test]
    async fn known_peer_can_join_and_end_a_single_session() {
        let sharer = Host::new().await;
        let viewer = Host::new().await;
        sharer.remember(&viewer).await;
        viewer.remember(&sharer).await;
        let id = sharer
            .app
            .invite_to_screen_share(&viewer.app.0.device_id, SDP.into())
            .await
            .unwrap();
        let invitation = viewer.app.assistance_session().await.unwrap();
        assert_eq!(invitation.id, id);
        assert_eq!(invitation.offer.as_deref(), Some(SDP));
        assert!(!invitation.accepted);
        assert!(sharer
            .app
            .invite_to_screen_share(&viewer.app.0.device_id, SDP.into())
            .await
            .is_err());
        assert!(sharer.app.join_screen_share(&id, SDP.into()).await.is_err());

        viewer.app.join_screen_share(&id, SDP.into()).await.unwrap();
        let shared = sharer.app.assistance_session().await.unwrap();
        assert!(shared.accepted);
        assert_eq!(shared.answer.as_deref(), Some(SDP));
        assert!(viewer.app.assistance_session().await.unwrap().accepted);
        sharer.app.end_screen_share(&id).await;
        assert!(sharer.app.assistance_session().await.is_none());
        assert!(viewer.app.assistance_session().await.is_none());
    }

    #[tokio::test]
    async fn requires_a_known_signer_and_rejects_wrong_targets_and_replays() {
        let sharer = Host::new().await;
        let viewer = Host::new().await;
        let stranger = Host::new().await;
        let id = uid();
        let invite = sharer.signed_invite(&id, now(), &viewer).await;
        assert_eq!(viewer.post(&invite).await, StatusCode::FORBIDDEN);
        viewer.remember(&sharer).await;
        viewer.remember(&stranger).await;

        let mut changed = sharer.signed_invite(&id, now(), &viewer).await;
        changed.payload["sdp"] = "v=0\r\ns=changed\r\n".into();
        assert_eq!(viewer.post(&changed).await, StatusCode::FORBIDDEN);
        let wrong_target = sharer.signed_invite(&id, now(), &stranger).await;
        assert_eq!(viewer.post(&wrong_target).await, StatusCode::BAD_REQUEST);
        let stale = sharer
            .signed_invite(&id, now() - SIGNAL_LIFETIME - 1, &viewer)
            .await;
        assert_eq!(viewer.post(&stale).await, StatusCode::BAD_REQUEST);
        assert_eq!(viewer.post(&invite).await, StatusCode::NO_CONTENT);

        let viewer_device = stranger
            .app
            .inspect_peer(&viewer.app.bootstrap().address, None)
            .await
            .unwrap();
        stranger
            .app
            .send_assistance(&viewer_device, &id, SignalAction::End)
            .await
            .unwrap();
        assert_eq!(viewer.app.assistance_session().await.unwrap().id, id);
        viewer.app.end_screen_share(&id).await;
        assert_eq!(viewer.post(&invite).await, StatusCode::CONFLICT);
        assert!(viewer.app.assistance_session().await.is_none());
    }
}
