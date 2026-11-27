use crate::{
    model::*,
    receive_conflicts::{ConflictChoices, FileConflict},
    App,
};
use axum::{
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State as AxState, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use tower_http::cors::CorsLayer;

#[derive(Debug)]
pub struct Error(StatusCode, String, Vec<FileConflict>);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}
impl std::error::Error for Error {}
impl Error {
    pub(crate) fn file_conflicts(files: Vec<FileConflict>) -> Self {
        Self(
            StatusCode::CONFLICT,
            "请选择同名文件的处理方式".into(),
            files,
        )
    }
    pub fn conflicts(&self) -> &[FileConflict] {
        &self.2
    }
    pub fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into(), vec![])
    }
    pub fn storage(message: impl Into<String>) -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("数据库操作失败：{}", message.into()),
            vec![],
        )
    }
    pub fn unauthorized() -> Self {
        Self(
            StatusCode::UNAUTHORIZED,
            "连接已失效，请重新配对".into(),
            vec![],
        )
    }
    pub fn forbidden() -> Self {
        Self(
            StatusCode::FORBIDDEN,
            "浏览器只能与当前连接的客户端互传".into(),
            vec![],
        )
    }
    pub(crate) fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "内容不存在或已过期".into(), vec![])
    }
    pub(crate) fn gone(message: impl Into<String>) -> Self {
        Self(StatusCode::GONE, message.into(), vec![])
    }
    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self(StatusCode::CONFLICT, message.into(), vec![])
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut body = json!({"error": self.1});
        if !self.2.is_empty() {
            body["conflicts"] = json!(self.2);
        }
        (self.0, Json(body)).into_response()
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("文件操作失败：{e}"),
            vec![],
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

#[derive(Clone, Copy)]
pub(crate) struct BrowserTransport(pub bool);

async fn check_transport(
    AxState(app): AxState<App>,
    Extension(transport): Extension<BrowserTransport>,
    request: Request,
    next: axum::middleware::Next,
) -> Result<Response> {
    if let Ok(credential) = token(request.headers()) {
        if !app
            .credential_matches_transport(credential, transport.0)
            .await
        {
            return Err(Error::unauthorized());
        }
    }
    Ok(next.run(request).await)
}

