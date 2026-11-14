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

pub struct Error(StatusCode, String);
impl Error {
    pub fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "连接已失效，请重新配对".into())
    }
    fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "内容不存在或已过期".into())
    }
    fn conflict(message: impl Into<String>) -> Self {
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
fn token(headers: &HeaderMap) -> Result<&str> {
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
        .route("/api/state", get(state))
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
        token.clone(),
        Session {
            id: id.clone(),
            name,
            last_seen: now(),
        },
    );
    drop(state);
    app.notify();
    Ok(Json(json!({"token": token, "id": id})))
}
fn rate_limit(state: &mut crate::model::State, key: String, limit: u32) -> Result<()> {
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
    Ok(Json(
        json!({"me": {"id":p.id,"name":p.name,"owner":p.owner}, "info":info,"devices":devices,"transfers":transfers,"storage_used":state.reserved,"storage_limit":MAX_STORAGE}),
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
    let mut config = app.0.config.lock().await;
    config.name = name;
    tokio::fs::write(
        app.0.directory.join("device.json"),
        serde_json::to_vec_pretty(&*config).unwrap(),
    )
    .await?;
    drop(config);
    app.notify();
    Ok(Json(
        json!({"ok":true,"note":"新名称在下次启动时用于设备发现"}),
    ))
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
    };
    let mut state = app.0.state.lock().await;
    if state.peers.len() >= 128 && !state.peers.contains_key(&id) {
        return Err(Error::conflict("设备列表已满"));
    }
    state.peers.insert(id.clone(), device);
    drop(state);
    app.notify();
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
async fn write_file(body: Body, file: &StoredFile) -> Result<()> {
    let part = file.path.with_extension("part");
    let result = async {
        let mut output = tokio::fs::File::create(&part).await?;
        let mut stream = body.into_data_stream();
        let mut count = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Error::bad("上传中断，请重新发送"))?;
            count += chunk.len() as u64;
            if count > file.meta.size {
                return Err(Error::bad("上传内容超过声明的大小"));
            }
            output.write_all(&chunk).await?;
        }
        if count != file.meta.size {
            return Err(Error::bad("文件不完整，请重新发送"));
        }
        output.flush().await?;
        drop(output);
        tokio::fs::rename(&part, &file.path).await?;
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
        write_file(body, &file),
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
    if request.text.len() > 65536 || request.targets.is_empty() || request.targets.len() > 8 {
        return Err(Error::bad("文字最多 64 KiB，接收设备需为 1–8 台"));
    }
    let info = app.info().await;
    let mut workers = Vec::new();
    let mut ids = Vec::new();
    let mut state = app.0.state.lock().await;
    let draft = state
        .drafts
        .get(&id)
        .filter(|d| d.owner == p.id)
        .ok_or_else(Error::missing)?
        .clone();
    if draft.sent || draft.files.iter().any(|f| !f.complete) {
        return Err(Error::conflict("文件尚未上传完成，或草稿已发送"));
    }
    if draft.files.is_empty() && request.text.trim().is_empty() {
        return Err(Error::bad("请添加文字或文件"));
    }
    if state.transfers.len() >= 512 {
        return Err(Error::conflict("传输记录已满，请清理或重启应用"));
    }
    let mut targets = request.targets;
    targets.sort();
    targets.dedup();
    let mut resolved = Vec::new();
    for target in targets {
        if target == p.id {
            return Err(Error::bad("不能发给当前会话"));
        }
        if target == info.id {
            resolved.push((target, info.name.clone(), None));
        } else if let Some(session) = state
            .sessions
            .values()
            .find(|s| s.id == target && now().saturating_sub(s.last_seen) < 45)
        {
            resolved.push((target, session.name.clone(), None));
        } else if let Some(peer) = state
            .peers
            .get(&target)
            .filter(|d| now().saturating_sub(d.last_seen) < 90)
        {
            resolved.push((target, peer.name.clone(), Some(peer.address.clone())));
        } else {
            return Err(Error::bad("接收设备已离线，请刷新后重试"));
        }
    }
    for (target, name, address) in resolved {
        let id = uid();
        let view = TransferView {
            id: id.clone(),
            sender_id: p.id.clone(),
            sender_name: p.name.clone(),
            recipient_id: target,
            recipient_name: name,
            text: request.text.clone(),
            files: draft.files.iter().map(|f| f.meta.clone()).collect(),
            status: "pending".into(),
            bytes: 0,
            total: draft.files.iter().map(|f| f.meta.size).sum(),
            created: now(),
            error: None,
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
        if let Some(address) = address {
            workers.push((id.clone(), address));
        }
        ids.push(id);
    }
    state.drafts.get_mut(&draft.id).unwrap().sent = true;
    drop(state);
    app.notify();
    for (id, address) in workers {
        let app = app.clone();
        tokio::spawn(async move {
            crate::transport::send(app, id, address).await;
        });
    }
    Ok(Json(json!({"ids":ids})))
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
    let mut state = app.0.state.lock().await;
    let transfer = state
        .transfers
        .get(&id)
        .filter(|t| {
            (t.view.recipient_id == p.id || t.view.sender_id == p.id)
                && t.view.status == "completed"
        })
        .ok_or_else(Error::missing)?;
    let file = transfer
        .files
        .iter()
        .find(|f| f.meta.id == file_id && f.complete)
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
    Json(offer): Json<Offer>,
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
    if offer
        .files
        .iter()
        .any(|f| uuid::Uuid::parse_str(&f.id).is_err() || !file_ids.insert(&f.id))
    {
        return Err(Error::bad("文件标识无效"));
    }
    let info = app.info().await;
    let id = uid();
    let token = secret();
    let mut state = app.0.state.lock().await;
    rate_limit(&mut state, format!("offer:{}", addr.ip()), 30)?;
    if state.reserved + total > MAX_STORAGE
        || state.transfers.len() >= 512
        || state
            .transfers
            .values()
            .filter(|t| !t.view.terminal() && t.incoming_remote)
            .count()
            >= 32
    {
        return Err(Error::conflict("接收队列或临时空间已满"));
    }
    tokio::fs::create_dir(app.0.work.join(&id)).await?;
    let files = offer
        .files
        .iter()
        .map(|f| StoredFile {
            meta: f.clone(),
            path: app.0.work.join(&id).join(&f.id),
            complete: false,
            uploading: false,
        })
        .collect();
    let view = TransferView {
        id: id.clone(),
        sender_id: format!("remote-{}", uid()),
        sender_name: offer.sender_name,
        recipient_id: info.id,
        recipient_name: info.name,
        text: offer.text,
        files: offer.files,
        status: "pending".into(),
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
        write_file(body, &file),
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
