use linchuan_core::{App, Options};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::TempDir;

struct Host {
    app: App,
    directory: TempDir,
}
impl Host {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut options = Options::new(directory.path().into());
        options.discovery = false;
        Self {
            app: App::start(options).await.unwrap(),
            directory,
        }
    }
    async fn get(&self, path: &str, token: &str) -> reqwest::Response {
        Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("{}{path}", self.app.bootstrap().address))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }
    async fn post(&self, path: &str, body: Value, token: &str) -> reqwest::Response {
        Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("{}{path}", self.app.bootstrap().address))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.app.stop();
    }
}

#[tokio::test]
async fn http_random_ports_work_without_certificates_and_preserve_protocol_one() {
    let host = Host::new().await;
    let other = Host::new().await;
    let info = host
        .get("/api/info", "")
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(info["protocol"], 1);
    assert!(info["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .all(|value| value.as_str().unwrap().starts_with("http://")));
    assert!(!info["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|capability| capability == "https-v1"));
    assert!(info.get("certificate_fingerprint").is_none());
    assert!(info.get("token").is_none() && info.get("pairing_code").is_none());
    assert_ne!(host.app.bootstrap().address, other.app.bootstrap().address);
    assert!(serde_json::to_value(host.app.bootstrap())
        .unwrap()
        .get("https_address")
        .is_none());
    assert_eq!(
        host.get("/api/certificate", "").await.status(),
        StatusCode::NOT_FOUND
    );
    let connection = host
        .post(
            "/api/peers",
            json!({"address":other.app.bootstrap().address}),
            &host.app.bootstrap().token,
        )
        .await;
    assert_eq!(connection.status(), StatusCode::OK);
    assert_eq!(
        connection.json::<Value>().await.unwrap()["id"],
        other.app.info().await.id
    );
    let database = rusqlite::Connection::open(host.directory.path().join("linchuan.db")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT count(*) FROM app_settings WHERE key='tls_identity'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn an_old_ca_setting_does_not_block_http_or_erase_identity_and_browser_history() {
    let mut host = Host::new().await;
    let before = host.app.info().await;
    let owner = host.app.bootstrap().token;
    assert!(host
        .post("/api/settings", json!({"name":"回退后保留名称"}), &owner)
        .await
        .status()
        .is_success());
    let paired = host
        .post(
            "/api/pair",
            json!({"name":"历史浏览器","code":host.app.bootstrap().pairing_code}),
            "",
        )
        .await
        .json::<Value>()
        .await
        .unwrap();
    let token = paired["token"].as_str().unwrap();
    let draft = host
        .post("/api/drafts", json!({"files":[]}), token)
        .await
        .json::<Value>()
        .await
        .unwrap();
    let sent = host
        .post(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":[before.id],"text":"HTTPS 版本留下的历史继续保留"}),
            token,
        )
        .await;
    assert_eq!(sent.status(), StatusCode::OK);
    let state = host
        .get("/api/state", &owner)
        .await
        .json::<Value>()
        .await
        .unwrap();
    let transfer = state["transfers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["text"] == "HTTPS 版本留下的历史继续保留")
        .unwrap();
    assert_eq!(transfer["status"], "completed");
    host.app.stop();
    {
        let database =
            rusqlite::Connection::open(host.directory.path().join("linchuan.db")).unwrap();
        database.execute("INSERT INTO app_settings(key,value) VALUES('tls_identity','unreadable legacy certificate material')", []).unwrap();
    }
    let mut options = Options::new(host.directory.path().into());
    options.discovery = false;
    host.app = App::start(options).await.unwrap();
    let after = host.app.info().await;
    assert_eq!(after.id, before.id);
    assert_eq!(after.public_key, before.public_key);
    assert_eq!(after.name, "回退后保留名称");
    assert_eq!(after.protocol, 1);
    let state = host
        .get("/api/state", token)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(state["me"]["name"], "历史浏览器");
    let conversation = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["peer_id"] == before.id)
        .unwrap();
    let history = host
        .get(
            &format!(
                "/api/conversations/{}/messages",
                conversation["id"].as_str().unwrap()
            ),
            token,
        )
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert!(history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["text"] == "HTTPS 版本留下的历史继续保留"));
    let database = rusqlite::Connection::open(host.directory.path().join("linchuan.db")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT value FROM app_settings WHERE key='tls_identity'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "unreadable legacy certificate material"
    );
    assert_eq!(
        database
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
}

#[tokio::test]
async fn websocket_origin_and_manual_address_checks_follow_http() {
    let host = Host::new().await;
    let bootstrap = host.app.bootstrap();
    let client = Client::builder().no_proxy().build().unwrap();
    for (origin, status) in [
        (bootstrap.address.clone(), StatusCode::SWITCHING_PROTOCOLS),
        (
            bootstrap.address.replace("http://", "https://"),
            StatusCode::UNAUTHORIZED,
        ),
        ("http://unrelated.example".into(), StatusCode::UNAUTHORIZED),
    ] {
        let response = client
            .get(format!("{}/api/events", bootstrap.address))
            .header("origin", origin)
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    let response = host
        .post(
            "/api/peers",
            json!({"address":bootstrap.address.replace("http://", "https://")}),
            &bootstrap.token,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(client
        .get(bootstrap.address.replace("http://", "https://") + "/api/info")
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_err());
}
