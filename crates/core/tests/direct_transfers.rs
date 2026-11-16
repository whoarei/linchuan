use linchuan_core::{App, Options};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
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
        let boot = app.bootstrap();
        Self {
            app,
            dir,
            url: boot.address,
            token: boot.token,
        }
    }
    async fn post(&self, path: &str, body: Value, token: &str) -> reqwest::Response {
        Client::new()
            .post(format!("{}{path}", self.url))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
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
    async fn pair(&self) -> (String, String) {
        let response: Value = self
            .post(
                "/api/pair",
                json!({"code":self.app.bootstrap().pairing_code}),
                &self.token,
            )
            .await
            .json()
            .await
            .unwrap();
        (
            response["token"].as_str().unwrap().into(),
            response["id"].as_str().unwrap().into(),
        )
    }
    async fn connect(&self, other: &Self) -> String {
        let response = self
            .post("/api/peers", json!({"address":other.url}), &self.token)
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        response.json::<Value>().await.unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn wait(&self, message: &str, status: &str) -> Value {
        for _ in 0..600 {
            let state = self.state(&self.token).await;
            if let Some(t) = state["transfers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["message_id"] == message && t["status"] == status)
            {
                return t.clone();
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("expected {status}: {}", self.state(&self.token).await);
    }
    async fn accept(&self, id: &str, token: &str) {
        let response = self
            .post(
                &format!("/api/transfers/{id}/decision"),
                json!({"accept":true}),
                token,
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{}",
            response.text().await.unwrap()
        );
    }
    async fn source_draft(&self, path: &Path) -> Value {
        let source = self
            .app
            .register_file(
                path.to_string_lossy().into_owned(),
                path.file_name().unwrap().to_string_lossy().into_owned(),
                String::new(),
                None,
            )
            .await
            .unwrap();
        let response=self.post("/api/drafts",json!({"files":[{"name":source.name,"size":source.size,"mime":source.mime,"source_id":source.source_id}]}),&self.token).await;
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }
    async fn send(&self, draft: &Value, targets: Value, group: Option<&str>, token: &str) -> Value {
        let response = self
            .post(
                &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
                json!({"targets":targets,"group_id":group,"text":"直接传输"}),
                token,
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{}",
            response.text().await.unwrap()
        );
        // The successful response remains unconsumed above.
        response.json().await.unwrap()
    }
    async fn download(&self, transfer: &str, file: &str, token: &str) -> reqwest::Response {
        let response = self
            .post(
                &format!("/api/transfers/{transfer}/files/{file}/ticket"),
                json!({}),
                token,
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let ticket: Value = response.json().await.unwrap();
        Client::new()
            .get(format!("{}{}", self.url, ticket["url"].as_str().unwrap()))
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
async fn published_receive_survives_delivery_commit_failure_and_restart_without_copying_again() {
    let mut host = Host::new("恢复接收").await;
    let folder = TempDir::new().unwrap();
    host.app
        .configure_receiving(folder.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    let (token, _) = host.pair().await;
    let draft: Value = host
        .post(
            "/api/drafts",
            json!({"streaming":true,"files":[{"name":"recovered.txt","size":4}]}),
            &token,
        )
        .await
        .json()
        .await
        .unwrap();
    let sent = host
        .send(&draft, json!([host.app.info().await.id]), None, &token)
        .await;
    let id = sent["ids"][0].as_str().unwrap();
    host.accept(id, &host.token).await;
    let database = rusqlite::Connection::open(host.dir.path().join("linchuan.db")).unwrap();
    database.execute_batch("CREATE TRIGGER fail_delivery BEFORE UPDATE ON deliveries WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    let response = Client::new()
        .put(format!(
            "{}/api/transfers/{id}/files/{}/upload",
            host.url,
            draft["files"][0]["id"].as_str().unwrap()
        ))
        .bearer_auth(token)
        .body("safe")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1);
    database
        .execute_batch("DROP TRIGGER fail_delivery;")
        .unwrap();
    drop(database);
    host.app.stop();
    let mut options = Options::new(host.dir.path().into());
    options.discovery = false;
    host.app = App::start(options).await.unwrap();
    let boot = host.app.bootstrap();
    host.url = boot.address;
    host.token = boot.token;
    let saved = host
        .app
        .saved_files(vec![sent["message_id"].as_str().unwrap().into()])
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(std::fs::read(&saved[0].location).unwrap(), b"safe");
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1);
    host.wait(sent["message_id"].as_str().unwrap(), "interrupted")
        .await;
}

#[tokio::test]
async fn uri_sources_and_destination_descriptors_stream_without_path_based_copies() {
    use linchuan_core::files::{PlatformFiles, PlatformOutput};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let sender = Host::new("URI 发送").await;
    let receiver = Host::new("URI 接收").await;
    let folder = TempDir::new().unwrap();
    let source = folder.path().join("source.bin");
    let target = folder.path().join("target.bin");
    let data = vec![97; 256 * 1024 + 19];
    std::fs::write(&source, &data).unwrap();
    let source_path = source.clone();
    let target_open = target.clone();
    let target_create = target.clone();
    let complete = Arc::new(AtomicUsize::new(0));
    let finished = complete.clone();
    sender.app.set_platform_files(PlatformFiles {
        open: Arc::new(move |_| {
            let path = source_path.clone();
            Box::pin(async move { std::fs::File::open(path).map_err(|e| e.to_string()) })
        }),
        begin: Arc::new(|_, _, _| Box::pin(async { Err("unused".into()) })),
        finish: Arc::new(|_| Box::pin(async { Err("unused".into()) })),
        abort: Arc::new(|_| Box::pin(async { Ok(()) })),
    });
    receiver.app.set_platform_files(PlatformFiles {
        open: Arc::new(move |_| {
            let path = target_open.clone();
            Box::pin(async move { std::fs::File::open(path).map_err(|e| e.to_string()) })
        }),
        begin: Arc::new(move |_, _, _| {
            let path = target_create.clone();
            Box::pin(async move {
                Ok(PlatformOutput {
                    file: std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|e| e.to_string())?,
                    location: "content://test/received".into(),
                })
            })
        }),
        finish: Arc::new(move |_| {
            let count = finished.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok("target.bin".into())
            })
        }),
        abort: Arc::new(|_| Box::pin(async { Ok(()) })),
    });
    let disk = folder.path().to_path_buf();
    receiver.app.set_space_probe(Arc::new(move |_| {
        let disk = disk.clone();
        Box::pin(async move { linchuan_core::storage::disk_space(&disk) })
    }));
    receiver
        .app
        .configure_receiving("@downloads".into())
        .await
        .unwrap();
    let source = sender
        .app
        .register_file(
            "content://test/original".into(),
            "source.bin".into(),
            "application/octet-stream".into(),
            Some(data.len() as u64),
        )
        .await
        .unwrap();
    let draft: Value = sender
        .post(
            "/api/drafts",
            json!({"files":[{"name":source.name,"size":source.size,"source_id":source.source_id}]}),
            &sender.token,
        )
        .await
        .json()
        .await
        .unwrap();
    let target_id = sender.connect(&receiver).await;
    let sent = sender
        .send(&draft, json!([target_id]), None, &sender.token)
        .await;
    let message = sent["message_id"].as_str().unwrap();
    let pending = receiver.wait(message, "pending").await;
    receiver
        .accept(pending["id"].as_str().unwrap(), &receiver.token)
        .await;
    receiver.wait(message, "completed").await;
    assert_eq!(std::fs::read(target).unwrap(), data);
    assert_eq!(complete.load(Ordering::SeqCst), 1);
    assert_eq!(bytes_in(&sender.dir.path().join("files")), 0);
    assert_eq!(bytes_in(&receiver.dir.path().join("files")), 0);
    let saved = receiver
        .app
        .saved_files(vec![message.into()])
        .await
        .unwrap();
    assert_eq!(saved[0].location, "content://test/received");
}
fn bytes_in(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() {
                bytes_in(&path)
            } else {
                path.metadata().unwrap().len()
            }
        })
        .sum()
}

#[tokio::test]
async fn browser_scope_is_host_only_in_all_control_apis() {
    let host = Host::new("宿主").await;
    let other = Host::new("其他客户端").await;
    let peer = host.connect(&other).await;
    let (token, id) = host.pair().await;
    let (_, second) = host.pair().await;
    let state = host.state(&token).await;
    assert!(state["devices"].as_array().unwrap().is_empty());
    assert!(state["groups"].as_array().unwrap().is_empty());
    for (path, body) in [
        ("/api/peers", json!({"address":other.url})),
        ("/api/conversations/direct", json!({"device_id":peer})),
        ("/api/conversations/direct", json!({"device_id":second})),
        ("/api/groups", json!({"name":"网页群","members":[peer]})),
    ] {
        assert_eq!(
            host.post(path, body, &token).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    let draft: Value = host
        .post("/api/drafts", json!({"streaming":true}), &token)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        host.post(
            &format!("/api/drafts/{}/send", draft["id"].as_str().unwrap()),
            json!({"targets":[peer],"text":"跨客户端"}),
            &token
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(host.post("/api/peer/offer",json!({"protocol":1,"target_id":id,"sender_name":"旧客户端","text":"远端投递","files":[]}),&other.token).await.status(),StatusCode::FORBIDDEN);
    let owner = host.state(&host.token).await;
    assert_eq!(
        host.post(
            "/api/groups",
            json!({"name":"禁止浏览器成员","members":[id,owner["me"]["id"]]}),
            &host.token
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn browser_upload_waits_for_acceptance_and_writes_one_copy_to_the_frozen_target() {
    let host = Host::new("宿主").await;
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    host.app
        .configure_receiving(first.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    let (token, _) = host.pair().await;
    let data = vec![61u8; 1024 * 1024 + 17];
    let draft: Value = host
        .post(
            "/api/drafts",
            json!({"streaming":true,"files":[{"name":"image.img","size":data.len()}]}),
            &token,
        )
        .await
        .json()
        .await
        .unwrap();
    let sent = host
        .send(&draft, json!([host.app.info().await.id]), None, &token)
        .await;
    let id = sent["ids"][0].as_str().unwrap();
    let file = draft["files"][0]["id"].as_str().unwrap();
    let upload_url = format!("{}/api/transfers/{id}/files/{file}/upload", host.url);
    assert_eq!(
        Client::new()
            .put(&upload_url)
            .bearer_auth(&token)
            .body(Vec::<u8>::new())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(bytes_in(&host.dir.path().join("files")), 0);
    assert_eq!(host.state(&host.token).await["storage_used"], 0);
    host.accept(id, &host.token).await;
    host.app
        .set_receive_directory(second.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    assert_eq!(
        Client::new()
            .put(&upload_url)
            .bearer_auth(&token)
            .body(data.clone())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    host.wait(sent["message_id"].as_str().unwrap(), "completed")
        .await;
    let saved = host
        .app
        .saved_files(vec![sent["message_id"].as_str().unwrap().into()])
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert!(Path::new(&saved[0].location).starts_with(first.path()));
    assert_eq!(std::fs::read(&saved[0].location).unwrap(), data);
    assert_eq!(std::fs::read_dir(first.path()).unwrap().count(), 1);
    assert_eq!(bytes_in(second.path()), 0);
    assert_eq!(bytes_in(&host.dir.path().join("files")), 0);
    host.post("/api/storage/clear", json!({}), &host.token)
        .await;
    assert!(Path::new(&saved[0].location).is_file());
}

#[tokio::test]
async fn native_original_downloads_to_browser_without_cache_and_acceptance_is_not_completion() {
    let host = Host::new("宿主").await;
    let source = TempDir::new().unwrap();
    let path = source.path().join("original.bin");
    let data = vec![29u8; 1024 * 1024 + 3];
    std::fs::write(&path, &data).unwrap();
    let (token, browser) = host.pair().await;
    let draft = host.source_draft(&path).await;
    let sent = host.send(&draft, json!([browser]), None, &host.token).await;
    let id = sent["ids"][0].as_str().unwrap();
    let file = draft["files"][0]["id"].as_str().unwrap();
    host.accept(id, &token).await;
    host.wait(sent["message_id"].as_str().unwrap(), "accepted")
        .await;
    assert_eq!(host.state(&host.token).await["storage_used"], 0);
    let response = host.download(id, file, &token).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await.unwrap().as_ref(), data);
    host.wait(sent["message_id"].as_str().unwrap(), "completed")
        .await;
    assert_eq!(bytes_in(&host.dir.path().join("files")), 0);
    host.post("/api/storage/clear", json!({}), &host.token)
        .await;
    assert!(path.is_file());
    // A later history download must reject changed content under the old attachment identity.
    std::fs::write(&path, b"changed").unwrap();
    let response = host.download(id, file, &token).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.text().await.unwrap().contains("变化"));
}

#[tokio::test]
async fn native_group_reads_one_original_independently_and_saves_one_copy_per_recipient() {
    let sender = Host::new("发送").await;
    let one = Host::new("接收一").await;
    let two = Host::new("接收二").await;
    let one_id = sender.connect(&one).await;
    let two_id = sender.connect(&two).await;
    let group: Value = sender
        .post(
            "/api/groups",
            json!({"name":"原生群","members":[one_id,two_id]}),
            &sender.token,
        )
        .await
        .json()
        .await
        .unwrap();
    let group = group["group"]["id"].as_str().unwrap();
    for receiver in [&one, &two] {
        assert_eq!(
            receiver
                .post(
                    &format!("/api/groups/{group}/decision"),
                    json!({"accept":true}),
                    &receiver.token
                )
                .await
                .status(),
            StatusCode::OK
        );
    }
    let source = TempDir::new().unwrap();
    let path = source.path().join("shared.bin");
    let data = vec![43u8; 1024 * 1024 + 9];
    std::fs::write(&path, &data).unwrap();
    let draft = sender.source_draft(&path).await;
    let sent = sender
        .send(&draft, json!([]), Some(group), &sender.token)
        .await;
    let message = sent["message_id"].as_str().unwrap();
    let pending = one.wait(message, "pending").await;
    one.accept(pending["id"].as_str().unwrap(), &one.token)
        .await;
    one.wait(message, "completed").await;
    two.wait(message, "pending").await;
    let pending = two.wait(message, "pending").await;
    two.accept(pending["id"].as_str().unwrap(), &two.token)
        .await;
    two.wait(message, "completed").await;
    for receiver in [&one, &two] {
        let saved = receiver
            .app
            .saved_files(vec![message.into()])
            .await
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(std::fs::read(&saved[0].location).unwrap(), data);
        assert_eq!(bytes_in(&receiver.dir.path().join("files")), 0);
        assert_eq!(receiver.state(&receiver.token).await["storage_used"], 0);
    }
    assert_eq!(bytes_in(&sender.dir.path().join("files")), 0);
    assert!(path.is_file());
}

#[tokio::test]
async fn cancelled_browser_upload_removes_partial_target_and_preserves_existing_user_file() {
    let host = Host::new("宿主").await;
    let folder = TempDir::new().unwrap();
    let existing = folder.path().join("file.bin");
    std::fs::write(&existing, b"keep").unwrap();
    host.app
        .configure_receiving(folder.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    let (token, _) = host.pair().await;
    let draft: Value = host
        .post(
            "/api/drafts",
            json!({"streaming":true,"files":[{"name":"file.bin","size":1024*1024}]}),
            &token,
        )
        .await
        .json()
        .await
        .unwrap();
    let sent = host
        .send(&draft, json!([host.app.info().await.id]), None, &token)
        .await;
    let id = sent["ids"][0].as_str().unwrap().to_owned();
    host.accept(&id, &host.token).await;
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|bytes| (Ok::<_, std::io::Error>(bytes), rx))
    }));
    let url = format!(
        "{}/api/transfers/{id}/files/{}/upload",
        host.url,
        draft["files"][0]["id"].as_str().unwrap()
    );
    let job = tokio::spawn(async move {
        Client::new()
            .put(url)
            .bearer_auth(token)
            .body(body)
            .send()
            .await
            .unwrap()
    });
    tx.send(vec![1; 256 * 1024]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        host.post(
            &format!("/api/transfers/{id}/cancel"),
            json!({}),
            &host.token
        )
        .await
        .status(),
        StatusCode::OK
    );
    let _ = tx.send(vec![1; 256 * 1024]).await;
    drop(tx);
    assert!(!job.await.unwrap().status().is_success());
    assert_eq!(std::fs::read(existing).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1);
    assert_eq!(bytes_in(&host.dir.path().join("files")), 0);
}

#[tokio::test]
async fn files_above_two_gib_use_original_and_final_target_without_full_cache() {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let sender = Host::new("大文件发送").await;
    let receiver = Host::new("大文件接收").await;
    let source = TempDir::new().unwrap();
    let path = source.path().join("large.img");
    let length = 2u64 * 1024 * 1024 * 1024 + 17;
    let block = vec![71u8; 1024 * 1024];
    let mut expected = Sha256::new();
    {
        let mut output = std::fs::File::create(&path).unwrap();
        let mut remaining = length;
        while remaining > 0 {
            let count = remaining.min(block.len() as u64) as usize;
            output.write_all(&block[..count]).unwrap();
            expected.update(&block[..count]);
            remaining -= count as u64;
        }
    }
    let target = sender.connect(&receiver).await;
    let draft = sender.source_draft(&path).await;
    let sent = sender
        .send(&draft, json!([target]), None, &sender.token)
        .await;
    let message = sent["message_id"].as_str().unwrap();
    let pending = receiver.wait(message, "pending").await;
    receiver
        .accept(pending["id"].as_str().unwrap(), &receiver.token)
        .await;
    let deadline = std::time::Instant::now() + Duration::from_secs(240);
    loop {
        let state = receiver.state(&receiver.token).await;
        if state["transfers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["message_id"] == message && t["status"] == "completed")
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{state}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let saved = receiver
        .app
        .saved_files(vec![message.into()])
        .await
        .unwrap();
    let mut input = std::fs::File::open(&saved[0].location).unwrap();
    let mut actual = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let count = input.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        actual.update(&buffer[..count]);
    }
    assert_eq!(actual.finalize(), expected.finalize());
    assert_eq!(bytes_in(&sender.dir.path().join("files")), 0);
    assert_eq!(bytes_in(&receiver.dir.path().join("files")), 0);
    assert_eq!(std::fs::metadata(&saved[0].location).unwrap().len(), length);
}

#[tokio::test]
async fn browser_image_stream_is_accepted_automatically_and_mixed_attachments_wait() {
    let host = Host::new("图片接收").await;
    let folder = TempDir::new().unwrap();
    host.app
        .configure_receiving(folder.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    let (token, _) = host.pair().await;
    for (files, status) in [
        (
            json!([{"name":"photo.png","size":3,"mime":"image/png"}]),
            "accepted",
        ),
        (
            json!([{"name":"photo.heic","size":32*1024*1024,"mime":"image/heic"}]),
            "accepted",
        ),
        (
            json!([{"name":"photo.png","size":3,"mime":"image/png"},{"name":"notes.txt","size":1,"mime":"text/plain"}]),
            "pending",
        ),
    ] {
        let draft: Value = host
            .post(
                "/api/drafts",
                json!({"streaming":true,"files":files}),
                &token,
            )
            .await
            .json()
            .await
            .unwrap();
        let sent = host
            .send(&draft, json!([host.app.info().await.id]), None, &token)
            .await;
        let id = sent["ids"][0].as_str().unwrap();
        host.wait(sent["message_id"].as_str().unwrap(), status)
            .await;
        if files[0]["size"] == 3 && files.as_array().unwrap().len() == 1 {
            let response = Client::new()
                .put(format!(
                    "{}/api/transfers/{id}/files/{}/upload",
                    host.url,
                    draft["files"][0]["id"].as_str().unwrap()
                ))
                .bearer_auth(&token)
                .body("png")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            host.wait(sent["message_id"].as_str().unwrap(), "completed")
                .await;
            let saved = host
                .app
                .saved_files(vec![sent["message_id"].as_str().unwrap().into()])
                .await
                .unwrap();
            assert_eq!(saved.len(), 1);
            assert_eq!(std::fs::read(&saved[0].location).unwrap(), b"png");
        } else {
            assert_eq!(
                host.post(&format!("/api/transfers/{id}/cancel"), json!({}), &token)
                    .await
                    .status(),
                StatusCode::OK
            );
        }
    }
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1);
    assert_eq!(bytes_in(&host.dir.path().join("files")), 0);
}

#[tokio::test]
async fn automatic_images_still_check_destination_space_and_can_be_retried() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let host = Host::new("空间检查").await;
    let available = Arc::new(AtomicBool::new(false));
    let probe = available.clone();
    host.app.set_space_probe(Arc::new(move |path| {
        let available = probe.load(Ordering::SeqCst);
        Box::pin(async move {
            let mut disk = linchuan_core::storage::disk_space(Path::new(&path))?;
            disk.available = if available { 8 * 1024 * 1024 * 1024 } else { 0 };
            Ok(disk)
        })
    }));
    let (token, _) = host.pair().await;
    let draft: Value = host
        .post(
            "/api/drafts",
            json!({"streaming":true,"files":[{"name":"photo.jpg","size":3,"mime":"image/jpeg"}]}),
            &token,
        )
        .await
        .json()
        .await
        .unwrap();
    let sent = host
        .send(&draft, json!([host.app.info().await.id]), None, &token)
        .await;
    let pending = host
        .wait(sent["message_id"].as_str().unwrap(), "pending")
        .await;
    assert!(pending["error"].as_str().unwrap().contains("磁盘空间不足"));
    available.store(true, Ordering::SeqCst);
    host.accept(sent["ids"][0].as_str().unwrap(), &host.token)
        .await;
    let accepted = host
        .wait(sent["message_id"].as_str().unwrap(), "accepted")
        .await;
    assert!(accepted["error"].is_null());
}

#[tokio::test]
async fn host_to_browser_text_is_immediate_but_images_complete_only_after_reading() {
    let host = Host::new("宿主").await;
    let (token, browser) = host.pair().await;
    let draft: Value = host
        .post("/api/drafts", json!({"files":[]}), &host.token)
        .await
        .json()
        .await
        .unwrap();
    let sent = host.send(&draft, json!([browser]), None, &host.token).await;
    host.wait(sent["message_id"].as_str().unwrap(), "completed")
        .await;
    let folder = TempDir::new().unwrap();
    let path = folder.path().join("photo.png");
    std::fs::write(&path, b"image bytes").unwrap();
    let draft = host.source_draft(&path).await;
    assert_eq!(draft["files"][0]["mime"], "image/png");
    let sent = host.send(&draft, json!([browser]), None, &host.token).await;
    host.wait(sent["message_id"].as_str().unwrap(), "accepted")
        .await;
    let response = host
        .download(
            sent["ids"][0].as_str().unwrap(),
            draft["files"][0]["id"].as_str().unwrap(),
            &token,
        )
        .await;
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"image bytes");
    host.wait(sent["message_id"].as_str().unwrap(), "completed")
        .await;
    assert_eq!(bytes_in(&host.dir.path().join("files")), 0);
}

#[tokio::test]
async fn native_group_images_are_received_without_individual_confirmation() {
    let sender = Host::new("图片发送").await;
    let one = Host::new("图片接收一").await;
    let two = Host::new("图片接收二").await;
    let members = json!([sender.connect(&one).await, sender.connect(&two).await]);
    let group: Value = sender
        .post(
            "/api/groups",
            json!({"name":"图片群","members":members}),
            &sender.token,
        )
        .await
        .json()
        .await
        .unwrap();
    let group = group["group"]["id"].as_str().unwrap();
    for receiver in [&one, &two] {
        assert_eq!(
            receiver
                .post(
                    &format!("/api/groups/{group}/decision"),
                    json!({"accept":true}),
                    &receiver.token
                )
                .await
                .status(),
            StatusCode::OK
        );
    }
    let source = TempDir::new().unwrap();
    let path = source.path().join("group.webp");
    let data = vec![9; 256 * 1024 + 19];
    std::fs::write(&path, &data).unwrap();
    let draft = sender.source_draft(&path).await;
    let sent = sender
        .send(&draft, json!([]), Some(group), &sender.token)
        .await;
    let message = sent["message_id"].as_str().unwrap();
    for receiver in [&one, &two] {
        receiver.wait(message, "completed").await;
        let saved = receiver
            .app
            .saved_files(vec![message.into()])
            .await
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(std::fs::read(&saved[0].location).unwrap(), data);
        assert_eq!(bytes_in(&receiver.dir.path().join("files")), 0);
    }
    let state = sender.wait(message, "completed").await;
    assert_eq!(state["status"], "completed");
    assert_eq!(bytes_in(&sender.dir.path().join("files")), 0);
}