pub fn router(app: App, secure: bool) -> Router {
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
        .route("/scan/{key}", get(scan_page))
        .route(
            "/api/scan",
            get(crate::scan::list).post(crate::scan::create),
        )
        .route("/api/scan/{id}/close", post(crate::scan::close))
        .route("/api/scan/visit/{key}", get(crate::scan::visit))
        .route(
            "/api/scan/visit/{key}/files/{id}",
            get(crate::scan::download)
                .put(crate::scan::submit)
                .layer(DefaultBodyLimit::disable()),
        )
        .route("/api/pair", post(pair))
        .route("/api/session/logout", post(logout))
        .route("/api/state", get(state))
        .route(
            "/api/browser/assistance",
            get(crate::assistance::browser_session),
        )
        .route(
            "/api/browser/assistance/{id}",
            post(crate::assistance::browser_action).layer(DefaultBodyLimit::max(96 * 1024)),
        )
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
        .route("/api/drafts/{id}/cancel", post(discard_draft))
        .route("/api/transfers/{id}/decision", post(decision))
        .route(
            "/api/transfers/{id}/files/{file}/upload",
            put(browser_upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/transfers/{id}/cancel", post(cancel))
        .route("/api/transfers/{id}/files/{file}/ticket", post(ticket))
        .route("/download/{ticket}", get(download))
        .route("/api/events", get(events))
        .route("/api/peer/offer", post(peer_offer))
        .route(
            "/api/peer/assistance",
            post(crate::assistance::receive).layer(DefaultBodyLimit::max(96 * 1024)),
        )
        .route("/api/peer/transfers/{id}", get(peer_status))
        .route(
            "/api/peer/transfers/{id}/files/{file}",
            put(peer_upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/peer/transfers/{id}/complete", post(peer_complete))
        .route("/api/peer/transfers/{id}/cancel", post(peer_cancel))
        .fallback(get(asset))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            check_transport,
        ))
        .layer(Extension(BrowserTransport(secure)))
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
    Extension(BrowserTransport(secure)): Extension<BrowserTransport>,
    headers: HeaderMap,
    Json(request): Json<PairRequest>,
) -> Result<Json<Value>> {
    let id = format!("browser-{}", uid());
    let name = if request.name.trim().is_empty() {
        crate::names::for_id(&id)
    } else {
        crate::clean_name(&request.name).map_err(|e| Error::bad(e.to_string()))?
    };
    let mut state = app.0.state.lock().await;
    rate_limit(&mut state, format!("pair:{secure}:{}", addr.ip()), 12)?;
    let code = if secure {
        &app.0
            .browser_https
            .as_ref()
            .ok_or_else(Error::unauthorized)?
            .pairing_code
    } else {
        &app.0.code
    };
    if &request.code != code {
        return Err(Error::unauthorized());
    }
    if state.sessions.len() >= 64 {
        return Err(Error::conflict("连接数量已达上限"));
    }
    let token = secret();
    state.sessions.insert(
        crate::identity::token_hash(&token),
        Session {
            id: id.clone(),
            name: name.clone(),
            last_seen: now(),
            expires: now() + RETENTION,
            secure,
        },
    );
    state.profiles.insert(
        id.clone(),
        Device {
            id: id.clone(),
            name: name.clone(),
            platform: crate::names::browser_platform(
                headers
                    .get("user-agent")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default(),
            ),
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
    Ok(Json(json!({"token": token, "id": id, "name": name})))
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
            vec![],
        ));
    }
    Ok(())
}
async fn state(AxState(app): AxState<App>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    let info = app.info().await;
    let mut conversations = app
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
            .filter(|s| p.owner && s.online())
            .map(|s| Device {
                id: s.id.clone(),
                name: s.name.clone(),
                platform: state
                    .profiles
                    .get(&s.id)
                    .map(|d| d.platform.clone())
                    .unwrap_or_else(|| "浏览器".into()),
                address: String::new(),
                last_seen: s.last_seen,
                kind: "browser".into(),
                host_device_id: info.id.clone(),
                public_key: info.public_key.clone(),
                capabilities: crate::assistance::browser_capabilities(
                    app.0.assistance.is_some(),
                    s.secure,
                ),
            }),
    );
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    let mut transfers: Vec<_> = state
        .transfers
        .values()
        .filter(|t| {
            (t.view.sender_id == p.id || t.view.recipient_id == p.id)
                && (p.owner
                    || (t.view.group_id.is_none()
                        && (t.view.sender_id == app.0.device_id
                            || t.view.recipient_id == app.0.device_id)))
        })
        .map(|t| t.view.clone())
        .collect();
    transfers.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));
    let groups: Vec<_> = state
        .groups
        .values()
        .filter(|g| p.owner && g.members.iter().any(|m| m.device.id == p.id))
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
    if !p.owner {
        conversations.retain(|c| {
            c.conversation.kind == "direct"
                && c.conversation.peer_id.as_deref() == Some(app.0.device_id.as_str())
        });
        devices.clear();
    }
    let platform = state
        .profiles
        .get(&p.id)
        .map(|d| d.platform.as_str())
        .unwrap_or("浏览器");
    let available = app.0.space.available(app.cache_location()).await.ok();
    Ok(Json(
        json!({"me": {"id":p.id,"name":p.name,"owner":p.owner,"platform":platform,"auto_name":p.owner.then(|| crate::names::for_id(&p.id))}, "info":info,"devices":devices,"transfers":transfers,"conversations":conversations,"groups":groups,"storage_used":state.reserved,"storage_available":available,"storage_headroom":crate::storage::DISK_HEADROOM}),
    ))
}
async fn settings(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<Json<Value>> {
    let credential = token(&headers)?;
    let principal = app.principal(credential).await?;
    let name = crate::clean_name(value["name"].as_str().unwrap_or(""))
        .map_err(|e| Error::bad(e.to_string()))?;
    let mut state = app.0.state.lock().await;
    let State {
        profiles, sessions, ..
    } = &mut *state;
    let profile = profiles.get_mut(&principal.id).ok_or_else(Error::missing)?;
    let session = if principal.owner {
        None
    } else {
        Some(
            sessions
                .get_mut(&crate::identity::token_hash(credential))
                .ok_or_else(Error::unauthorized)?,
        )
    };
    profile.name = name.clone();
    if let Some(session) = session {
        session.name = name.clone();
    }
    app.commit(&mut state).await?;
    drop(state);
    if principal.owner {
        app.0.config.lock().await.name = name.clone();
        if let Err(e) = crate::discovery::announce(&app).await {
            eprintln!("Device announcement update failed: {e}");
        }
    }
    app.notify();
    Ok(Json(json!({"ok":true,"name":name})))
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
    if !app.principal(token(&headers)?).await?.owner {
        return Err(Error::forbidden());
    }
    let address = local_address(value["address"].as_str().unwrap_or(""))?;
    // Addresses may be reassigned. remember_peer checks the stable device ID
    // and its recorded key without treating an old IP/port as an identity.
    let device = app.inspect_peer(&address, None).await?;
    let id = device.id.clone();
    if id == app.info().await.id {
        return Err(Error::bad("这是当前设备的地址"));
    }
    app.remember_peer(device).await?;
    Ok(Json(json!({"id":id})))
}
pub(crate) fn validate_files(files: &[FileInput]) -> Result<u64> {
    if files.len() > 64 {
        return Err(Error::bad("一次最多发送 64 个文件"));
    }
    let mut total = 0u64;
    for file in files {
        if !crate::safe_filename(&file.name)
            || file.size > crate::storage::MAX_EXACT_SIZE
            || file.mime.len() > 128
        {
            return Err(Error::bad("文件名、大小或类型无效"));
        }
        total = total
            .checked_add(file.size)
            .filter(|n| *n <= crate::storage::MAX_EXACT_SIZE)
            .ok_or_else(|| Error::bad("文件总大小超出可表示范围"))?;
    }
    Ok(total)
}
async fn create_draft(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Json(request): Json<DraftRequest>,
) -> Result<Json<DraftResponse>> {
    let p = app.principal(token(&headers)?).await?;
    validate_files(&request.files)?;
    if request.streaming && p.owner {
        return Err(Error::bad("原生文件请使用本机文件来源"));
    }
    let id = uid();
    let mut files = Vec::new();
    for input in request.files {
        if let Some(source) = input.source_id {
            if !p.owner {
                return Err(Error::forbidden());
            }
            let file = app
                .0
                .sources
                .lock()
                .unwrap()
                .get(&source)
                .map(|v| v.0.clone())
                .ok_or_else(|| Error::bad("原文件引用已失效，请重新选择"))?;
            let mut file = file;
            file.meta.id = uid();
            files.push(file);
        } else {
            let file_id = uid();
            files.push(StoredFile {
                receive_target: None,
                path: app.0.work.join(&id).join(&file_id),
                meta: FileMeta {
                    id: file_id,
                    name: input.name,
                    size: input.size,
                    mime: input.mime,
                },
                complete: false,
                uploading: false,
                storage: if request.streaming {
                    FileStorage::Browser
                } else {
                    FileStorage::Cache
                },
                downloaded: false,
            });
        }
    }
    let lease = app
        .0
        .space
        .reserve(
            files
                .iter()
                .filter(|f| f.cached())
                .map(|f| (f.meta.id.clone(), app.cache_location(), f.meta.size))
                .collect(),
        )
        .await
        .map_err(Error::conflict)?;
    let mut state = app.0.state.lock().await;
    if state.drafts.len() >= 256 {
        return Err(Error::conflict("待发送文件过多，请稍后重试"));
    }
    tokio::fs::create_dir(app.0.work.join(&id)).await?;
    let response = DraftResponse {
        id: id.clone(),
        files: files.iter().map(|f| f.meta.clone()).collect(),
    };
    state.reserved += files
        .iter()
        .filter(|f| f.cached())
        .map(|f| f.meta.size)
        .sum::<u64>();
    state.drafts.insert(
        id.clone(),
        Draft {
            id: id.clone(),
            owner: p.id,
            files,
            created: now(),
            sent: false,
        },
    );
    app.0
        .space_leases
        .lock()
        .unwrap()
        .insert(format!("draft:{id}"), lease);
    Ok(Json(response))
}
async fn write_file(
    body: Body,
    file: &StoredFile,
    progress: Option<(App, String)>,
    space: Option<crate::storage::Progress>,
) -> Result<()> {
    let part = file.path.with_extension("part");
    let result = async {
        let mut output = tokio::fs::File::create(&part).await?;
        if let Some(space) = &space {
            space.set(0);
            space.check().map_err(Error::conflict)?;
        }
        let mut stream = body.into_data_stream();
        let mut count = 0u64;
        let mut last_progress = std::time::Instant::now();
        let mut idle_deadline = tokio::time::Instant::now() + crate::storage::IDLE_TIMEOUT;
        while let Some(chunk) = tokio::time::timeout_at(idle_deadline, stream.next())
            .await
            .map_err(|_| Error::bad("连续 5 分钟未收到文件数据，请重新发送"))?
        {
            let chunk = chunk.map_err(|_| Error::bad("上传中断，请重新发送"))?;
            if chunk.is_empty() {
                continue;
            }
            idle_deadline = tokio::time::Instant::now() + crate::storage::IDLE_TIMEOUT;
            count += chunk.len() as u64;
            if count > file.meta.size {
                return Err(Error::bad("上传内容超过声明的大小"));
            }
            output.write_all(&chunk).await?;
            if let Some(space) = &space {
                space.set(count);
            }
            if last_progress.elapsed() >= std::time::Duration::from_millis(200) {
                if let Some(space) = &space {
                    space.check().map_err(Error::conflict)?;
                }
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
        if let Some(space) = &space {
            space.set(0);
        }
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
        if !file.cached() {
            return Err(Error::bad("此文件不需要准备缓存"));
        }
        if file.uploading || file.complete {
            return Err(Error::conflict("文件已上传或正在上传"));
        }
        file.uploading = true;
        file.clone()
    };
    let result = async {
        let space = app.space_progress(&file.meta.id, &app.cache_location());
        if space.is_some() {
            write_file(body, &file, None, space).await
        } else {
            // A retry after a failed upload must acquire space again.
            let lease = app
                .0
                .space
                .reserve(vec![(
                    file.meta.id.clone(),
                    app.cache_location(),
                    file.meta.size,
                )])
                .await
                .map_err(Error::conflict)?;
            write_file(
                body,
                &file,
                None,
                lease.progress(&file.meta.id, &app.cache_location()),
            )
            .await
        }
    }
    .await;
    let mut state = app.0.state.lock().await;
    if let Some(file) = state
        .drafts
        .get_mut(&id)
        .and_then(|d| d.files.iter_mut().find(|f| f.meta.id == file_id))
    {
        file.uploading = false;
        file.complete = result.is_ok();
    }
    if let Some(draft) = state.drafts.get_mut(&id) {
        draft.created = now();
    }
    if result.is_err() {
        app.0
            .space_leases
            .lock()
            .unwrap()
            .remove(&format!("draft:{id}"));
    }
    result?;
    Ok(Json(json!({"ok":true})))
}
async fn discard_draft(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    let mut state = app.0.state.lock().await;
    let draft = state
        .drafts
        .get(&id)
        .filter(|d| d.owner == p.id)
        .ok_or_else(Error::missing)?;
    if draft.sent {
        return Err(Error::conflict("该草稿已发送"));
    }
    // Active writers hold their own lease until their request has stopped.
    app.0
        .space_leases
        .lock()
        .unwrap()
        .remove(&format!("draft:{id}"));
    if draft.files.iter().any(|f| f.uploading) {
        return Err(Error::conflict("上传正在结束，请稍后重试"));
    }
    let bytes: u64 = draft
        .files
        .iter()
        .filter(|f| f.cached())
        .map(|f| f.meta.size)
        .sum();
    match tokio::fs::remove_dir_all(app.0.work.join(&draft.id)).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    state.drafts.remove(&id);
    state.reserved = state.reserved.saturating_sub(bytes);
    drop(state);
    app.notify();
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
    if draft.sent
        || draft
            .files
            .iter()
            .any(|f| !f.complete && f.storage != FileStorage::Browser)
    {
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
    if !p.owner && (request.group_id.is_some() || targets != [app.0.device_id.clone()]) {
        return Err(Error::forbidden());
    }
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
        if group
            .members
            .iter()
            .any(|m| m.state == "joined" && m.device.id.starts_with("browser-"))
        {
            return Err(Error::bad("此旧群包含浏览器成员，请新建仅包含客户端的群聊"));
        }
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
        if device.kind == "browser" && device.host_device_id != app.0.device_id {
            return Err(Error::forbidden());
        }
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
    let has_cache = draft.files.iter().any(|f| f.cached());
    if has_cache {
        tokio::fs::rename(app.0.work.join(&draft.id), &destination).await?;
        crate::sync_directory(app.0.directory.join("files")).await?;
        for file in &mut draft.files {
            if file.cached() {
                file.path = destination.join(&file.meta.id);
            }
        }
    } else {
        let _ = tokio::fs::remove_dir(app.0.work.join(&draft.id)).await;
    }
    let message_id = uid();
    let created_at = now_ms();
    let mut workers = Vec::new();
    let mut ids = Vec::new();
    for (device, address) in resolved {
        let id = uid();
        let offline = address.is_none();

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
            status: if offline { "failed" } else { "pending" }.into(),
            can_select_files: matches!(address, Some(None)) && draft.files.len() > 1,
            selected_file_ids: None,
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
                stream_bytes: Default::default(),
                peer_token: None,
                incoming_remote: false,
                reserved: 0,
            },
        );
        if matches!(address, Some(None)) {
            auto_receive(&app, &mut state, &id).await;
        }
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
        for id in &ids {
            release_acceptance(&app, id);
        }
        if has_cache {
            let _ = tokio::fs::rename(&destination, app.0.work.join(&id)).await;
        }
        state.reserved += original_draft
            .files
            .iter()
            .filter(|f| f.cached())
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
// Text and image-only messages use the same acceptance checks as a manual decision.
fn automatically_received(files: &[FileMeta]) -> bool {
    files.iter().all(|f| f.mime.starts_with("image/"))
}
struct ReceiveDecision<'a> {
    choices: ConflictChoices,
    accept: bool,
    directory: Option<&'a str>,
    file_ids: Option<Vec<String>>,
}
async fn set_decision(
    app: &App,
    state: &mut crate::model::State,
    id: &str,
    recipient: &str,
    owner: bool,
    ReceiveDecision {
        choices,
        accept,
        directory,
        file_ids,
    }: ReceiveDecision<'_>,
) -> Result<()> {
    let active: Vec<_> = state
        .transfers
        .values()
        .filter(|t| !t.view.terminal() && t.view.id != id)
        .flat_map(|t| t.files.iter().filter_map(|f| f.receive_target.clone()))
        .collect();
    let transfer = state
        .transfers
        .get_mut(id)
        .filter(|t| t.view.recipient_id == recipient)
        .ok_or_else(Error::missing)?;
    if transfer.view.status != "pending" {
        return Err(Error::conflict("该请求已处理"));
    }
    if !owner && (transfer.view.group_id.is_some() || transfer.view.sender_id != app.0.device_id) {
        return Err(Error::forbidden());
    }
    let selected = if let Some(ids) = file_ids {
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        if !accept
            || ids.is_empty()
            || unique.len() != ids.len()
            || !ids
                .iter()
                .all(|id| transfer.view.files.iter().any(|f| &f.id == id))
        {
            return Err(Error::bad("请至少选择一个有效文件"));
        }
        if ids.len() == transfer.files.len() {
            None
        } else if transfer.view.can_select_files {
            Some(ids)
        } else {
            return Err(Error::conflict("发送方版本仅支持整批接收"));
        }
    } else {
        None
    };
    let streams_to_disk = transfer
        .files
        .iter()
        .any(|f| matches!(f.storage, FileStorage::Browser | FileStorage::Receiving));
    let saves_from_cache = transfer.files.iter().any(StoredFile::cached);
    if accept && owner && (streams_to_disk || saves_from_cache) {
        let explicit_directory = directory.is_some();
        let directory = match directory {
            Some(directory) => directory.to_owned(),
            None => app.target_directory().await?,
        };
        let files: Vec<_> = transfer
            .files
            .iter()
            .filter(|f| selected.as_ref().is_none_or(|ids| ids.contains(&f.meta.id)))
            .map(|f| &f.meta)
            .collect();
        let mut targets = app
            .plan_receive(&directory, &files, &active, &choices)
            .await?;
        let lease = app
            .0
            .space
            .reserve(
                transfer
                    .files
                    .iter()
                    .filter(|f| selected.as_ref().is_none_or(|ids| ids.contains(&f.meta.id)))
                    .filter(|f| {
                        !f.complete || (saves_from_cache && f.cached() && explicit_directory)
                    })
                    .map(|f| {
                        (
                            format!("receive:{id}:{}", f.meta.id),
                            directory.clone(),
                            f.meta.size,
                        )
                    })
                    .collect(),
            )
            .await
            .map_err(Error::conflict)?;
        for f in &mut transfer.files {
            f.receive_target = targets.remove(&f.meta.id);
        }
        if streams_to_disk {
            app.0
                .space_leases
                .lock()
                .unwrap()
                .insert(format!("incoming:{id}"), lease);
        }
        // Cached exports reserve again in the saver. Drop their preflight lease
        // before committing the queue so the saver cannot count them twice.
    }
    transfer.view.select_files(selected);
    transfer.view.status = if !accept {
        "rejected"
    } else if transfer.files.is_empty() {
        "completed"
    } else if !owner
        || transfer.incoming_remote
        || transfer
            .files
            .iter()
            .any(|f| f.storage == FileStorage::Browser)
    {
        "accepted"
    } else {
        "completed"
    }
    .into();
    if transfer.view.status == "completed" {
        transfer.view.bytes = transfer.view.total;
    }
    if accept {
        transfer.view.created = now();
    }

    transfer.view.error = None;
    Ok(())
}
async fn auto_receive(app: &App, state: &mut crate::model::State, id: &str) {
    let transfer = &state.transfers[id];
    if transfer.view.status != "pending" || !automatically_received(&transfer.view.files) {
        return;
    }
    let recipient = transfer.view.recipient_id.clone();
    let owner = recipient == app.0.device_id;
    let decision = ReceiveDecision {
        choices: Default::default(),
        accept: true,
        directory: None,
        file_ids: None,
    };
    if let Err(error) = set_decision(app, state, id, &recipient, owner, decision).await {
        // Leave the offer pending so the recipient can retry after fixing storage.
        state.transfers.get_mut(id).unwrap().view.error = Some(format!("自动接收未完成：{error}"));
    }
}
fn release_acceptance(app: &App, id: &str) {
    app.0
        .space_leases
        .lock()
        .unwrap()
        .remove(&format!("incoming:{id}"));
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
    let file_ids = serde_json::from_value(value["file_ids"].clone())
        .map_err(|_| Error::bad("文件标识无效"))?;
    app.decide_transfer(
        &id,
        &p.id,
        p.owner,
        ReceiveDecision {
            choices: serde_json::from_value(
                value.get("choices").cloned().unwrap_or_else(|| json!({})),
            )
            .map_err(|_| Error::bad("同名文件选项无效"))?,
            accept,
            directory: None,
            file_ids,
        },
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}

impl App {
    pub async fn accept_transfer_to_directory(&self, id: &str, directory: String) -> Result<()> {
        self.accept_selected_files_to_directory(id, directory, None)
            .await
    }

    pub async fn accept_selected_files_to_directory(
        &self,
        id: &str,
        directory: String,
        file_ids: Option<Vec<String>>,
    ) -> Result<()> {
        self.accept_files_with_choices(id, directory, file_ids, Default::default())
            .await
    }
    pub async fn accept_files_with_choices(
        &self,
        id: &str,
        directory: String,
        file_ids: Option<Vec<String>>,
        choices: ConflictChoices,
    ) -> Result<()> {
        if directory.is_empty() {
            return Err(Error::bad("请选择接收目录"));
        }
        self.decide_transfer(
            id,
            &self.0.device_id,
            true,
            ReceiveDecision {
                choices,
                accept: true,
                directory: Some(&directory),
                file_ids,
            },
        )
        .await
    }

    async fn decide_transfer(
        &self,
        id: &str,
        recipient: &str,
        owner: bool,
        decision: ReceiveDecision<'_>,
    ) -> Result<()> {
        self.cache_transfer(id).await?;
        let mut state = self.0.state.lock().await;
        let directory = decision.directory;
        set_decision(self, &mut state, id, recipient, owner, decision).await?;
        if let Err(error) = self
            .commit_with_receive_directory(
                &mut state,
                directory.map(|directory| (id.into(), directory.into())),
            )
            .await
        {
            release_acceptance(self, id);
            return Err(error);
        }
        drop(state);
        self.notify();
        Ok(())
    }
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
        .filter(|t| {
            (t.view.sender_id == p.id || t.view.recipient_id == p.id)
                && (p.owner
                    || (t.view.group_id.is_none()
                        && (t.view.sender_id == app.0.device_id
                            || t.view.recipient_id == app.0.device_id)))
        })
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
        .filter(|t| t.view.sender_id == p.id || t.view.recipient_id == p.id)
        .ok_or_else(Error::missing)?;
    if !p.owner
        && (transfer.view.group_id.is_some()
            || (transfer.view.sender_id != app.0.device_id
                && transfer.view.recipient_id != app.0.device_id))
    {
        return Err(Error::forbidden());
    }
    if transfer.view.recipient_id == p.id
        && (!transfer.view.includes_file(&file_id)
            || !matches!(
                transfer.view.status.as_str(),
                "accepted" | "transferring" | "completed"
            ))
    {
        return Err(Error::conflict("请先接收文件"));
    }
    let file = transfer
        .files
        .iter()
        .find(|f| f.meta.id == file_id && f.available())
        .ok_or_else(Error::missing)?
        .clone();
    let track = !p.owner
        && transfer.view.recipient_id == p.id
        && !transfer.view.terminal()
        && !file.downloaded;
    if state.tickets.len() >= 2048 {
        return Err(Error::conflict("下载请求过多，请稍后重试"));
    }
    let key = secret();
    state.tickets.insert(
        key.clone(),
        Ticket {
            file,
            transfer_id: id,
            track,
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
    let _permit = app
        .0
        .streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::conflict("同时下载过多，请稍后重试"))?;
    let publication = if matches!(ticket.file.storage, FileStorage::Received { .. }) {
        // A ticket issued for an earlier message may outlive an overwrite.
        let guard = app.0.publishing_replacement.lock().await;
        let id = ticket.transfer_id.clone();
        let file = ticket.file.meta.id.clone();
        let location = ticket.file.path.to_string_lossy().into_owned();
        let owns_file: bool = app.0.store.run(move |db, _| Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM saved_files s JOIN deliveries d ON d.message_id=s.message_id WHERE d.id=?1 AND s.file_id=?2 AND s.status='saved' AND s.location=?3)",
            rusqlite::params![id,file,location], |r| r.get(0))?)).await.map_err(|e| Error::storage(e.to_string()))?;
        if !owns_file {
            return Err(Error::missing());
        }
        Some(guard)
    } else {
        None
    };
    let input = app.open_source(&ticket.file).await.map_err(Error::bad)?;
    drop(publication);
    let length = ticket.file.meta.size;
    let name = ticket.file.meta.name.clone();
    if ticket.track {
        let mut state = app.0.state.lock().await;
        let t = state
            .transfers
            .get_mut(&ticket.transfer_id)
            .ok_or_else(Error::missing)?;
        if !matches!(t.view.status.as_str(), "accepted" | "transferring")
            || !t.view.includes_file(&ticket.file.meta.id)
        {
            return Err(Error::conflict("传输已结束"));
        }
        let f = t
            .files
            .iter_mut()
            .find(|f| f.meta.id == ticket.file.meta.id)
            .ok_or_else(Error::missing)?;
        if f.uploading || f.downloaded {
            return Err(Error::conflict("文件已下载或正在下载"));
        }
        f.uploading = true;
        t.view.status = "transferring".into();
        app.commit(&mut state).await?;
        drop(state);
        app.notify();
    }
    let verification = Some(input.try_clone().await?);
    let stream = futures_util::stream::try_unfold(
        DownloadStream {
            app: app.clone(),
            ticket,
            reader: Some(ReaderStream::new(input)),
            verification,
            bytes: 0,
            finished: false,
            updated: std::time::Instant::now(),
            _permit,
        },
        |mut download| async move {
            if download.finished {
                return Ok(None);
            }
            if download.ticket.track {
                let state = download.app.0.state.lock().await;
                if state
                    .transfers
                    .get(&download.ticket.transfer_id)
                    .is_none_or(|t| t.view.terminal())
                {
                    return Err(std::io::Error::other("传输已取消"));
                }
            }
            match download.reader.as_mut().unwrap().next().await {
                Some(Ok(chunk)) => {
                    download.bytes += chunk.len() as u64;
                    if download.bytes > download.ticket.file.meta.size {
                        return Err(std::io::Error::other("原文件大小已变化"));
                    }
                    if download.bytes == download.ticket.file.meta.size {
                        let input = download.verification.take().unwrap().into_std().await;
                        App::check_source(&download.ticket.file, &input)
                            .map_err(std::io::Error::other)?;
                        if download.ticket.track {
                            download.report(true).await.map_err(std::io::Error::other)?;
                        }
                        download.finish();
                    } else if download.ticket.track && download.updated.elapsed().as_millis() >= 200
                    {
                        download
                            .report(false)
                            .await
                            .map_err(std::io::Error::other)?;
                        download.updated = std::time::Instant::now();
                    }
                    Ok(Some((chunk, download)))
                }
                Some(Err(error)) => Err(error),
                None => {
                    if download.bytes != download.ticket.file.meta.size {
                        return Err(std::io::Error::other("文件不完整"));
                    }
                    let input = download.verification.take().unwrap().into_std().await;
                    App::check_source(&download.ticket.file, &input)
                        .map_err(std::io::Error::other)?;
                    if download.ticket.track {
                        download.report(true).await.map_err(std::io::Error::other)?;
                    }
                    download.finish();
                    Ok(None)
                }
            }
        },
    );
    attachment(Body::from_stream(stream), length, &name)
}

pub(crate) fn attachment(body: Body, length: u64, name: &str) -> Result<Response> {
    let encoded = url::form_urlencoded::byte_serialize(name.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    let mut response = body.into_response();
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
struct DownloadStream {
    app: App,
    ticket: Ticket,
    reader: Option<ReaderStream<tokio::fs::File>>,
    verification: Option<tokio::fs::File>,
    bytes: u64,
    finished: bool,
    updated: std::time::Instant,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl DownloadStream {
    fn finish(&mut self) {
        self.finished = true;
    }
    async fn report(&self, done: bool) -> std::result::Result<(), String> {
        let mut state = self.app.0.state.lock().await;
        let t = state
            .transfers
            .get_mut(&self.ticket.transfer_id)
            .ok_or("传输不存在")?;
        if t.view.terminal() {
            return Err("传输已结束".into());
        }
        if done {
            t.view.error = None;
            let file = t
                .files
                .iter_mut()
                .find(|f| f.meta.id == self.ticket.file.meta.id)
                .ok_or("文件不存在")?;
            file.downloaded = true;
            file.uploading = false;
        }
        t.view.created = now();
        if done {
            t.stream_bytes.remove(&self.ticket.file.meta.id);
        } else {
            t.stream_bytes
                .insert(self.ticket.file.meta.id.clone(), self.bytes);
        }
        t.refresh_stream_bytes(true);
        if t.files
            .iter()
            .all(|f| f.downloaded || !t.view.includes_file(&f.meta.id))
        {
            t.view.status = "completed".into();
        }
        if done {
            self.app
                .commit(&mut state)
                .await
                .map_err(|e| e.to_string())?;
        }
        drop(state);
        self.app.notify();
        Ok(())
    }
}
impl Drop for DownloadStream {
    fn drop(&mut self) {
        if !self.ticket.track || self.finished {
            return;
        }
        let app = self.app.clone();
        let id = self.ticket.transfer_id.clone();
        let file = self.ticket.file.meta.id.clone();
        tokio::spawn(async move {
            let mut state = app.0.state.lock().await;
            if let Some(t) = state.transfers.get_mut(&id) {
                if let Some(f) = t.files.iter_mut().find(|f| f.meta.id == file) {
                    f.uploading = false;
                }
                t.stream_bytes.remove(&file);
                if !t.view.terminal() {
                    t.view.status = if t.files.iter().any(|f| f.uploading) {
                        "transferring"
                    } else {
                        "accepted"
                    }
                    .into();
                    t.view.error = Some("下载中断，可重新下载；原文件需要保持可用".into());
                    t.refresh_stream_bytes(true);
                }
                let _ = app.commit(&mut state).await;
            }
            drop(state);
            app.notify();
        });
    }
}
async fn peer_offer(
    AxState(app): AxState<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
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
            source_id: None,
        })
        .collect();
    let total = validate_files(&input)?;
    let mut file_ids = std::collections::HashSet::new();
    if offer.files.iter().any(|f| {
        f.id.len() != 36
            || uuid::Uuid::parse_str(&f.id)
                .ok()
                .is_none_or(|id| id.to_string() != f.id)
            || !file_ids.insert(&f.id)
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
    if recipient != info.id {
        return Err(Error::forbidden());
    }
    let sender = if let Some(env) = &offer.identity {
        crate::identity::verify(env).map_err(|e| Error::bad(e.to_string()))?;
        if env.payload != crate::identity::offer_payload(&offer)
            || uuid::Uuid::parse_str(&offer.message_id).is_err()
        {
            return Err(Error::bad("消息签名与内容不符"));
        }
        if offer.sender_id != env.host_id {
            return Err(Error::forbidden());
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
    if sender.starts_with("browser-") {
        return Err(Error::forbidden());
    }
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
    if state
        .transfers
        .values()
        .filter(|t| !t.view.terminal() && t.incoming_remote)
        .count()
        >= 32
    {
        return Err(Error::conflict("接收队列已满"));
    }
    let id = uid();
    let token = secret();
    let files = offer
        .files
        .iter()
        .map(|f| StoredFile {
            receive_target: None,
            meta: f.clone(),
            path: Default::default(),
            complete: false,
            uploading: false,
            storage: FileStorage::Receiving,
            downloaded: false,
        })
        .collect();
    let recipient_name = state
        .profiles
        .get(&recipient)
        .map(|d| d.name.clone())
        .unwrap_or(info.name);
    let can_select_files = offer.files.len() > 1
        && headers
            .get(FILE_SELECTION_HEADER)
            .is_some_and(|value| value == "1");
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
        status: "pending".into(),
        can_select_files,
        selected_file_ids: None,
        bytes: 0,
        total,
        created: now(),
        error: None,
    };
    state.transfers.insert(
        id.clone(),
        Transfer {
            view,
            files,
            stream_bytes: Default::default(),
            peer_token: Some(token.clone()),
            incoming_remote: true,
            reserved: 0,
        },
    );
    auto_receive(&app, &mut state, &id).await;
    if let Err(error) = app.commit(&mut state).await {
        release_acceptance(&app, &id);
        return Err(error);
    }
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
        if !t.view.includes_file(&file_id) {
            return Err(Error::conflict("接收方未选择此文件"));
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
    let result = app.receive_stream(&id, &file, body).await;
    let mut state = app.0.state.lock().await;
    let t = authorized_transfer(&mut state, &id, token(&headers)?)?;
    let active = matches!(t.view.status.as_str(), "accepted" | "transferring");
    if let Some(f) = t.files.iter_mut().find(|f| f.meta.id == file_id) {
        if let Ok(saved) = &result {
            *f = saved.clone();
        }
        f.uploading = false;
    }
    if result.is_err() && active {
        t.view.status = "failed".into();
        t.view.error = result.as_ref().err().map(ToString::to_string);
    }
    t.stream_bytes.remove(&file_id);
    t.refresh_stream_bytes(false);
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    result?;
    if !active {
        return Err(Error::conflict("传输已取消"));
    }
    Ok(Json(json!({"ok":true})))
}
async fn browser_upload(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path((id, file_id)): Path<(String, String)>,
    body: Body,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    if p.owner {
        return Err(Error::forbidden());
    }
    let _permit = app
        .0
        .streams
        .try_acquire()
        .map_err(|_| Error::conflict("同时传输过多，请稍后重试"))?;
    let file = {
        let mut state = app.0.state.lock().await;
        let t = state
            .transfers
            .get_mut(&id)
            .filter(|t| {
                t.view.sender_id == p.id
                    && t.view.recipient_id == app.0.device_id
                    && t.view.group_id.is_none()
            })
            .ok_or_else(Error::missing)?;
        if !matches!(t.view.status.as_str(), "accepted" | "transferring") {
            return Err(Error::conflict("对方尚未接收或传输已结束"));
        }
        if !t.view.includes_file(&file_id) {
            return Err(Error::conflict("接收方未选择此文件"));
        }
        let f = t
            .files
            .iter_mut()
            .find(|f| f.meta.id == file_id)
            .ok_or_else(Error::missing)?;
        if f.storage != FileStorage::Browser || f.uploading || f.complete {
            return Err(Error::conflict("文件已上传或正在上传"));
        }
        f.uploading = true;
        t.view.status = "transferring".into();
        f.clone()
    };
    app.notify();
    let result = app.receive_stream(&id, &file, body).await;
    let mut state = app.0.state.lock().await;
    let t = state.transfers.get_mut(&id).ok_or_else(Error::missing)?;
    let active = !t.view.terminal();
    if let Some(f) = t.files.iter_mut().find(|f| f.meta.id == file_id) {
        if let Ok(saved) = &result {
            *f = saved.clone();
        }
        f.uploading = false;
    }
    if result.is_err() && active {
        t.view.status = "failed".into();
        t.view.error = result.as_ref().err().map(ToString::to_string);
    }
    if result.is_ok()
        && active
        && t.files
            .iter()
            .all(|f| f.complete || !t.view.includes_file(&f.meta.id))
    {
        t.view.status = "completed".into();
    }
    t.stream_bytes.remove(&file_id);
    t.refresh_stream_bytes(false);
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
        || t.files
            .iter()
            .any(|f| !f.complete && t.view.includes_file(&f.meta.id))
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
    Extension(BrowserTransport(secure)): Extension<BrowserTransport>,
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
        ) && origin != format!("{}://{host}", if secure { "https" } else { "http" })
        {
            return Err(Error::unauthorized());
        }
    }
    Ok(ws.on_upgrade(move |mut socket| async move {
        let first=tokio::time::timeout(std::time::Duration::from_secs(5),socket.recv()).await;
        let Ok(Some(Ok(axum::extract::ws::Message::Text(message))))=first else {return};
        let Ok(value)=serde_json::from_str::<Value>(&message) else{return};
        let Some(token)=value["token"].as_str() else{return};
        if !app.credential_matches_transport(token, secure).await || app.principal(token).await.is_err(){return;}
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
async fn scan_page() -> Response {
    let mut response = asset(axum::http::Uri::from_static("/")).await;
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

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
    for transfer in state
        .transfers
        .values_mut()
        .filter(|t| !t.view.terminal() && (t.view.sender_id == p.id || t.view.recipient_id == p.id))
    {
        transfer.view.status = "cancelled".into();
        transfer.view.error = Some("浏览器已断开连接".into());
    }
    app.commit(&mut state).await?;
    drop(state);
    app.disconnect_browser_viewer(&p.id).await;
    app.notify();
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod transfer_timeout_tests {
    use super::*;
    fn file(path: std::path::PathBuf, size: u64) -> StoredFile {
        StoredFile {
            receive_target: None,
            path,
            meta: FileMeta {
                id: "file".into(),
                name: "file".into(),
                size,
                mime: String::new(),
            },
            complete: false,
            uploading: true,
            storage: crate::model::FileStorage::Cache,
            downloaded: false,
        }
    }
    #[tokio::test(start_paused = true)]
    async fn progressing_upload_can_run_longer_than_one_hour() {
        let dir = tempfile::tempdir().unwrap();
        let target = file(dir.path().join("large"), 18);
        let body = Body::from_stream(futures_util::stream::unfold(0, |n| async move {
            if n == 18 {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_secs(240)).await;
            Some((Ok::<_, std::io::Error>(vec![b'x']), n + 1))
        }));
        let start = tokio::time::Instant::now();
        write_file(body, &target, None, None).await.unwrap();
        assert!(start.elapsed() > std::time::Duration::from_secs(3600));
        assert_eq!(std::fs::read(target.path).unwrap(), vec![b'x'; 18]);
    }
    #[tokio::test(start_paused = true)]
    async fn idle_upload_times_out_and_removes_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = file(dir.path().join("idle"), 1);
        let body = Body::from_stream(futures_util::stream::pending::<
            std::result::Result<Vec<u8>, std::io::Error>,
        >());
        let error = write_file(body, &target, None, None).await.unwrap_err();
        assert!(error.to_string().contains("5 分钟"));
        assert!(!target.path.exists());
        assert!(!target.path.with_extension("part").exists());
    }
}
