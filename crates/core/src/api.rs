use crate::{model::*, App};
use axum::{
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Path, State as AxState, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use tower_http::cors::CorsLayer;

#[derive(Debug)]
pub struct Error(StatusCode, String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}
impl std::error::Error for Error {}
impl Error {
    pub fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    pub fn storage(message: impl Into<String>) -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("数据库操作失败：{}", message.into()),
        )
    }
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "连接已失效，请重新配对".into())
    }
    pub(crate) fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "内容不存在或已过期".into())
    }
    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self(StatusCode::CONFLICT, message.into())
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("文件操作失败：{e}"),
        )
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn token(headers: &HeaderMap) -> Result<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(Error::unauthorized)
}

pub fn router(app: App) -> Router {
    let cors = CorsLayer::new()
        .allow_origin([
            "tauri://localhost".parse::<HeaderValue>().unwrap(),
            "http://tauri.localhost".parse().unwrap(),
            "https://tauri.localhost".parse().unwrap(),
            "http://localhost:1420".parse().unwrap(),
            "http://127.0.0.1:1420".parse().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);
    Router::new()
        .route("/api/info", get(info))
        .route("/api/pair", post(pair))
        .route("/api/session/logout", post(logout))
        .route("/api/state", get(state))
        .route(
            "/api/conversations/direct",
            post(crate::conversations::direct),
        )
        .route(
            "/api/conversations/{id}/messages",
            get(crate::conversations::timeline),
        )
        .route(
            "/api/conversations/{id}/read",
            post(crate::conversations::read),
        )
        .route(
            "/api/conversations/{id}/draft",
            post(crate::conversations::draft),
        )
        .route(
            "/api/storage/clear",
            post(crate::conversations::clear_files),
        )
        .route("/api/groups", post(crate::groups::create))
        .route("/api/groups/{id}/decision", post(crate::groups::decide))
        .route("/api/groups/{id}/leave", post(crate::groups::leave))
        .route("/api/groups/{id}/invite", post(crate::groups::invite))
        .route("/api/peer/group", post(crate::groups::receive))
        .route("/api/settings", post(settings))
        .route("/api/peers", post(add_peer))
        .route("/api/drafts", post(create_draft))
        .route(
            "/api/drafts/{id}/files/{file}",
            put(upload_draft).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/drafts/{id}/send", post(send))
        .route("/api/transfers/{id}/decision", post(decision))
        .route("/api/transfers/{id}/cancel", post(cancel))
        .route("/api/transfers/{id}/files/{file}/ticket", post(ticket))
        .route("/download/{ticket}", get(download))
        .route("/api/events", get(events))
        .route("/api/peer/offer", post(peer_offer))
        .route("/api/peer/transfers/{id}", get(peer_status))
        .route(
            "/api/peer/transfers/{id}/files/{file}",
            put(peer_upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/peer/transfers/{id}/complete", post(peer_complete))
        .route("/api/peer/transfers/{id}/cancel", post(peer_cancel))
        .fallback(get(asset))
        .layer(cors)
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(app)
}
async fn info(AxState(app): AxState<App>) -> Json<Info> {
    Json(app.info().await)
}
async fn pair(
    AxState(app): AxState<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<PairRequest>,
) -> Result<Json<Value>> {
    let name = crate::clean_name(&request.name).map_err(|e| Error::bad(e.to_string()))?;
    let mut state = app.0.state.lock().await;
    rate_limit(&mut state, format!("pair:{}", addr.ip()), 12)?;
    if request.code != app.0.code {
        return Err(Error::unauthorized());
    }
    if state.sessions.len() >= 64 {
        return Err(Error::conflict("连接数量已达上限"));
    }
    let token = secret();
    let id = format!("browser-{}", uid());
    state.sessions.insert(
        crate::identity::token_hash(&token),
        Session {
            id: id.clone(),
            name: name.clone(),
            last_seen: now(),
            expires: now() + RETENTION,
        },
    );
    state.profiles.insert(
        id.clone(),
        Device {
            id: id.clone(),
            name,
            platform: "浏览器".into(),
            kind: "browser".into(),
            host_device_id: app.0.device_id.clone(),
            public_key: hex::encode(app.0.identity.verifying_key().to_bytes()),
            capabilities: crate::capabilities(),
            address: crate::addresses(app.0.port)[0].clone(),
            last_seen: now(),
        },
    );
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(json!({"token": token, "id": id})))
}
pub(crate) fn rate_limit(state: &mut crate::model::State, key: String, limit: u32) -> Result<()> {
    if state.attempts.len() > 1024 {
        state
            .attempts
            .retain(|_, (t, _)| now().saturating_sub(*t) < 60);
    }
    let entry = state.attempts.entry(key).or_insert((now(), 0));
    if now().saturating_sub(entry.0) >= 60 {
        *entry = (now(), 0);
    }
    entry.1 += 1;
    if entry.1 > limit {
        return Err(Error(
            StatusCode::TOO_MANY_REQUESTS,
            "请求过于频繁，请稍后重试".into(),
        ));
    }
    Ok(())
}
async fn state(AxState(app): AxState<App>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    let info = app.info().await;
    let conversations = app
        .0
        .store
        .conversations(p.id.clone())
        .await
        .map_err(|e| Error::storage(e.to_string()))?;
    let state = app.0.state.lock().await;
    let mut devices: Vec<Device> = state
        .peers
        .values()
        .filter(|d| now().saturating_sub(d.last_seen) < 90)
        .cloned()
        .collect();
    if !p.owner {
        devices.push(Device {
            id: info.id.clone(),
            name: info.name.clone(),
            platform: info.platform.clone(),
            address: String::new(),
            last_seen: now(),
            kind: "host".into(),
            host_device_id: info.id.clone(),
            public_key: info.public_key.clone(),
            capabilities: info.capabilities.clone(),
        });
    }
    devices.extend(
        state
            .sessions
            .values()
            .filter(|s| s.id != p.id && now().saturating_sub(s.last_seen) < 45)
            .map(|s| Device {
                id: s.id.clone(),
                name: s.name.clone(),
                platform: "浏览器".into(),
                address: String::new(),
                last_seen: s.last_seen,
                kind: "browser".into(),
                host_device_id: info.id.clone(),
                public_key: info.public_key.clone(),
                capabilities: info.capabilities.clone(),
            }),
    );
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    let mut transfers: Vec<_> = state
        .transfers
        .values()
        .filter(|t| t.view.sender_id == p.id || t.view.recipient_id == p.id)
        .map(|t| t.view.clone())
        .collect();
    transfers.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));
    let groups: Vec<_> = state
        .groups
        .values()
        .filter(|g| g.members.iter().any(|m| m.device.id == p.id))
        .cloned()
        .map(|mut group| {
            for member in &mut group.members {
                if let Some(device) = state.profiles.get(&member.device.id) {
                    member.device.name = device.name.clone();
                }
            }
            group
        })
        .collect();
    Ok(Json(
        json!({"me": {"id":p.id,"name":p.name,"owner":p.owner}, "info":info,"devices":devices,"transfers":transfers,"conversations":conversations,"groups":groups,"storage_used":state.reserved,"storage_limit":MAX_STORAGE}),
    ))
}
async fn settings(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<Json<Value>> {
    if !app.principal(token(&headers)?).await?.owner {
        return Err(Error::unauthorized());
    }
    let name = crate::clean_name(value["name"].as_str().unwrap_or(""))
        .map_err(|e| Error::bad(e.to_string()))?;
    let mut state = app.0.state.lock().await;
    state
        .profiles
        .get_mut(&app.0.device_id)
        .ok_or_else(Error::missing)?
        .name = name.clone();
    app.commit(&mut state).await?;
    drop(state);
    app.0.config.lock().await.name = name;
    if let Err(e) = crate::discovery::announce(&app).await {
        eprintln!("Device announcement update failed: {e}");
    }
    app.notify();
    Ok(Json(json!({"ok":true})))
}
pub(crate) fn local_address(address: &str) -> Result<String> {
    let url = url::Url::parse(address)
        .map_err(|_| Error::bad("请输入完整局域网地址，例如 http://192.168.1.20:52783"))?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(Error::bad("仅支持不带路径的局域网 HTTP 地址"));
    }
    let Some(url::Host::Ipv4(ip)) = url.host() else {
        return Err(Error::bad("请使用局域网 IPv4 地址"));
    };
    if !(ip.is_private() || ip.is_loopback() || ip.is_link_local())
        || url.port_or_known_default() == Some(0)
    {
        return Err(Error::bad("仅支持局域网地址"));
    }
    Ok(url.origin().ascii_serialization())
}
async fn add_peer(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<Json<Value>> {
    app.principal(token(&headers)?).await?;
    let address = local_address(value["address"].as_str().unwrap_or(""))?;
    let response = app
        .0
        .client
        .get(format!("{address}/api/info"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| Error::bad("连接失败，请检查地址、网络和防火墙"))?;
    if !response.status().is_success() || response.content_length().unwrap_or(0) > 64 * 1024 {
        return Err(Error::bad("对方没有返回有效设备信息"));
    }
    let info: Value = response
        .json()
        .await
        .map_err(|_| Error::bad("对方不是邻传设备"))?;
    if info["protocol"] != 1 {
        return Err(Error::bad("协议版本不兼容，请升级应用"));
    }
    let id = info["id"]
        .as_str()
        .filter(|s| uuid::Uuid::parse_str(s).is_ok())
        .ok_or_else(|| Error::bad("设备 ID 无效"))?
        .to_owned();
    if id == app.info().await.id {
        return Err(Error::bad("这是当前设备的地址"));
    }
    let device = Device {
        id: id.clone(),
        name: info["name"]
            .as_str()
            .unwrap_or("邻传设备")
            .chars()
            .take(64)
            .collect(),
        platform: info["platform"]
            .as_str()
            .unwrap_or("设备")
            .chars()
            .take(20)
            .collect(),
        address,
        last_seen: now(),
        kind: "app".into(),
        host_device_id: id.clone(),
        public_key: info["public_key"].as_str().unwrap_or("").into(),
        capabilities: serde_json::from_value(info["capabilities"].clone()).unwrap_or_default(),
    };
    app.remember_peer(device).await?;
    Ok(Json(json!({"id":id})))
}
fn validate_files(files: &[FileInput]) -> Result<u64> {
    if files.len() > 64 {
        return Err(Error::bad("一次最多发送 64 个文件"));
    }
    let mut total = 0;
    for file in files {
        if !crate::safe_filename(&file.name) || file.size > MAX_FILE || file.mime.len() > 128 {
            return Err(Error::bad("文件名无效或单文件超过 2 GiB"));
        }
        total += file.size;
    }
    if total > MAX_STORAGE {
        return Err(Error::bad("本次文件总量超过 10 GiB"));
    }
    Ok(total)
}
async fn create_draft(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Json(request): Json<DraftRequest>,
) -> Result<Json<DraftResponse>> {
    let p = app.principal(token(&headers)?).await?;
    let total = validate_files(&request.files)?;
    let id = uid();
    let mut state = app.0.state.lock().await;
    if state.reserved + total > MAX_STORAGE || state.drafts.len() >= 256 {
        return Err(Error::conflict(
            "临时空间不足或草稿过多，请稍后重试或重启应用清理",
        ));
    }
    tokio::fs::create_dir(app.0.work.join(&id)).await?;
    let files: Vec<_> = request
        .files
        .into_iter()
        .map(|f| {
            let file_id = uid();
            StoredFile {
                path: app.0.work.join(&id).join(&file_id),
                meta: FileMeta {
                    id: file_id,
                    name: f.name,
                    size: f.size,
                    mime: f.mime,
                },
                complete: false,
                uploading: false,
            }
        })
        .collect();
    let response = DraftResponse {
        id: id.clone(),
        files: files.iter().map(|f| f.meta.clone()).collect(),
    };
    state.reserved += total;
    state.drafts.insert(
        id.clone(),
        Draft {
            id,
            owner: p.id,
            files,
            created: now(),
            sent: false,
        },
    );
    Ok(Json(response))
}
async fn write_file(body: Body, file: &StoredFile, progress: Option<(App, String)>) -> Result<()> {
    let part = file.path.with_extension("part");
    let result = async {
        let mut output = tokio::fs::File::create(&part).await?;
        let mut stream = body.into_data_stream();
        let mut count = 0u64;
        let mut last_progress = std::time::Instant::now();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Error::bad("上传中断，请重新发送"))?;
            count += chunk.len() as u64;
            if count > file.meta.size {
                return Err(Error::bad("上传内容超过声明的大小"));
            }
            output.write_all(&chunk).await?;
            if last_progress.elapsed() >= std::time::Duration::from_millis(200) {
                if let Some((app, id)) = progress.as_ref() {
                    let mut state = app.0.state.lock().await;
                    let transfer = state.transfers.get_mut(id).ok_or_else(Error::missing)?;
                    if transfer.view.terminal() {
                        return Err(Error::conflict("传输已结束"));
                    }
                    transfer.view.bytes = transfer
                        .files
                        .iter()
                        .filter(|f| f.complete)
                        .map(|f| f.meta.size)
                        .sum::<u64>()
                        + count;
                    drop(state);
                    app.notify();
                }
                last_progress = std::time::Instant::now();
            }
        }
        if count != file.meta.size {
            return Err(Error::bad("文件不完整，请重新发送"));
        }
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        tokio::fs::rename(&part, &file.path).await?;
        crate::sync_directory(file.path.parent().unwrap().to_path_buf()).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result
}
async fn upload_draft(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path((id, file_id)): Path<(String, String)>,
    body: Body,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    let _permit = app
        .0
        .streams
        .try_acquire()
        .map_err(|_| Error::conflict("同时传输过多，请稍后重试"))?;
    let file = {
        let mut state = app.0.state.lock().await;
        let draft = state
            .drafts
            .get_mut(&id)
            .filter(|d| d.owner == p.id)
            .ok_or_else(Error::missing)?;
        if draft.sent {
            return Err(Error::conflict("该草稿已发送"));
        }
        let file = draft
            .files
            .iter_mut()
            .find(|f| f.meta.id == file_id)
            .ok_or_else(Error::missing)?;
        if file.uploading || file.complete {
            return Err(Error::conflict("文件已上传或正在上传"));
        }
        file.uploading = true;
        file.clone()
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3600),
        write_file(body, &file, None),
    )
    .await
    .unwrap_or_else(|_| Err(Error::bad("上传超时")));
    let mut state = app.0.state.lock().await;
    if let Some(file) = state
        .drafts
        .get_mut(&id)
        .and_then(|d| d.files.iter_mut().find(|f| f.meta.id == file_id))
    {
        file.uploading = false;
        file.complete = result.is_ok();
    }
    result?;
    Ok(Json(json!({"ok":true})))
}
async fn send(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<SendRequest>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    if request.text.len() > 65536 {
        return Err(Error::bad("文字最多 64 KiB"));
    }
    let info = app.info().await;
    let mut state = app.0.state.lock().await;
    let mut draft = state
        .drafts
        .get(&id)
        .filter(|d| d.owner == p.id)
        .ok_or_else(Error::missing)?
        .clone();
    if draft.sent || draft.files.iter().any(|f| !f.complete) {
        return Err(Error::conflict("文件尚未上传完成或草稿已发送"));
    }
    if draft.files.is_empty() && request.text.trim().is_empty() {
        return Err(Error::bad("请添加文字或文件"));
    }
    if state
        .transfers
        .values()
        .filter(|t| !t.view.terminal())
        .count()
        >= 128
    {
        return Err(Error::conflict("同时进行的传输过多"));
    }
    let mut targets = request.targets;
    if let Some(group_id) = &request.group_id {
        let group = state
            .groups
            .get(group_id)
            .filter(|g| {
                g.members
                    .iter()
                    .any(|m| m.device.id == p.id && m.state == "joined")
            })
            .ok_or_else(Error::missing)?;
        targets = group
            .members
            .iter()
            .filter(|m| m.state == "joined" && m.device.id != p.id)
            .map(|m| m.device.id.clone())
            .collect();
    } else if targets.is_empty() || targets.len() > 8 {
        return Err(Error::bad("接收设备需为 1–8 台"));
    }
    targets.sort();
    targets.dedup();
    if targets.is_empty() {
        return Err(Error::bad("群内暂无其他已加入成员"));
    }
    let mut resolved = Vec::new();
    for target in targets {
        if target == p.id {
            return Err(Error::bad("不能发给当前会话"));
        }
        let device = state
            .profiles
            .get(&target)
            .ok_or_else(Error::missing)?
            .clone();
        let address = if target == info.id
            || state.sessions.values().any(|s| {
                s.id == target && s.expires > now() && now().saturating_sub(s.last_seen) < 45
            }) {
            Some(None)
        } else if let Some(peer) = state
            .peers
            .get(if device.host_device_id.is_empty() {
                &target
            } else {
                &device.host_device_id
            })
            .filter(|d| now().saturating_sub(d.last_seen) < 90)
        {
            Some(Some(peer.address.clone()))
        } else if request.group_id.is_some()
            && device.host_device_id != app.0.device_id
            && !device.address.is_empty()
        {
            Some(Some(device.address.clone()))
        } else {
            None
        };
        if address.is_none() && request.group_id.is_none() {
            return Err(Error::bad("接收设备已离线，请刷新后重试"));
        }
        resolved.push((device, address));
    }
    let original_draft = draft.clone();
    let destination = app.0.directory.join("files").join(&draft.id);
    tokio::fs::rename(app.0.work.join(&draft.id), &destination).await?;
    crate::sync_directory(app.0.directory.join("files")).await?;
    for file in &mut draft.files {
        file.path = destination.join(&file.meta.id);
    }
    let message_id = uid();
    let created_at = now_ms();
    let mut workers = Vec::new();
    let mut ids = Vec::new();
    for (device, address) in resolved {
        let id = uid();
        let offline = address.is_none();
        let completed =
            request.group_id.is_some() && draft.files.is_empty() && matches!(address, Some(None));
        let view = TransferView {
            id: id.clone(),
            message_id: message_id.clone(),
            group_id: request.group_id.clone(),
            created_at,
            sender_id: p.id.clone(),
            sender_name: p.name.clone(),
            recipient_id: device.id,
            recipient_name: device.name,
            text: request.text.clone(),
            files: draft.files.iter().map(|f| f.meta.clone()).collect(),
            status: if offline {
                "failed"
            } else if completed {
                "completed"
            } else {
                "pending"
            }
            .into(),
            bytes: 0,
            total: draft.files.iter().map(|f| f.meta.size).sum(),
            created: now(),
            error: if offline {
                Some("接收设备离线，未自动排队".into())
            } else {
                None
            },
        };
        state.transfers.insert(
            id.clone(),
            Transfer {
                view,
                files: draft.files.clone(),
                peer_token: None,
                incoming_remote: false,
                reserved: 0,
            },
        );
        if let Some(Some(address)) = address {
            workers.push((id.clone(), address));
        }
        ids.push(id);
    }
    draft.sent = true;
    state.drafts.insert(draft.id.clone(), draft);
    let conversation = if let Some(g) = &request.group_id {
        Conversation::group(&p.id, g)
    } else {
        Conversation::direct(&p.id, &state.transfers[&ids[0]].view.recipient_id)
    };
    state
        .composition
        .insert(conversation.id.clone(), String::new());
    if let Err(error) = app.commit(&mut state).await {
        let _ = tokio::fs::rename(&destination, app.0.work.join(&id)).await;
        state.reserved += original_draft
            .files
            .iter()
            .map(|f| f.meta.size)
            .sum::<u64>();
        state
            .drafts
            .insert(original_draft.id.clone(), original_draft);
        return Err(error);
    }
    drop(state);
    app.notify();
    for (id, address) in workers {
        let app = app.clone();
        tokio::spawn(async move {
            crate::transport::send(app, id, address).await;
        });
    }
    Ok(Json(
        json!({"ids":ids,"message_id":message_id,"conversation_id":conversation.id}),
    ))
}
async fn decision(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(value): Json<Value>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    let accept = value["accept"]
        .as_bool()
        .ok_or_else(|| Error::bad("缺少接收决定"))?;
    app.cache_transfer(&id).await?;
    let mut state = app.0.state.lock().await;
    let transfer = state
        .transfers
        .get_mut(&id)
        .filter(|t| t.view.recipient_id == p.id)
        .ok_or_else(Error::missing)?;
    if transfer.view.status != "pending" {
        return Err(Error::conflict("该请求已处理"));
    }
    transfer.view.status = if !accept {
        "rejected"
    } else if transfer.incoming_remote {
        "accepted"
    } else {
        "completed"
    }
    .into();
    if transfer.view.status == "completed" {
        transfer.view.bytes = transfer.view.total;
    }
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(json!({"ok":true})))
}
async fn cancel(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    app.cache_transfer(&id).await?;
    let mut state = app.0.state.lock().await;
    let transfer = state
        .transfers
        .get_mut(&id)
        .filter(|t| t.view.sender_id == p.id || t.view.recipient_id == p.id)
        .ok_or_else(Error::missing)?;
    if transfer.view.terminal() {
        return Err(Error::conflict("该传输已结束"));
    }
    transfer.view.status = "cancelled".into();
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(json!({"ok":true})))
}
async fn ticket(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path((id, file_id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    app.cache_transfer(&id).await?;
    let mut state = app.0.state.lock().await;
    let transfer = state
        .transfers
        .get(&id)
        .filter(|t| {
            t.view.sender_id == p.id
                || (t.view.recipient_id == p.id && t.view.status == "completed")
        })
        .ok_or_else(Error::missing)?;
    let file = transfer
        .files
        .iter()
        .find(|f| f.meta.id == file_id && f.complete && f.path.is_file())
        .ok_or_else(Error::missing)?
        .clone();
    if state.tickets.len() >= 2048 {
        return Err(Error::conflict("下载请求过多，请稍后重试"));
    }
    let key = secret();
    state.tickets.insert(
        key.clone(),
        Ticket {
            path: file.path,
            name: file.meta.name,
            expires: now() + 120,
        },
    );
    Ok(Json(json!({"url":format!("/download/{key}")})))
}
async fn download(AxState(app): AxState<App>, Path(key): Path<String>) -> Result<Response> {
    let ticket = app
        .0
        .state
        .lock()
        .await
        .tickets
        .remove(&key)
        .filter(|t| t.expires > now())
        .ok_or_else(Error::missing)?;
    let file = tokio::fs::File::open(ticket.path).await?;
    let length = file.metadata().await?.len();
    let encoded: String = url::form_urlencoded::byte_serialize(ticket.name.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    let mut response = Body::from_stream(ReaderStream::new(file)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert("content-length", length.into());
    headers.insert(
        "content-disposition",
        HeaderValue::from_str(&format!(
            "attachment; filename=\"download\"; filename*=UTF-8''{encoded}"
        ))
        .map_err(|_| Error::bad("文件名无效"))?,
    );
    Ok(response)
}
async fn peer_offer(
    AxState(app): AxState<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(mut offer): Json<Offer>,
) -> Result<Json<OfferResponse>> {
    if offer.protocol != 1
        || offer.text.len() > 65536
        || crate::clean_name(&offer.sender_name).is_err()
        || (offer.files.is_empty() && offer.text.trim().is_empty())
    {
        return Err(Error::bad("传输请求无效或版本不兼容"));
    }
    let input: Vec<_> = offer
        .files
        .iter()
        .map(|f| FileInput {
            name: f.name.clone(),
            size: f.size,
            mime: f.mime.clone(),
        })
        .collect();
    let total = validate_files(&input)?;
    let mut file_ids = std::collections::HashSet::new();
    if offer.files.iter().any(|f| {
        f.id.len() != 36 || uuid::Uuid::parse_str(&f.id).is_err() || !file_ids.insert(&f.id)
    }) {
        return Err(Error::bad("文件标识无效"));
    }
    let info = app.info().await;
    let recipient = if offer.target_id.is_empty() {
        info.id.clone()
    } else {
        offer.target_id.clone()
    };
    let mut state = app.0.state.lock().await;
    rate_limit(&mut state, format!("offer:{}", addr.ip()), 60)?;
    if recipient != info.id
        && !state
            .sessions
            .values()
            .any(|s| s.id == recipient && s.expires > now())
    {
        return Err(Error::bad("接收身份不存在"));
    }
    let sender = if let Some(env) = &offer.identity {
        crate::identity::verify(env).map_err(|e| Error::bad(e.to_string()))?;
        if env.payload != crate::identity::offer_payload(&offer)
            || uuid::Uuid::parse_str(&offer.message_id).is_err()
        {
            return Err(Error::bad("消息签名与内容不符"));
        }
        if offer.sender_id != env.host_id
            && !offer
                .sender_id
                .strip_prefix("browser-")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        {
            return Err(Error::bad("发送身份无效"));
        }
        if let Some(host) = state.profiles.get(&env.host_id) {
            if !host.public_key.is_empty() && host.public_key != env.public_key {
                return Err(Error::bad("发送宿主身份不匹配"));
            }
        }
        if let Some(old) = state.profiles.get(&offer.sender_id) {
            if !old.public_key.is_empty()
                && (old.public_key != env.public_key || old.host_device_id != env.host_id)
            {
                return Err(Error::bad("发送身份与已知设备不符"));
            }
        }
        if let Some(group_id) = &offer.group_id {
            if let Some(proof) = &offer.membership {
                crate::groups::apply(&mut state, proof, &app.0.device_id)?;
            }
            let group = state.groups.get(group_id).ok_or_else(Error::missing)?;
            if !group
                .members
                .iter()
                .any(|m| m.device.id == recipient && m.state == "joined")
                || !group.members.iter().any(|m| {
                    m.device.id == offer.sender_id
                        && m.state == "joined"
                        && m.device.public_key == env.public_key
                        && m.device.host_device_id == env.host_id
                })
            {
                return Err(Error::bad("发送者或接收者尚未加入该群"));
            }
        }
        let mut profile = state
            .profiles
            .get(&offer.sender_id)
            .cloned()
            .unwrap_or_else(|| Device {
                id: offer.sender_id.clone(),
                kind: if offer.sender_id == env.host_id {
                    "app"
                } else {
                    "browser"
                }
                .into(),
                platform: "设备".into(),
                host_device_id: env.host_id.clone(),
                public_key: env.public_key.clone(),
                ..Default::default()
            });
        profile.name = offer.sender_name.clone();
        state.profiles.insert(profile.id.clone(), profile);
        offer.sender_id.clone()
    } else {
        if offer.group_id.is_some() {
            return Err(Error::bad("群消息必须验证发送身份"));
        }
        offer.message_id = uid();
        offer.created_at = now_ms();
        format!(
            "legacy-{}",
            &crate::identity::token_hash(&format!("{}:{}", addr.ip(), offer.sender_name))[..24]
        )
    };
    if let Some(existing) = state
        .transfers
        .values_mut()
        .find(|t| t.view.message_id == offer.message_id && t.view.recipient_id == recipient)
    {
        if existing.view.sender_id != sender
            || existing.view.text != offer.text
            || existing.view.group_id != offer.group_id
            || serde_json::to_value(&existing.view.files).unwrap()
                != serde_json::to_value(&offer.files).unwrap()
        {
            return Err(Error::conflict("消息标识冲突"));
        }
        let token = existing.peer_token.get_or_insert_with(secret).clone();
        return Ok(Json(OfferResponse {
            id: existing.view.id.clone(),
            token,
        }));
    }
    if app
        .0
        .store
        .message_exists(offer.message_id.clone(), recipient.clone())
        .await
        .map_err(|e| Error::storage(e.to_string()))?
    {
        return Err(Error::conflict("消息已收录，请勿重复发送"));
    }
    if state.reserved + total > MAX_STORAGE
        || state
            .transfers
            .values()
            .filter(|t| !t.view.terminal() && t.incoming_remote)
            .count()
            >= 32
    {
        return Err(Error::conflict("接收队列或文件空间已满"));
    }
    let id = uid();
    let token = secret();
    let path = app.0.directory.join("files").join(&id);
    tokio::fs::create_dir(&path).await?;
    let files = offer
        .files
        .iter()
        .map(|f| StoredFile {
            meta: f.clone(),
            path: path.join(&f.id),
            complete: false,
            uploading: false,
        })
        .collect();
    let recipient_name = state
        .profiles
        .get(&recipient)
        .map(|d| d.name.clone())
        .unwrap_or(info.name);
    let text_only_group = offer.group_id.is_some() && offer.files.is_empty();
    let view = TransferView {
        id: id.clone(),
        message_id: offer.message_id,
        group_id: offer.group_id,
        created_at: offer.created_at,
        sender_id: sender,
        sender_name: offer.sender_name,
        recipient_id: recipient,
        recipient_name,
        text: offer.text,
        files: offer.files,
        status: if text_only_group {
            "completed"
        } else {
            "pending"
        }
        .into(),
        bytes: 0,
        total,
        created: now(),
        error: None,
    };
    state.reserved += total;
    state.transfers.insert(
        id.clone(),
        Transfer {
            view,
            files,
            peer_token: Some(token.clone()),
            incoming_remote: true,
            reserved: total,
        },
    );
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(OfferResponse { id, token }))
}
fn authorized_transfer<'a>(
    state: &'a mut crate::model::State,
    id: &str,
    token: &str,
) -> Result<&'a mut Transfer> {
    state
        .transfers
        .get_mut(id)
        .filter(|t| t.incoming_remote && t.peer_token.as_deref() == Some(token))
        .ok_or_else(Error::missing)
}
async fn peer_status(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TransferView>> {
    let mut state = app.0.state.lock().await;
    Ok(Json(
        authorized_transfer(&mut state, &id, token(&headers)?)?
            .view
            .clone(),
    ))
}
async fn peer_upload(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path((id, file_id)): Path<(String, String)>,
    body: Body,
) -> Result<Json<Value>> {
    let _permit = app
        .0
        .streams
        .try_acquire()
        .map_err(|_| Error::conflict("同时传输过多"))?;
    let file = {
        let mut state = app.0.state.lock().await;
        let t = authorized_transfer(&mut state, &id, token(&headers)?)?;
        if !matches!(t.view.status.as_str(), "accepted" | "transferring") {
            return Err(Error::conflict("对方尚未同意或传输已结束"));
        }
        let file = t
            .files
            .iter_mut()
            .find(|f| f.meta.id == file_id)
            .ok_or_else(Error::missing)?;
        if file.uploading || file.complete {
            return Err(Error::conflict("文件已上传或正在上传"));
        }
        file.uploading = true;
        t.view.status = "transferring".into();
        file.clone()
    };
    app.notify();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3600),
        write_file(body, &file, Some((app.clone(), id.clone()))),
    )
    .await
    .unwrap_or_else(|_| Err(Error::bad("上传超时")));
    let mut state = app.0.state.lock().await;
    let t = authorized_transfer(&mut state, &id, token(&headers)?)?;
    let active = matches!(t.view.status.as_str(), "accepted" | "transferring");
    if let Some(f) = t.files.iter_mut().find(|f| f.meta.id == file_id) {
        f.uploading = false;
        f.complete = result.is_ok() && active;
    }
    if result.is_err() && active {
        t.view.status = "failed".into();
        t.view.error = Some("文件上传中断，请重新发送".into());
    }
    t.view.bytes = t
        .files
        .iter()
        .filter(|f| f.complete)
        .map(|f| f.meta.size)
        .sum();
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    result?;
    if !active {
        return Err(Error::conflict("传输已取消"));
    }
    Ok(Json(json!({"ok":true})))
}
async fn peer_complete(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let mut state = app.0.state.lock().await;
    let t = authorized_transfer(&mut state, &id, token(&headers)?)?;
    if !matches!(t.view.status.as_str(), "accepted" | "transferring")
        || t.files.iter().any(|f| !f.complete)
    {
        return Err(Error::conflict("传输尚未完成或已结束"));
    }
    t.view.status = "completed".into();
    t.view.bytes = t.view.total;
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(json!({"ok":true})))
}
async fn peer_cancel(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let mut state = app.0.state.lock().await;
    let t = authorized_transfer(&mut state, &id, token(&headers)?)?;
    if !t.view.terminal() {
        t.view.status = "cancelled".into();
    }
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(json!({"ok":true})))
}
async fn events(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let host = headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !matches!(
            origin,
            "tauri://localhost"
                | "http://tauri.localhost"
                | "https://tauri.localhost"
                | "http://localhost:1420"
                | "http://127.0.0.1:1420"
        ) && origin != format!("http://{host}")
        {
            return Err(Error::unauthorized());
        }
    }
    Ok(ws.on_upgrade(move |mut socket| async move {
        let first=tokio::time::timeout(std::time::Duration::from_secs(5),socket.recv()).await;
        let Ok(Some(Ok(axum::extract::ws::Message::Text(message))))=first else {return};
        let Ok(value)=serde_json::from_str::<Value>(&message) else{return};
        let Some(token)=value["token"].as_str() else{return};
        if app.principal(token).await.is_err(){return;}
        let mut events=app.0.events.subscribe();
        let mut heartbeat=tokio::time::interval(std::time::Duration::from_secs(20));
        loop {tokio::select!{
            result=events.recv()=>{if result.is_err_and(|e|matches!(e,broadcast::error::RecvError::Closed)){break;}
                    if socket.send(axum::extract::ws::Message::Text("changed".into())).await.is_err(){break;}},
            _=heartbeat.tick()=>{if socket.send(axum::extract::ws::Message::Ping(Vec::new().into())).await.is_err(){break;}},
            message=socket.recv()=>{if matches!(message,None|Some(Err(_))|Some(Ok(axum::extract::ws::Message::Close(_)))){break;}}
        }}
    }))
}
use tokio::sync::broadcast;
#[derive(rust_embed::RustEmbed)]
#[folder = "../../dist/"]
struct Assets;
async fn asset(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(file) => (
            [(
                "content-type",
                mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string(),
            )],
            file.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn logout(AxState(app): AxState<App>, headers: HeaderMap) -> Result<Json<Value>> {
    let credential = token(&headers)?;
    let p = app.principal(credential).await?;
    if p.owner {
        return Err(Error::bad("本机身份不能通过网页退出"));
    }
    let mut state = app.0.state.lock().await;
    state
        .sessions
        .remove(&crate::identity::token_hash(credential));
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(json!({"ok":true})))
}
