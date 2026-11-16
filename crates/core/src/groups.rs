use crate::{
    api::{token, Error, Result},
    identity,
    model::*,
    App,
};
use axum::{
    extract::{ConnectInfo, Path, State as AxState},
    http::HeaderMap,
    Json,
};
use serde_json::{json, Value};
use std::{collections::HashSet, net::SocketAddr};
fn bad(e: impl std::fmt::Display) -> Error {
    Error::bad(e.to_string())
}
fn valid_endpoint(d: &Device) -> bool {
    let native = uuid::Uuid::parse_str(&d.id).is_ok() && d.host_device_id == d.id;
    native
        && hex::decode(&d.public_key).is_ok_and(|k| k.len() == 32)
        && crate::clean_name(&d.name).is_ok()
}
fn normalized(group: &Group) -> Group {
    let mut g = group.clone();
    g.snapshot = None;
    for member in &mut g.members {
        if matches!(member.device.kind.as_str(), "local" | "host") {
            member.device.kind = "app".into();
        }
        member.state = if member.device.id == g.owner_id {
            "joined"
        } else {
            "invited"
        }
        .into();
        member.sequence = 0;
        member.event = None;
    }
    g.members.sort_by(|a, b| a.device.id.cmp(&b.device.id));
    g
}
pub fn apply(state: &mut State, env: &SignedEnvelope, owner: &str) -> Result<()> {
    identity::verify(env).map_err(bad)?;
    if env.payload["protocol"] != 1 {
        return Err(Error::bad("群协议版本不兼容"));
    }
    match env.payload["type"].as_str() {
        Some("snapshot") => {
            let target = env.payload["target_id"]
                .as_str()
                .ok_or_else(|| Error::bad("缺少邀请目标"))?;
            if target != owner {
                return Err(Error::forbidden());
            }
            let mut group: Group =
                serde_json::from_value(env.payload["group"].clone()).map_err(bad)?;
            if uuid::Uuid::parse_str(&group.id).is_err()
                || crate::clean_name(&group.name).is_err()
                || group.revision == 0
                || group.members.len() > 16
                || group.members.len() < 2
            {
                return Err(Error::bad("群资料格式错误"));
            }
            let mut ids = HashSet::new();
            if group.members.iter().any(|m| {
                !ids.insert(m.device.id.clone())
                    || !valid_endpoint(&m.device)
                    || uuid::Uuid::parse_str(&m.invite_id).is_err()
                    || !matches!(m.role.as_str(), "owner" | "member")
            }) {
                return Err(Error::bad("群成员身份无效"));
            }
            let creator = group
                .members
                .iter()
                .find(|m| m.device.id == group.owner_id && m.role == "owner")
                .ok_or_else(|| Error::bad("缺少群主身份"))?;
            if creator.device.host_device_id != env.host_id
                || creator.device.public_key != env.public_key
                || group.members.iter().filter(|m| m.role == "owner").count() != 1
            {
                return Err(Error::bad("只有群主可以更新邀请名单"));
            }
            if !group.members.iter().any(|m| m.device.id == target) {
                return Err(Error::bad("目标不在邀请名单中"));
            }
            if let Some(old) = state.groups.get(&group.id) {
                let old_owner = old
                    .members
                    .iter()
                    .find(|m| m.device.id == old.owner_id)
                    .ok_or_else(|| Error::bad("群资料异常"))?;
                if old.owner_id != group.owner_id
                    || old_owner.device.public_key != env.public_key
                    || old_owner.device.host_device_id != env.host_id
                    || group.revision < old.revision
                {
                    return Err(Error::bad("群资料版本或身份不匹配"));
                }
                if group.revision == old.revision {
                    if old
                        .snapshot
                        .as_ref()
                        .map(|s| s.payload["group"].clone())
                        .unwrap_or(serde_json::to_value(normalized(old)).map_err(bad)?)
                        != serde_json::to_value(normalized(&group)).map_err(bad)?
                    {
                        return Err(Error::bad("同一版本的群资料发生冲突"));
                    }
                    return Ok(());
                }
                // A roster update cannot force a joined member to leave or accept on their behalf.
                for member in &mut group.members {
                    if let Some(previous) = old.members.iter().find(|m| {
                        m.device.id == member.device.id && m.invite_id == member.invite_id
                    }) {
                        member.state = previous.state.clone();
                        member.sequence = previous.sequence;
                        member.event = previous.event.clone();
                    } else {
                        member.state = if member.device.id == group.owner_id {
                            "joined"
                        } else {
                            "invited"
                        }
                        .into();
                        member.sequence = 0;
                        member.event = None;
                    }
                }
                if old
                    .members
                    .iter()
                    .any(|m| !group.members.iter().any(|n| n.device.id == m.device.id))
                {
                    return Err(Error::bad("不能通过名单更新移除既有成员"));
                }
            } else {
                if state.groups.len() >= 128 {
                    return Err(Error::conflict("群数量已达上限"));
                }
                for member in &mut group.members {
                    member.state = if member.device.id == group.owner_id {
                        "joined"
                    } else {
                        "invited"
                    }
                    .into();
                    member.sequence = 0;
                    member.event = None;
                }
            }
            for member in &group.members {
                if let Some(old) = state.profiles.get(&member.device.id) {
                    if !old.public_key.is_empty()
                        && (old.public_key != member.device.public_key
                            || old.host_device_id != member.device.host_device_id)
                    {
                        return Err(Error::bad("邀请中的设备身份与已知设备不符"));
                    }
                }
            }
            for member in &group.members {
                state
                    .profiles
                    .entry(member.device.id.clone())
                    .or_insert(member.device.clone());
                if member.device.id == owner
                    || state.sessions.values().any(|s| s.id == member.device.id)
                {
                    let c = Conversation::group(&member.device.id, &group.id);
                    state.conversations.entry(c.id.clone()).or_insert(c);
                }
            }
            group.snapshot = Some(env.clone());
            state.groups.insert(group.id.clone(), group);
            Ok(())
        }
        Some("member") => {
            let id = env.payload["group_id"]
                .as_str()
                .ok_or_else(|| Error::bad("缺少群 ID"))?;
            let group = state.groups.get_mut(id).ok_or_else(Error::missing)?;
            let member = group
                .members
                .iter_mut()
                .find(|m| Some(m.device.id.as_str()) == env.payload["member_id"].as_str())
                .ok_or_else(Error::missing)?;
            let sequence = env.payload["sequence"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| Error::bad("成员事件序号无效"))?;
            let status = env.payload["state"]
                .as_str()
                .filter(|s| matches!(*s, "joined" | "left"))
                .ok_or_else(|| Error::bad("成员状态无效"))?;
            if member.device.host_device_id != env.host_id
                || member.device.public_key != env.public_key
                || Some(member.invite_id.as_str()) != env.payload["invite_id"].as_str()
            {
                return Err(Error::bad("无权修改该成员状态"));
            }
            if sequence < member.sequence {
                return Err(Error::bad("成员事件已过期"));
            }
            if sequence == member.sequence {
                if member.state != status {
                    return Err(Error::bad("成员事件冲突"));
                }
                return Ok(());
            }
            if member.state == "left" && status == "joined" {
                return Err(Error::bad("退出后需要新的邀请才能加入"));
            }
            member.sequence = sequence;
            member.state = status.into();
            member.event = Some(env.clone());
            Ok(())
        }
        _ => Err(Error::bad("未知群事件")),
    }
}
pub async fn receive(
    AxState(app): AxState<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(env): Json<SignedEnvelope>,
) -> Result<Json<Value>> {
    let mut state = app.0.state.lock().await;
    crate::api::rate_limit(&mut state, format!("group:{}", addr.ip()), 60)?;
    apply(&mut state, &env, &app.0.device_id)?;
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    Ok(Json(json!({"ok":true})))
}
pub async fn resolve(app: &App, device: &Device) -> Result<Option<String>> {
    if device.host_device_id == app.0.device_id {
        return Ok(None);
    }
    let address = {
        let state = app.0.state.lock().await;
        state
            .peers
            .get(&device.host_device_id)
            .map(|d| d.address.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| device.address.clone())
    };
    let address = crate::api::local_address(&address)?;
    let response = app
        .0
        .client
        .get(format!("{address}/api/info"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map_err(|_| Error::bad("设备离线或连接失败"))?;
    if !response.status().is_success() || response.content_length().unwrap_or(0) > 65536 {
        return Err(Error::bad("设备信息无效"));
    }
    let info: Value = response.json().await.map_err(bad)?;
    if info["id"] != device.host_device_id || info["public_key"] != device.public_key {
        return Err(Error::bad("目标设备身份不匹配"));
    }
    Ok(Some(address))
}
async fn transmit(app: &App, device: &Device, env: &SignedEnvelope) -> Result<()> {
    if let Some(address) = resolve(app, device).await? {
        let response = app
            .0
            .client
            .post(format!("{address}/api/peer/group"))
            .json(env)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| Error::bad("群通知发送失败"))?;
        if !response.status().is_success() {
            return Err(Error::bad("对方拒绝了群通知，请确认版本和邀请状态"));
        }
    }
    Ok(())
}
async fn broadcast_snapshot(app: &App, group: &Group) -> Vec<String> {
    let mut tasks = Vec::new();
    for member in &group.members {
        if member.device.host_device_id == app.0.device_id {
            continue;
        }
        let app = app.clone();
        let device = member.device.clone();
        let group = normalized(group);
        tasks.push(async move {
            let env = app
                .signed(json!({"type":"snapshot","protocol":1,"target_id":device.id,"group":group}))
                .await;
            let result = transmit(&app, &device, &env).await;
            (device.name, result)
        });
    }
    futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|(name, r)| r.err().map(|_| name))
        .collect()
}
async fn broadcast_member(app: &App, group: &Group, env: &SignedEnvelope) -> Vec<String> {
    let mut hosts = HashSet::new();
    let mut tasks = Vec::new();
    for member in &group.members {
        if member.device.host_device_id == app.0.device_id
            || !hosts.insert(member.device.host_device_id.clone())
        {
            continue;
        }
        let app = app.clone();
        let device = member.device.clone();
        let env = env.clone();
        tasks.push(async move {
            let result = transmit(&app, &device, &env).await;
            (device.name, result)
        });
    }
    futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|(name, r)| r.err().map(|_| name))
        .collect()
}
pub async fn create(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    if !p.owner {
        return Err(Error::forbidden());
    }
    let name = crate::clean_name(body["name"].as_str().unwrap_or("")).map_err(bad)?;
    let selected: Vec<String> = serde_json::from_value(body["members"].clone()).map_err(bad)?;
    let mut ids: HashSet<String> = selected.into_iter().collect();
    ids.insert(p.id.clone());
    if ids.len() < 2 || ids.len() > 16 {
        return Err(Error::bad("群需有 2–16 位成员"));
    }
    let mut state = app.0.state.lock().await;
    let mut members = Vec::new();
    for id in ids {
        let d = state
            .profiles
            .get(&id)
            .ok_or_else(|| Error::bad("设备不存在"))?
            .clone();
        if !valid_endpoint(&d) || !d.capabilities.iter().any(|c| c == "groups-v1") {
            return Err(Error::bad(format!("{} 需要更新后才能加入群聊", d.name)));
        }
        members.push(GroupMember {
            role: if id == p.id { "owner" } else { "member" }.into(),
            state: if id == p.id { "joined" } else { "invited" }.into(),
            device: d,
            invite_id: uid(),
            sequence: 0,
            event: None,
        });
    }
    members.sort_by(|a, b| a.device.id.cmp(&b.device.id));
    let mut group = Group {
        id: uid(),
        name,
        owner_id: p.id.clone(),
        revision: 1,
        created_at: now_ms(),
        snapshot: None,
        members,
    };
    let env = app
        .signed(json!({"type":"snapshot","protocol":1,"target_id":p.id,"group":normalized(&group)}))
        .await;
    apply(&mut state, &env, &app.0.device_id)?;
    group = state.groups[&group.id].clone();
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    let failed = broadcast_snapshot(&app, &group).await;
    Ok(Json(json!({"group":group,"unreachable":failed})))
}
pub async fn decide(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    if !p.owner {
        return Err(Error::forbidden());
    }
    let accept = body["accept"]
        .as_bool()
        .ok_or_else(|| Error::bad("缺少加入决定"))?;
    let mut state = app.0.state.lock().await;
    let group = state.groups.get(&id).ok_or_else(Error::missing)?;
    let m = group
        .members
        .iter()
        .find(|m| m.device.id == p.id)
        .ok_or_else(Error::missing)?;
    if m.state != "invited" {
        return Err(Error::conflict("邀请已处理"));
    }
    let env=app.signed(json!({"type":"member","protocol":1,"group_id":id,"member_id":p.id,"invite_id":m.invite_id,"state":if accept{"joined"}else{"left"},"sequence":m.sequence+1})).await;
    apply(&mut state, &env, &app.0.device_id)?;
    let group = state.groups[&id].clone();
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    let failed = broadcast_member(&app, &group, &env).await;
    Ok(Json(json!({"ok":true,"unreachable":failed})))
}
pub async fn leave(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    if !p.owner {
        return Err(Error::forbidden());
    }
    let mut state = app.0.state.lock().await;
    let group = state.groups.get(&id).ok_or_else(Error::missing)?;
    let m = group
        .members
        .iter()
        .find(|m| m.device.id == p.id && m.state == "joined")
        .ok_or_else(Error::missing)?;
    let env=app.signed(json!({"type":"member","protocol":1,"group_id":id,"member_id":p.id,"invite_id":m.invite_id,"state":"left","sequence":m.sequence+1})).await;
    apply(&mut state, &env, &app.0.device_id)?;
    let group = state.groups[&id].clone();
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    let failed = broadcast_member(&app, &group, &env).await;
    Ok(Json(json!({"ok":true,"unreachable":failed})))
}
pub async fn invite(
    AxState(app): AxState<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = app.principal(token(&headers)?).await?;
    if !p.owner {
        return Err(Error::forbidden());
    }
    let ids: Vec<String> = serde_json::from_value(body["members"].clone()).map_err(bad)?;
    let name = body["name"]
        .as_str()
        .map(crate::clean_name)
        .transpose()
        .map_err(bad)?;
    let mut state = app.0.state.lock().await;
    let mut group = state
        .groups
        .get(&id)
        .filter(|g| {
            g.owner_id == p.id
                && g.members
                    .iter()
                    .any(|m| m.device.id == p.id && m.state == "joined")
        })
        .ok_or_else(Error::missing)?
        .clone();
    for id in ids {
        let device = state
            .profiles
            .get(&id)
            .filter(|d| valid_endpoint(d) && d.capabilities.iter().any(|c| c == "groups-v1"))
            .ok_or_else(|| Error::bad("设备不存在或不支持群聊"))?
            .clone();
        if let Some(old) = group.members.iter_mut().find(|m| m.device.id == id) {
            if old.state == "left" {
                old.invite_id = uid();
                old.state = "invited".into();
                old.sequence = 0;
                old.event = None;
            }
        } else {
            group.members.push(GroupMember {
                device,
                invite_id: uid(),
                role: "member".into(),
                state: "invited".into(),
                sequence: 0,
                event: None,
            });
        }
    }
    if group.members.len() > 16 {
        return Err(Error::bad("群最多 16 位成员"));
    }
    if let Some(name) = name {
        group.name = name;
    }
    group.revision += 1;
    let env = app
        .signed(json!({"type":"snapshot","protocol":1,"target_id":p.id,"group":normalized(&group)}))
        .await;
    apply(&mut state, &env, &app.0.device_id)?;
    let group = state.groups[&id].clone();
    app.commit(&mut state).await?;
    drop(state);
    app.notify();
    let failed = broadcast_snapshot(&app, &group).await;
    Ok(Json(json!({"ok":true,"unreachable":failed})))
}
