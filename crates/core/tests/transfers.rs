use linchuan_core::{App, Options};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::TempDir;

struct Host {
    app: App,
    _dir: TempDir,
    url: String,
    token: String,
    code: String,
}

#[tokio::test]
async fn files_above_old_limits_use_available_space_and_reservations() {
    let host = Host::new("动态空间").await;
    host.app.set_space_probe(std::sync::Arc::new(|path| {
        Box::pin(async move {
            let mut disk = linchuan_core::storage::disk_space(std::path::Path::new(&path))?;
            disk.available = 32 * 1024 * 1024 * 1024;
            Ok(disk)
        })
    }));
    let response = host
        .post(
            "/api/drafts",
            json!({"files":[{"name":"eleven-gib.bin","size":11u64*1024*1024*1024}]}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let draft = response.json::<Value>().await.unwrap();
    let response = host
        .post(
            "/api/drafts",
            json!({"files":[{"name":"another.bin","size":20u64*1024*1024*1024}]}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(response.text().await.unwrap().contains("磁盘空间不足"));
    assert_eq!(
        host.post(
            &format!("/api/drafts/{}/cancel", draft["id"].as_str().unwrap()),
            json!({})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        host.post(
            "/api/drafts",
            json!({"files":[{"name":"after-cancel.bin","size":20u64*1024*1024*1024}]})
        )
        .await
        .status(),
        StatusCode::OK
    );
    host.app.stop();
}

#[tokio::test]
async fn files_larger_than_two_gib_stream_upload_and_download_intact() {
    use futures_util::StreamExt;
    use sha2::{Digest, Sha256};
    let host = Host::new("大文件流式验证").await;
    let recipient = host.pair("大文件接收浏览器").await;
    let size = 2u64 * 1024 * 1024 * 1024 + 17;
    let draft = host
        .draft(json!([{"name":"over-two-gib.bin","size":size}]))
        .await;
    let digest = std::sync::Arc::new(std::sync::Mutex::new(Sha256::new()));
    let expected = digest.clone();
    let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(
        (0u64, digest),
        move |(offset, digest)| async move {
            if offset == size {
                return None;
            }
            let count = (size - offset).min(1024 * 1024) as usize;
            let chunk = vec![(offset / (1024 * 1024) % 251) as u8; count];
            digest.lock().unwrap().update(&chunk);
            Some((
                Ok::<_, std::io::Error>(chunk),
                (offset + count as u64, digest),
            ))
        },
    ));
    let client = Client::new();
    let response = client
        .put(format!(
            "{}/api/drafts/{}/files/{}",
            host.url,
            draft["id"].as_str().unwrap(),
            draft["files"][0]["id"].as_str().unwrap()
        ))
        .bearer_auth(&host.token)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
    // A slow upload can outlast the browser's online window.
    let recipient_id = host.state(&recipient).await["me"]["id"].clone();
    let response = host
        .post(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":[recipient_id],"text":"大文件验证"}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let sent = response.json::<Value>().await.unwrap();
    let id = sent["ids"][0].as_str().unwrap();
    assert_eq!(
        client
            .post(format!("{}/api/transfers/{id}/decision", host.url))
            .bearer_auth(&recipient)
            .json(&json!({"accept":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let ticket: Value = client
        .post(format!(
            "{}/api/transfers/{id}/files/{}/ticket",
            host.url,
            draft["files"][0]["id"].as_str().unwrap()
        ))
        .bearer_auth(&recipient)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut stream = client
        .get(format!("{}{}", host.url, ticket["url"].as_str().unwrap()))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let mut actual = Sha256::new();
    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();
        downloaded += chunk.len() as u64;
        actual.update(&chunk);
    }
    assert_eq!(downloaded, size);
    assert_eq!(
        actual.finalize(),
        expected.lock().unwrap().clone().finalize()
    );
    host.app.stop();
    tokio::time::sleep(Duration::from_millis(30)).await;
}

#[tokio::test]
async fn updater_preflight_detects_incoming_transfer_and_file_preparation() {
    let host = Host::new("更新检查").await;
    assert!(!host.app.has_active_work().await);
    let offer = host.post("/api/peer/offer", json!({"protocol":1,"sender_name":"外部设备","text":"","files":[{"id":uuid::Uuid::new_v4().to_string(),"name":"file.txt","size":1,"mime":"text/plain"}]})).await.json::<Value>().await.unwrap();
    let id = offer["id"].as_str().unwrap();
    assert!(!host.app.has_active_work().await);
    assert_eq!(
        host.post(
            &format!("/api/transfers/{id}/decision"),
            json!({"accept":true})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(host.app.has_active_work().await);
    assert_eq!(
        host.post(&format!("/api/transfers/{id}/cancel"), json!({}))
            .await
            .status(),
        StatusCode::OK
    );
    assert!(!host.app.has_active_work().await);
    let draft = host
        .draft(json!([{"name":"draft.txt","size":2,"mime":"text/plain"}]))
        .await;
    assert!(
        !host.app.has_active_work().await,
        "An idle draft must not block updates"
    );
    let (sender, receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
    let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(
        receiver,
        |mut receiver| async {
            receiver
                .recv()
                .await
                .map(|chunk| (Ok::<_, std::io::Error>(chunk), receiver))
        },
    ));
    let url = format!(
        "{}/api/drafts/{}/files/{}",
        host.url,
        draft["id"].as_str().unwrap(),
        draft["files"][0]["id"].as_str().unwrap()
    );
    let token = host.token.clone();
    let upload = tokio::spawn(async move {
        Client::new()
            .put(url)
            .bearer_auth(token)
            .body(body)
            .send()
            .await
    });
    sender.send(vec![b'a']).await.unwrap();
    for _ in 0..100 {
        if host.app.has_active_work().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        host.app.has_active_work().await,
        "An active draft upload must block installation"
    );
    upload.abort();
    let _ = upload.await;
    drop(sender);
    for _ in 0..100 {
        if !host.app.has_active_work().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !host.app.has_active_work().await,
        "Cancelled uploads must not leave updates blocked"
    );
}
impl Host {
    async fn new(name: &str) -> Self {
        let dir = TempDir::new().unwrap();
        let mut opts = Options::new(dir.path().into());
        opts.name = Some(name.into());
        opts.discovery = false;
        let app = App::start(opts).await.unwrap();
        let b = app.bootstrap();
        Self {
            app,
            _dir: dir,
            url: b.address,
            token: b.token,
            code: b.pairing_code,
        }
    }
    async fn state(&self, token: &str) -> Value {
        Client::new()
            .get(format!("{}/api/state", self.url))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    async fn post(&self, path: &str, body: Value) -> reqwest::Response {
        Client::new()
            .post(format!("{}{path}", self.url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn pair(&self, name: &str) -> String {
        let response = self
            .post("/api/pair", json!({"name":name,"code":self.code}))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        response.json::<Value>().await.unwrap()["token"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn draft(&self, files: Value) -> Value {
        let r = self.post("/api/drafts", json!({"files":files})).await;
        assert_eq!(r.status(), StatusCode::OK);
        r.json().await.unwrap()
    }
    async fn offer_from(&self, sender: &Host, text: &str, files: Value) -> String {
        let response = sender.post("/api/peers", json!({"address":self.url})).await;
        assert_eq!(response.status(), StatusCode::OK);
        let target = response.json::<Value>().await.unwrap()["id"].clone();
        let draft = sender.draft(files).await;
        for file in draft["files"].as_array().unwrap() {
            let response = Client::new()
                .put(format!(
                    "{}/api/drafts/{}/files/{}",
                    sender.url,
                    draft["id"].as_str().unwrap(),
                    file["id"].as_str().unwrap()
                ))
                .bearer_auth(&sender.token)
                .body(vec![0u8; file["size"].as_u64().unwrap() as usize])
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let result = sender
            .post(
                &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
                json!({"targets":[target],"text":text}),
            )
            .await;
        assert_eq!(result.status(), StatusCode::OK);
        result.json::<Value>().await.unwrap()["ids"][0]
            .as_str()
            .unwrap()
            .into()
    }
    async fn wait_incoming(&self) -> Value {
        for _ in 0..100 {
            let s = self.state(&self.token).await;
            if let Some(t) = s["transfers"].as_array().unwrap().first() {
                return t.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("incoming request timed out")
    }
    async fn wait_status(&self, id: &str, status: &str) -> Value {
        for _ in 0..150 {
            let s = self.state(&self.token).await;
            if let Some(t) = s["transfers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["id"] == id)
            {
                if t["status"] == status {
                    return t.clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{status} timed out: {}", self.state(&self.token).await)
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.app.stop();
    }
}

#[tokio::test]
async fn random_ports_persistent_identity_and_no_public_credentials() {
    let a = Host::new("设备 A").await;
    let b = Host::new("设备 B").await;
    assert_ne!(a.url, b.url);
    let info: Value = Client::new()
        .get(format!("{}/api/info", a.url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(info.get("token").is_none());
    assert!(info.get("pairing_code").is_none());
    let mut opts = Options::new(a._dir.path().into());
    opts.discovery = false;
    let restarted = App::start(opts).await.unwrap();
    assert_eq!(info["id"], restarted.info().await.id);
    assert_ne!(a.token, restarted.bootstrap().token);
    restarted.stop();
}
#[tokio::test]
async fn browser_pairing_isolation_and_automatic_text_reception() {
    let host = Host::new("电脑").await;
    let client = Client::new();
    assert_eq!(
        client
            .get(format!("{}/api/state", host.url))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        host.post("/api/pair", json!({"name":"x","code":"bad"}))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let one = host.pair("手机一").await;
    let two = host.pair("手机二").await;
    let own_id = host.app.info().await.id;
    let draft: Value = client
        .post(format!("{}/api/drafts", host.url))
        .bearer_auth(&one)
        .json(&json!({"files":[]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let response = client
        .post(format!(
            "{}/api/drafts/{}/send",
            host.url,
            draft["id"].as_str().unwrap()
        ))
        .bearer_auth(&one)
        .json(&json!({"targets":[own_id],"text":"你好，邻传\nhttps://example.com"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let incoming = host.wait_incoming().await;
    let id = incoming["id"].as_str().unwrap();
    assert_eq!(incoming["status"], "completed");
    assert_eq!(
        host.state(&two).await["transfers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        client
            .post(format!("{}/api/transfers/{id}/decision", host.url))
            .bearer_auth(&two)
            .json(&json!({"accept":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        host.post(
            &format!("/api/transfers/{id}/decision"),
            json!({"accept":true})
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        host.state(&one).await["transfers"][0]["status"],
        "completed"
    );
    assert_eq!(
        host.post(
            &format!("/api/transfers/{id}/decision"),
            json!({"accept":false})
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn peer_text_is_automatic_and_ordinary_files_can_be_rejected_or_cancelled() {
    let sender = Host::new("Windows").await;
    let receiver = Host::new("Android").await;
    let id = receiver.offer_from(&sender, "文字直达", json!([])).await;
    let incoming = receiver.wait_incoming().await;
    let remote = incoming["id"].as_str().unwrap();
    assert_eq!(incoming["status"], "completed");
    sender.wait_status(&id, "completed").await;
    receiver.wait_status(remote, "completed").await;
    let other = Host::new("拒绝端").await;
    let id = other
        .offer_from(
            &sender,
            "拒绝这条",
            json!([{"name":"ordinary.bin","size":0}]),
        )
        .await;
    let incoming = other.wait_incoming().await;
    other
        .post(
            &format!(
                "/api/transfers/{}/decision",
                incoming["id"].as_str().unwrap()
            ),
            json!({"accept":false}),
        )
        .await;
    sender.wait_status(&id, "rejected").await;
    let cancel = Host::new("取消端").await;
    let id = cancel
        .offer_from(
            &sender,
            "取消这条",
            json!([{"name":"ordinary.bin","size":0}]),
        )
        .await;
    let incoming = cancel.wait_incoming().await;
    sender
        .post(&format!("/api/transfers/{id}/cancel"), json!({}))
        .await;
    cancel
        .wait_status(incoming["id"].as_str().unwrap(), "cancelled")
        .await;
}
#[tokio::test]
async fn multiple_files_unicode_names_binary_integrity_and_one_use_download() {
    let a = Host::new("发送方").await;
    let b = Host::new("接收方").await;
    let c = Client::new();
    let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let second = "这是原始文字文件。\n".as_bytes();
    let draft=a.draft(json!([{"name":"原图 数据.bin","size":data.len(),"mime":"application/octet-stream"},{"name":"说明.txt","size":second.len(),"mime":"text/plain"},{"name":"空文件.txt","size":0}])).await;
    for (i, bytes) in [data.as_slice(), second, &[]].iter().enumerate() {
        let response = c
            .put(format!(
                "{}/api/drafts/{}/files/{}",
                a.url,
                draft["id"].as_str().unwrap(),
                draft["files"][i]["id"].as_str().unwrap()
            ))
            .bearer_auth(&a.token)
            .body(bytes.to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    a.post("/api/peers", json!({"address":b.url})).await;
    let bid = b.app.info().await.id;
    let r = a
        .post(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":[bid],"text":"附件"}),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    let incoming = b.wait_incoming().await;
    let id = incoming["id"].as_str().unwrap();
    b.post(
        &format!("/api/transfers/{id}/decision"),
        json!({"accept":true}),
    )
    .await;
    b.wait_status(id, "completed").await;
    for (i, bytes) in [data.as_slice(), second, &[]].iter().enumerate() {
        let r = b
            .post(
                &format!(
                    "/api/transfers/{id}/files/{}/ticket",
                    draft["files"][i]["id"].as_str().unwrap()
                ),
                json!({}),
            )
            .await;
        assert_eq!(r.status(), StatusCode::OK);
        let path = r.json::<Value>().await.unwrap()["url"]
            .as_str()
            .unwrap()
            .to_owned();
        let response = c.get(format!("{}{path}", b.url)).send().await.unwrap();
        assert!(response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("UTF-8''"));
        assert_eq!(response.bytes().await.unwrap().as_ref(), *bytes);
        assert_eq!(
            c.get(format!("{}{path}", b.url))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}
#[tokio::test]
async fn unsafe_paths_limits_partial_upload_and_premature_completion() {
    let host = Host::new("测试").await;
    let id = "abcdef12-3456-4789-abcd-0123456789ab";
    for files in [
        json!([{"id":id.to_uppercase(),"name":"uppercase.txt","size":0,"mime":""}]),
        json!([{"id":id,"name":"first.txt","size":0,"mime":""},{"id":id.to_uppercase(),"name":"second.txt","size":0,"mime":""}]),
    ] {
        assert_eq!(
            host.post(
                "/api/peer/offer",
                json!({"protocol":1,"sender_name":"对端","text":"","files":files})
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    for name in ["../escape", "/etc/passwd", "a\\b", "C:escape", ".."] {
        assert_eq!(
            host.post("/api/drafts", json!({"files":[{"name":name,"size":1}]}))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        host.post(
            "/api/drafts",
            json!({"files":[{"name":"invalid-size.bin","size":u64::MAX}]})
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    for address in [
        "http://example.com:8080",
        "https://192.168.1.1:80",
        "http://8.8.8.8",
        "http://127.0.0.1/path",
        "http://u:p@127.0.0.1:80",
    ] {
        assert_eq!(
            host.post("/api/peers", json!({"address":address}))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let draft = host.draft(json!([{"name":"partial.bin","size":10}])).await;
    let c = Client::new();
    let url = format!(
        "{}/api/drafts/{}/files/{}",
        host.url,
        draft["id"].as_str().unwrap(),
        draft["files"][0]["id"].as_str().unwrap()
    );
    assert_eq!(
        c.put(url)
            .bearer_auth(&host.token)
            .body("short")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let session = host.pair("目标").await;
    let target = host.state(&session).await["me"]["id"].clone();
    assert_eq!(
        host.post(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":[target],"text":""})
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let offer=host.post("/api/peer/offer",json!({"protocol":1,"sender_name":"外部设备","text":"","files":[{"id":uuid::Uuid::new_v4().to_string(),"name":"file","size":1,"mime":""}]})).await.json::<Value>().await.unwrap();
    let id = offer["id"].as_str().unwrap();
    let t = offer["token"].as_str().unwrap();
    assert_eq!(
        c.post(format!("{}/api/peer/transfers/{id}/complete", host.url))
            .bearer_auth(t)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        c.get(format!("{}/api/peer/transfers/{id}", host.url))
            .bearer_auth("wrong")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    host.post(
        &format!("/api/transfers/{id}/decision"),
        json!({"accept":true}),
    )
    .await;
    assert_eq!(
        c.post(format!("{}/api/peer/transfers/{id}/complete", host.url))
            .bearer_auth(t)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn multiple_browser_recipients_have_independent_decisions() {
    let host = Host::new("桌面").await;
    let one = host.pair("一").await;
    let two = host.pair("二").await;
    let target1 = host.state(&one).await["me"]["id"].clone();
    let target2 = host.state(&two).await["me"]["id"].clone();
    let draft = host.draft(json!([{"name":"ordinary.bin","size":0}])).await;
    assert_eq!(
        Client::new()
            .put(format!(
                "{}/api/drafts/{}/files/{}",
                host.url,
                draft["id"].as_str().unwrap(),
                draft["files"][0]["id"].as_str().unwrap()
            ))
            .bearer_auth(&host.token)
            .body(Vec::<u8>::new())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let response = host
        .post(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":[target1,target2],"text":"多人发送"}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let state1 = host.state(&one).await;
    let state2 = host.state(&two).await;
    assert_eq!(state1["transfers"].as_array().unwrap().len(), 1);
    assert_eq!(state2["transfers"].as_array().unwrap().len(), 1);
    let c = Client::new();
    for (state, token, accept) in [(state1, one, true), (state2, two, false)] {
        let id = state["transfers"][0]["id"].as_str().unwrap();
        assert_eq!(
            c.post(format!("{}/api/transfers/{id}/decision", host.url))
                .bearer_auth(token)
                .json(&json!({"accept":accept}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    let state = host.state(&host.token).await;
    let statuses: Vec<_> = state["transfers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["status"].as_str().unwrap())
        .collect();
    assert!(statuses.contains(&"accepted") && statuses.contains(&"rejected"));
}
