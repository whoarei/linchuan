use linchuan_core::{App, Options};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::TempDir;
struct Host {
    app: App,
    dir: TempDir,
    url: String,
    token: String,
}
impl Host {
    async fn new(name: &str) -> Self {
        let dir = TempDir::new().unwrap();
        let mut options = Options::new(dir.path().into());
        options.name = Some(name.into());
        options.discovery = false;
        let app = App::start(options).await.unwrap();
        let b = app.bootstrap();
        Self {
            app,
            dir,
            url: b.address,
            token: b.token,
        }
    }
    async fn restart(&mut self) {
        self.app.stop();
        let mut options = Options::new(self.dir.path().into());
        options.discovery = false;
        self.app = App::start(options).await.unwrap();
        let b = self.app.bootstrap();
        self.url = b.address;
        self.token = b.token;
    }
    async fn get(&self, path: &str, token: &str) -> reqwest::Response {
        Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }
    async fn post_as(&self, path: &str, body: Value, token: &str) -> reqwest::Response {
        Client::new()
            .post(format!("{}{path}", self.url))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn post(&self, path: &str, body: Value) -> Value {
        let r = self.post_as(path, body, &self.token).await;
        let status = r.status();
        let value = r.json::<Value>().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{path}: {value}");
        value
    }
    async fn state(&self) -> Value {
        self.get("/api/state", &self.token)
            .await
            .json()
            .await
            .unwrap()
    }
    async fn connect(&self, other: &Host) -> String {
        self.post("/api/peers", json!({"address":other.url})).await["id"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn pair(&self, name: &str) -> String {
        self.post(
            "/api/pair",
            json!({"name":name,"code":self.app.bootstrap().pairing_code}),
        )
        .await["token"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn send(
        &self,
        target: Option<String>,
        group: Option<String>,
        text: &str,
        bytes: Option<&[u8]>,
    ) -> Value {
        let files = bytes
            .map(|b| json!([{"name":"持久文件.txt","size":b.len(),"mime":"text/plain"}]))
            .unwrap_or(json!([]));
        let draft = self.post("/api/drafts", json!({"files":files})).await;
        if let Some(b) = bytes {
            let r = Client::new()
                .put(format!(
                    "{}/api/drafts/{}/files/{}",
                    self.url,
                    draft["id"].as_str().unwrap(),
                    draft["files"][0]["id"].as_str().unwrap()
                ))
                .bearer_auth(&self.token)
                .body(b.to_vec())
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK);
        }
        self.post(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":target.into_iter().collect::<Vec<_>>(),"group_id":group,"text":text}),
        )
        .await
    }
    async fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..180 {
            let state = self.state().await;
            if predicate(&state) {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("State timeout: {}", self.state().await)
    }
    async fn timeline(&self, cid: &str, token: &str) -> Value {
        let response = self
            .get(&format!("/api/conversations/{cid}/messages"), token)
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.app.stop();
    }
}
fn cid(state: &Value, peer: &str) -> String {
    state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["peer_id"] == peer || c["group_id"] == peer)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .into()
}
async fn group(a: &Host, b: &Host, c: &Host) -> String {
    let b_id = a.connect(b).await;
    let c_id = a.connect(c).await;
    let group = a
        .post(
            "/api/groups",
            json!({"name":"家庭设备","members":[b_id,c_id]}),
        )
        .await["group"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    b.post(
        &format!("/api/groups/{group}/decision"),
        json!({"accept":true}),
    )
    .await;
    c.post(
        &format!("/api/groups/{group}/decision"),
        json!({"accept":true}),
    )
    .await;
    group
}
#[tokio::test]
async fn completed_files_history_and_drafts_survive_restart_and_missing_files_keep_cards() {
    let sender = Host::new("电脑").await;
    let mut receiver = Host::new("手机").await;
    let rid = sender.connect(&receiver).await;
    let sid = sender.app.info().await.id;
    let sent = sender
        .send(Some(rid), None, "跨重启历史", Some("原始字节".as_bytes()))
        .await;
    let message = sent["message_id"].clone();
    let state = receiver
        .wait(|s| {
            s["transfers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["message_id"] == message)
        })
        .await;
    let tid = state["transfers"][0]["id"].as_str().unwrap().to_owned();
    receiver
        .post(
            &format!("/api/transfers/{tid}/decision"),
            json!({"accept":true}),
        )
        .await;
    receiver
        .wait(|s| {
            s["transfers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["message_id"] == message && t["status"] == "completed")
        })
        .await;
    let conversation = cid(&receiver.state().await, &sid);
    receiver
        .post(
            &format!("/api/conversations/{conversation}/draft"),
            json!({"text":"没发完的草稿"}),
        )
        .await;
    let info = receiver.app.info().await;
    receiver.restart().await;
    assert_eq!(receiver.app.info().await.id, info.id);
    assert_eq!(receiver.app.info().await.public_key, info.public_key);
    assert!(receiver.state().await["devices"]
        .as_array()
        .unwrap()
        .is_empty());
    let page = receiver.timeline(&conversation, &receiver.token).await;
    assert_eq!(page["draft"], "没发完的草稿");
    assert_eq!(page["messages"][0]["text"], "跨重启历史");
    assert_eq!(page["messages"][0]["files"][0]["available"], true);
    let file = &page["messages"][0]["files"][0];
    let path = receiver
        .post(
            &format!(
                "/api/transfers/{}/files/{}/ticket",
                file["transfer_id"].as_str().unwrap(),
                file["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        Client::new()
            .get(format!("{}{path}", receiver.url))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "原始字节".as_bytes()
    );
    receiver.post("/api/storage/clear", json!({})).await;
    let page = receiver.timeline(&conversation, &receiver.token).await;
    assert_eq!(page["messages"][0]["files"][0]["available"], false);
    assert_eq!(page["messages"][0]["deliveries"][0]["status"], "completed");
    assert_eq!(receiver.state().await["storage_used"], 0);
}
#[tokio::test]
async fn interrupted_transfer_is_not_completed_after_restart() {
    let sender = Host::new("发送").await;
    let mut receiver = Host::new("接收").await;
    let target = sender.connect(&receiver).await;
    sender.send(Some(target), None, "尚未确认", None).await;
    receiver
        .wait(|s| !s["transfers"].as_array().unwrap().is_empty())
        .await;
    receiver.restart().await;
    assert_eq!(
        receiver.state().await["transfers"][0]["status"],
        "interrupted"
    );
}
#[tokio::test]
async fn migration_rename_credentials_and_logout_are_durable() {
    let dir = TempDir::new().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    tokio::fs::write(
        dir.path().join("device.json"),
        serde_json::to_vec(&json!({"id":id,"name":"旧名称"})).unwrap(),
    )
    .await
    .unwrap();
    let mut options = Options::new(dir.path().into());
    options.discovery = false;
    let app = App::start(options).await.unwrap();
    let mut host = Host {
        url: app.bootstrap().address,
        token: app.bootstrap().token,
        app,
        dir,
    };
    assert_eq!(host.app.info().await.id, id);
    host.post("/api/settings", json!({"name":"新的设备名"}))
        .await;
    assert_eq!(host.app.info().await.name, "新的设备名");
    let browser = host.pair("浏览器").await;
    let conn = rusqlite::Connection::open(host.dir.path().join("linchuan.db")).unwrap();
    let stored: String = conn
        .query_row("SELECT token_hash FROM browser_sessions", [], |r| r.get(0))
        .unwrap();
    assert_ne!(stored, browser);
    assert_eq!(stored.len(), 64);
    drop(conn);
    host.restart().await;
    assert_eq!(host.app.info().await.name, "新的设备名");
    assert_eq!(
        host.get("/api/state", &browser).await.status(),
        StatusCode::OK
    );
    let r = host
        .post_as("/api/session/logout", json!({}), &browser)
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    host.restart().await;
    assert_eq!(
        host.get("/api/state", &browser).await.status(),
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn default_native_name_is_saved_and_manual_name_takes_precedence() {
    let dir = TempDir::new().unwrap();
    let mut options = Options::new(dir.path().into());
    options.discovery = false;
    let app = App::start(options).await.unwrap();
    let mut host = Host {
        url: app.bootstrap().address,
        token: app.bootstrap().token,
        app,
        dir,
    };
    let original = host.app.info().await;
    assert!(!original.name.is_empty());
    assert_eq!(host.state().await["me"]["auto_name"], original.name);
    host.restart().await;
    assert_eq!(host.app.info().await.id, original.id);
    assert_eq!(host.app.info().await.name, original.name);
    host.post("/api/settings", json!({"name":"我的设备"})).await;
    host.restart().await;
    assert_eq!(host.app.info().await.name, "我的设备");
    assert_eq!(host.state().await["me"]["auto_name"], original.name);
}

#[tokio::test]
async fn automatic_browser_names_and_rename_preserve_identity_and_scope() {
    let mut host = Host::new("宿主电脑").await;
    let paired = Client::new()
        .post(format!("{}/api/pair", host.url))
        .header("user-agent", "Windows Chrome/131 Safari/537 Edg/131")
        .json(&json!({"code":host.app.bootstrap().pairing_code}))
        .send()
        .await
        .unwrap();
    assert_eq!(paired.status(), StatusCode::OK);
    let paired: Value = paired.json().await.unwrap();
    let token = paired["token"].as_str().unwrap();
    let id = paired["id"].as_str().unwrap();
    assert!(paired["name"]
        .as_str()
        .unwrap()
        .starts_with("Windows · Edge · "));
    let unnamed = host
        .post(
            "/api/pair",
            json!({"code":host.app.bootstrap().pairing_code,"name":"  "}),
        )
        .await;
    assert!(unnamed["name"].as_str().unwrap().starts_with("浏览器 · "));
    assert_ne!(unnamed["id"], paired["id"]);
    let other = host.pair(paired["name"].as_str().unwrap()).await;
    host.send(Some(id.into()), None, "改名前的历史", None).await;
    let conversation = cid(&host.state().await, id);
    let result = host
        .post_as(
            "/api/settings",
            json!({"name":"我的网页","id":host.app.info().await.id}),
            token,
        )
        .await;
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(host.app.info().await.name, "宿主电脑");
    let me: Value = host.get("/api/state", token).await.json().await.unwrap();
    assert_eq!(me["me"]["id"], id);
    assert_eq!(me["me"]["name"], "我的网页");
    assert!(me["me"]["auto_name"].is_null());
    let sibling: Value = host.get("/api/state", &other).await.json().await.unwrap();
    assert_eq!(sibling["me"]["name"], paired["name"]);
    assert_ne!(sibling["me"]["id"], id);
    let state = host.state().await;
    assert_eq!(cid(&state, id), conversation);
    assert_eq!(
        state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == conversation)
            .unwrap()["name"],
        "我的网页"
    );
    host.restart().await;
    let me: Value = host.get("/api/state", token).await.json().await.unwrap();
    assert_eq!(me["me"]["name"], "我的网页");
    assert_eq!(me["me"]["id"], id);
    assert_eq!(cid(&host.state().await, id), conversation);
    let history = host.timeline(&conversation, &host.token).await;
    assert_eq!(history["messages"].as_array().unwrap().len(), 1);
    assert_eq!(history["messages"][0]["text"], "改名前的历史");
    assert_eq!(host.app.info().await.name, "宿主电脑");
}

#[tokio::test]
async fn timeline_cursor_and_browser_scope_are_enforced() {
    let host = Host::new("电脑").await;
    let one = host.pair("手机一").await;
    let two = host.pair("手机二").await;
    let me: Value = host.get("/api/state", &one).await.json().await.unwrap();
    let id = me["me"]["id"].as_str().unwrap().to_owned();
    for i in 0..23 {
        host.send(Some(id.clone()), None, &format!("第{i}条"), None)
            .await;
    }
    let state: Value = host.get("/api/state", &one).await.json().await.unwrap();
    let conversation = cid(&state, &host.app.info().await.id);
    let mut cursor = None;
    let mut ids = std::collections::HashSet::new();
    loop {
        let path = format!(
            "/api/conversations/{conversation}/messages?limit=7{}",
            cursor
                .map(|v: u64| format!("&before={v}"))
                .unwrap_or_default()
        );
        let page: Value = host.get(&path, &one).await.json().await.unwrap();
        for message in page["messages"].as_array().unwrap() {
            assert!(ids.insert(message["id"].as_str().unwrap().to_owned()));
        }
        cursor = page["next_before"].as_u64();
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 23);
    assert_eq!(
        host.get(&format!("/api/conversations/{conversation}/messages"), &two)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        host.post_as(
            &format!("/api/conversations/{conversation}/draft"),
            json!({"text":"越权"}),
            &two
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn group_text_and_files_continue_without_creator_and_after_restart() {
    let a = Host::new("群主").await;
    let mut b = Host::new("电脑").await;
    let mut c = Host::new("手机").await;
    let group = group(&a, &b, &c).await;
    let sent = a.send(None, Some(group.clone()), "群文字", None).await;
    let mid = sent["message_id"].clone();
    c.wait(|s| {
        s["transfers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["message_id"] == mid && t["status"] == "completed")
    })
    .await;
    a.app.stop();
    let result = b
        .send(
            None,
            Some(group.clone()),
            "群主离线后发文件",
            Some(b"group file contents"),
        )
        .await;
    let message = result["message_id"].clone();
    let state = c
        .wait(|s| {
            s["transfers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["message_id"] == message)
        })
        .await;
    let delivery = state["transfers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["message_id"] == message)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    c.post(
        &format!("/api/transfers/{delivery}/decision"),
        json!({"accept":true}),
    )
    .await;
    let state = b
        .wait(|s| {
            let ds: Vec<_> = s["transfers"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["message_id"] == message)
                .collect();
            ds.len() == 2
                && ds.iter().any(|t| t["status"] == "completed")
                && ds.iter().any(|t| t["status"] == "failed")
        })
        .await;
    let conversation = cid(&state, &group);
    let page = b.timeline(&conversation, &b.token).await;
    assert_eq!(
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["id"] == message)
            .count(),
        1
    );
    assert_eq!(
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == message)
            .unwrap()["deliveries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    b.restart().await;
    c.restart().await;
    b.connect(&c).await;
    let result = b
        .send(None, Some(group.clone()), "重启后群仍存在", None)
        .await;
    let mid = result["message_id"].clone();
    c.wait(|s| {
        s["transfers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["message_id"] == mid && t["status"] == "completed")
    })
    .await;
    let page = c.timeline(&cid(&c.state().await, &group), &c.token).await;
    assert_eq!(
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["id"] == mid)
            .count(),
        1
    );
}
fn signed(host: &Host, payload: Value) -> Value {
    use ed25519_dalek::Signer;
    let conn = rusqlite::Connection::open(host.dir.path().join("linchuan.db")).unwrap();
    let secret: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key='identity_secret'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let key =
        ed25519_dalek::SigningKey::from_bytes(&hex::decode(secret).unwrap().try_into().unwrap());
    let host_id: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key='device_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let public_key = hex::encode(key.verifying_key().to_bytes());
    let bytes =
        serde_json::to_vec(&json!({"host_id":host_id,"public_key":public_key,"payload":payload}))
            .unwrap();
    json!({"host_id":host_id,"public_key":public_key,"payload":payload,"signature":hex::encode(key.sign(&bytes).to_bytes())})
}
#[tokio::test]
async fn forged_group_events_unsigned_messages_and_left_members_are_rejected() {
    let a = Host::new("群主").await;
    let b = Host::new("成员B").await;
    let c = Host::new("成员C").await;
    let group = group(&a, &b, &c).await;
    let state = a.state().await;
    let g = &state["groups"][0];
    let mut snapshot = g["snapshot"].clone();
    snapshot["payload"]["group"]["name"] = json!("篡改名称");
    assert_eq!(
        b.post_as("/api/peer/group", snapshot, &b.token)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let bid = b.app.info().await.id;
    let member = g["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["device"]["id"] == bid)
        .unwrap();
    let event = signed(
        &c,
        json!({"type":"member","protocol":1,"group_id":group,"member_id":bid,"invite_id":member["invite_id"],"state":"left","sequence":999}),
    );
    assert_eq!(
        a.post_as("/api/peer/group", event, &a.token).await.status(),
        StatusCode::BAD_REQUEST
    );
    let response = a
        .post_as(
            "/api/peer/offer",
            json!({"protocol":1,"sender_name":"冒用","text":"未签名","files":[],"group_id":group}),
            &a.token,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    c.post(&format!("/api/groups/{group}/leave"), json!({}))
        .await;
    let response = c
        .post_as("/api/drafts", json!({"files":[]}), &c.token)
        .await
        .json::<Value>()
        .await
        .unwrap();
    let response = c
        .post_as(
            &format!("/api/drafts/{}/send", response["id"].as_str().unwrap()),
            json!({"group_id":group,"text":"已退出仍试图发消息"}),
            &c.token,
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        a.state().await["groups"][0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["device"]["id"] == bid)
            .unwrap()["state"],
        "joined"
    );
}
#[tokio::test]
async fn database_write_failure_does_not_acknowledge_or_publish_phantom_message() {
    let host = Host::new("电脑").await;
    let browser = host.pair("手机").await;
    let s: Value = host.get("/api/state", &browser).await.json().await.unwrap();
    let id = s["me"]["id"].clone();
    let conn = rusqlite::Connection::open(host.dir.path().join("linchuan.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_messages BEFORE INSERT ON messages BEGIN SELECT RAISE(FAIL,'simulated disk failure'); END;").unwrap();
    let draft = host.post("/api/drafts", json!({"files":[]})).await;
    let response = host
        .post_as(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":[id],"text":"不能误报成功"}),
            &host.token,
        )
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(host.state().await["transfers"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn duplicate_signed_group_messages_do_not_duplicate_history_and_tampering_is_rejected() {
    let a = Host::new("群主").await;
    let b = Host::new("发送者").await;
    let mut c = Host::new("接收者").await;
    let gid = group(&a, &b, &c).await;
    let id = uuid::Uuid::new_v4().to_string();
    let created = 1_800_000_000_000u64;
    let sender = b.app.info().await.id;
    let target = c.app.info().await.id;
    let state = b.state().await;
    let member = state["groups"][0]["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["device"]["id"] == sender)
        .unwrap();
    let payload = json!({"type":"message","protocol":1,"message_id":id,"group_id":gid,"sender_id":sender,"target_id":target,"sender_name":"发送者","text":"仅保存一次","files":[],"created_at":created});
    let mut offer = payload.clone();
    offer["identity"] = signed(&b, payload);
    offer["membership"] = member["event"].clone();
    for _ in 0..2 {
        assert_eq!(
            c.post_as("/api/peer/offer", offer.clone(), &c.token)
                .await
                .status(),
            StatusCode::OK
        );
    }
    c.restart().await;
    assert_eq!(
        c.post_as("/api/peer/offer", offer.clone(), &c.token)
            .await
            .status(),
        StatusCode::OK
    );
    let conversation = cid(&c.state().await, &gid);
    let page = c.timeline(&conversation, &c.token).await;
    assert_eq!(
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["id"] == id)
            .count(),
        1
    );
    offer["text"] = json!("篡改正文");
    assert_eq!(
        c.post_as("/api/peer/offer", offer, &c.token).await.status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn failed_file_message_preserves_draft_budget_and_can_retry_after_database_recovers() {
    let host = Host::new("电脑").await;
    let browser = host.pair("手机").await;
    let s: Value = host.get("/api/state", &browser).await.json().await.unwrap();
    let target = s["me"]["id"].clone();
    let bytes = b"staged source survives failed commit";
    let draft = host
        .post(
            "/api/drafts",
            json!({"files":[{"name":"file.txt","size":bytes.len(),"mime":"text/plain"}]}),
        )
        .await;
    let draft_id = draft["id"].as_str().unwrap();
    let response = Client::new()
        .put(format!(
            "{}/api/drafts/{draft_id}/files/{}",
            host.url,
            draft["files"][0]["id"].as_str().unwrap()
        ))
        .bearer_auth(&host.token)
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let conn = rusqlite::Connection::open(host.dir.path().join("linchuan.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_messages BEFORE INSERT ON messages BEGIN SELECT RAISE(FAIL,'simulated disk failure'); END;").unwrap();
    let result = host
        .post_as(
            &format!("/api/drafts/{draft_id}/send"),
            json!({"targets":[target.clone()],"text":"失败后重试"}),
            &host.token,
        )
        .await;
    assert_eq!(result.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(host.state().await["storage_used"], bytes.len());
    conn.execute_batch("DROP TRIGGER reject_messages;").unwrap();
    host.post(
        &format!("/api/drafts/{draft_id}/send"),
        json!({"targets":[target],"text":"失败后重试"}),
    )
    .await;
    assert_eq!(host.state().await["transfers"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn shared_file_source_does_not_allow_another_browser_to_skip_acceptance() {
    let host = Host::new("电脑").await;
    let one = host.pair("一").await;
    let two = host.pair("二").await;
    let one_state: Value = host.get("/api/state", &one).await.json().await.unwrap();
    let two_state: Value = host.get("/api/state", &two).await.json().await.unwrap();
    let bytes = b"private attachment";
    let draft = host
        .post(
            "/api/drafts",
            json!({"files":[{"name":"private.txt","size":bytes.len(),"mime":"text/plain"}]}),
        )
        .await;
    let draftid = draft["id"].as_str().unwrap();
    let file = draft["files"][0]["id"].as_str().unwrap();
    assert_eq!(
        Client::new()
            .put(format!("{}/api/drafts/{draftid}/files/{file}", host.url))
            .bearer_auth(&host.token)
            .body(bytes.to_vec())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    host.post(
        &format!("/api/drafts/{draftid}/send"),
        json!({"targets":[one_state["me"]["id"],two_state["me"]["id"]],"text":"分别接收"}),
    )
    .await;
    let state: Value = host.get("/api/state", &one).await.json().await.unwrap();
    let t1 = state["transfers"][0]["id"].as_str().unwrap();
    assert_eq!(
        host.post_as(
            &format!("/api/transfers/{t1}/decision"),
            json!({"accept":true}),
            &one
        )
        .await
        .status(),
        StatusCode::OK
    );
    let state: Value = host.get("/api/state", &two).await.json().await.unwrap();
    let t2 = state["transfers"][0]["id"].as_str().unwrap();
    for tid in [t1, t2] {
        assert_eq!(
            host.post_as(
                &format!("/api/transfers/{tid}/files/{file}/ticket"),
                json!({}),
                &two
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
}
