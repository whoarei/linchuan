use crate::{
    api::{token, Error, Result},
    model::*,
    App,
};
use axum::{
    extract::{Path, Query, State as AxState},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
pub async fn direct(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Conversation>> {
    let principal = app.principal(token(&headers)?).await?;
    let peer = body["device_id"]
        .as_str()
        .ok_or_else(|| Error::bad("请选择设备"))?;
    if !principal.owner && peer != app.0.device_id {
        return Err(Error::forbidden());
    }
    let mut state = app.0.state.lock().await;
    if state
        .profiles
        .get(peer)
        .is_some_and(|d| d.kind == "browser" && d.host_device_id != app.0.device_id)
    {
        return Err(Error::forbidden());
    }
    if peer == principal.id || !state.profiles.contains_key(peer) {
        return Err(Error::bad("设备不存在"));
    }
    let conversation = Conversation::direct(&principal.id, peer);
    state
        .conversations
        .entry(conversation.id.clone())
        .or_insert(conversation.clone());
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(conversation))
}
#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    before: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}
pub async fn timeline(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    {
        let state = app.0.state.lock().await;
        if state.conversations.get(&id).is_none_or(|c| {
            c.principal_id != p.id
                || (!p.owner
                    && (c.kind != "direct"
                        || c.peer_id.as_deref() != Some(app.0.device_id.as_str())))
        }) {
            return Err(Error::missing());
        }
    }
    let limit = page.limit.unwrap_or(50).clamp(1, 100);
    let mut messages = app
        .0
        .store
        .timeline(
            p.id.clone(),
            id.clone(),
            page.before.unwrap_or(i64::MAX as u64),
            limit,
        )
        .await
        .map_err(|e| Error::storage(e.to_string()))?;
    let state = app.0.state.lock().await;
    for message in &mut messages {
        for delivery in &mut message.deliveries {
            if let Some(t) = state.transfers.get(&delivery.id) {
                *delivery = t.view.clone();
            }
        }
    }
    let cursor = messages.first().map(|m| m.position);
    let draft = state.composition.get(&id).cloned().unwrap_or_default();
    Ok(Json(
        json!({"messages":messages,"next_before":if messages.len()==limit{cursor}else{None},"draft":draft}),
    ))
}
pub async fn read(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    {
        let state = app.0.state.lock().await;
        if state.conversations.get(&id).is_none_or(|c| {
            c.principal_id != p.id
                || (!p.owner
                    && (c.kind != "direct"
                        || c.peer_id.as_deref() != Some(app.0.device_id.as_str())))
        }) {
            return Err(Error::missing());
        }
    }
    let through = body["through_position"]
        .as_u64()
        .ok_or_else(|| Error::bad("缺少读取位置"))?;
    app.0
        .store
        .mark_read(p.id, id, through)
        .await
        .map_err(|e| Error::storage(e.to_string()))?;
    app.notify();
    Ok(Json(json!({"ok":true})))
}
pub async fn draft(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    let text = body["text"]
        .as_str()
        .ok_or_else(|| Error::bad("草稿格式错误"))?;
    if text.len() > 65536 {
        return Err(Error::bad("草稿超过 64 KiB"));
    }
    let mut state = app.0.state.lock().await;
    if state.conversations.get(&id).is_none_or(|c| {
        c.principal_id != p.id
            || (!p.owner
                && (c.kind != "direct" || c.peer_id.as_deref() != Some(app.0.device_id.as_str())))
    }) {
        return Err(Error::missing());
    }
    state.composition.insert(id, text.into());
    app.commit(&mut state).await?;
    Ok(Json(json!({"ok":true})))
}
pub async fn clear_files(AxState(app): AxState<App>, headers: HeaderMap) -> Result<Json<Value>> {
    if !app.principal(token(&headers)?).await?.owner {
        return Err(Error::unauthorized());
    }
    let mut state = app.0.state.lock().await;
    // Include archived history, while excluding every path used by an active task or staging draft.
    let active: std::collections::HashSet<_> = state
        .transfers
        .values()
        .filter(|t| !t.view.terminal())
        .flat_map(|t| t.files.iter().map(|f| f.path.clone()))
        .chain(
            state
                .drafts
                .values()
                .filter(|d| !d.sent)
                .flat_map(|d| d.files.iter().map(|f| f.path.clone())),
        )
        .collect();
    let paths: Vec<(String, u64)> = app
        .0
        .store
        .run(|c, _| {
            let rows = c
                .prepare("SELECT b.relative_path,b.size FROM file_blobs b WHERE b.state='available' AND NOT EXISTS(SELECT 1 FROM delivery_files df JOIN deliveries d ON d.id=df.delivery_id JOIN saved_files s ON s.message_id=d.message_id AND s.file_id=df.file_id WHERE df.blob_id=b.id AND s.status IN ('pending','failed'))")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        })
        .await
        .map_err(|e| Error::storage(e.to_string()))?;
    let mut freed = 0u64;
    let mut removed = Vec::new();
    for (path, size) in paths {
        let absolute = app.0.directory.join(&path);
        if active.contains(&absolute) {
            continue;
        }
        if absolute.is_file() {
            tokio::fs::remove_file(&absolute).await?;
            freed += size;
        }
        removed.push(path);
    }
    state.reserved = state.reserved.saturating_sub(freed);
    for t in state.transfers.values_mut() {
        if t.incoming_remote && t.view.terminal() {
            t.reserved = t
                .files
                .iter()
                .filter(|f| f.available())
                .map(|f| f.meta.size)
                .sum();
        }
    }
    app.commit(&mut state).await?;
    app.0
        .store
        .run(move |c, _| {
            let tx = c.transaction()?;
            for path in removed {
                tx.execute(
                    "UPDATE file_blobs SET state='expired' WHERE relative_path=?1",
                    [path],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| Error::storage(e.to_string()))?;
    drop(state);
    app.notify();
    Ok(Json(json!({"freed":freed})))
}
