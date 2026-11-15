use linchuan_core::{App, Options};
use reqwest::{Certificate, Client, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
    async fn restart(&mut self) {
        self.app.stop();
        let mut options = Options::new(self.directory.path().into());
        options.discovery = false;
        self.app = App::start(options).await.unwrap();
    }
    async fn certificate(&self) -> String {
        Client::new()
            .get(format!("{}/api/certificate", self.app.bootstrap().address))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    }
    async fn client(&self) -> Client {
        Client::builder()
            .add_root_certificate(
                Certificate::from_pem(self.certificate().await.as_bytes()).unwrap(),
            )
            .no_proxy()
            .build()
            .unwrap()
    }
    async fn post(&self, path: &str, value: Value) -> Value {
        let bootstrap = self.app.bootstrap();
        let response = Client::new()
            .post(format!("{}{path}", bootstrap.address))
            .bearer_auth(bootstrap.token)
            .json(&value)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        body
    }
    async fn state(&self) -> Value {
        let bootstrap = self.app.bootstrap();
        Client::new()
            .get(format!("{}/api/state", bootstrap.address))
            .bearer_auth(bootstrap.token)
            .send()
            .await
            .unwrap()
            .json()
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
async fn tls_requires_the_right_ca_and_never_serves_plaintext_on_the_lan_port() {
    let host = Host::new().await;
    let other = Host::new().await;
    let bootstrap = host.app.bootstrap();
    assert!(bootstrap.https_address.starts_with("https://"));
    let info = host.app.info().await;
    assert_eq!(info.protocol, 2);
    assert!(info
        .addresses
        .iter()
        .all(|address| address.starts_with("https://")));
    let certificate = host.certificate().await;
    assert!(certificate.starts_with("-----BEGIN CERTIFICATE-----"));
    assert!(!certificate.contains("PRIVATE KEY"));
    let (_, pem) = x509_parser::pem::parse_x509_pem(certificate.as_bytes()).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&pem.contents)),
        info.certificate_fingerprint
    );

    let url = format!("{}/api/info", bootstrap.https_address);
    assert!(Client::new().get(&url).send().await.is_err());
    assert!(other.client().await.get(&url).send().await.is_err());
    let response = host.client().await.get(&url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let public: Value = response.json().await.unwrap();
    assert_eq!(public["id"], info.id);
    assert!(public.get("token").is_none() && public.get("pairing_code").is_none());
    assert!(!public.to_string().contains("PRIVATE KEY"));
    assert!(Client::new()
        .get(url.replace("https://", "http://"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_err());

    let bridge_port = url::Url::parse(&bootstrap.address).unwrap().port().unwrap();
    let mut network_bridge = url::Url::parse(&info.addresses[0]).unwrap();
    if !network_bridge.host_str().unwrap().starts_with("127.") {
        network_bridge.set_scheme("http").unwrap();
        network_bridge.set_port(Some(bridge_port)).unwrap();
        assert!(Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(network_bridge.join("api/info").unwrap())
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_err());
    }
}

#[tokio::test]
async fn the_same_ca_survives_rename_restart_and_new_random_ports() {
    let mut host = Host::new().await;
    let old_info = host.app.info().await;
    let certificate = host.certificate().await;
    let client = host.client().await;
    host.post("/api/settings", json!({"name":"重启后仍可信"}))
        .await;
    host.restart().await;
    let info = host.app.info().await;
    assert_eq!(info.name, "重启后仍可信");
    assert_eq!(info.id, old_info.id);
    assert_eq!(info.public_key, old_info.public_key);
    assert_eq!(
        info.certificate_fingerprint,
        old_info.certificate_fingerprint
    );
    assert_eq!(host.certificate().await, certificate);
    let response = client
        .get(format!("{}/api/info", host.app.bootstrap().https_address))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn changed_peer_identity_cannot_replace_a_known_device_after_restart() {
    let mut sender = Host::new().await;
    let receiver = Host::new().await;
    let impostor = Host::new().await;
    let target = sender
        .post(
            "/api/peers",
            json!({"address":receiver.app.bootstrap().https_address}),
        )
        .await["id"]
        .clone();
    sender.app.stop();
    {
        let database =
            rusqlite::Connection::open(sender.directory.path().join("linchuan.db")).unwrap();
        database
            .execute(
                "UPDATE devices SET last_address=?1 WHERE id=?2",
                rusqlite::params![
                    impostor.app.bootstrap().https_address,
                    target.as_str().unwrap()
                ],
            )
            .unwrap();
    }
    sender.restart().await;
    let bootstrap = sender.app.bootstrap();
    let response = Client::new()
        .post(format!("{}/api/peers", bootstrap.address))
        .bearer_auth(bootstrap.token)
        .json(&json!({"address":impostor.app.bootstrap().https_address}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(impostor.state().await["transfers"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn websocket_origins_follow_the_actual_transport_and_http_peers_are_rejected() {
    let host = Host::new().await;
    let bootstrap = host.app.bootstrap();
    let secure = host.client().await;
    for origin in [
        bootstrap.https_address.replace("https://", "http://"),
        "https://unrelated.example".into(),
    ] {
        let response = secure
            .get(format!("{}/api/events", bootstrap.https_address))
            .header("origin", origin)
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = secure
        .get(format!("{}/api/events", bootstrap.https_address))
        .header("origin", &bootstrap.https_address)
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let response = Client::new()
        .post(format!("{}/api/peers", bootstrap.address))
        .bearer_auth(bootstrap.token)
        .json(&json!({"address":"http://192.168.1.20:52783"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
