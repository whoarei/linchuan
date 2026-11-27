use linchuan_core::{App, Bootstrap, Options};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

async fn start() -> (tempfile::TempDir, App, Bootstrap, Client) {
    let directory = tempfile::tempdir().unwrap();
    let mut options = Options::new(directory.path().into());
    options.discovery = false;
    let app = App::start(options).await.unwrap();
    let boot = app.bootstrap();
    (
        directory,
        app,
        boot,
        Client::builder().no_proxy().build().unwrap(),
    )
}

#[tokio::test]
async fn pickup_downloads_original_bytes_for_multiple_visitors_until_closed() {
    let (directory, app, boot, client) = start().await;
    let file_path = directory.path().join("讲义.txt");
    let contents = "会议纪要\n";
    tokio::fs::write(&file_path, contents).await.unwrap();
    let source = app
        .register_file(
            file_path.to_string_lossy().into_owned(),
            "讲义.txt".into(),
            String::new(),
            None,
        )
        .await
        .unwrap();
    let owner = format!("{}/api/scan", boot.address);
    let paired: Value = client
        .post(format!("{}/api/pair", boot.address))
        .json(&json!({
            "name":"普通访客", "code":boot.pairing_code,
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        client
            .get(&owner)
            .bearer_auth(paired["token"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let page: Value = client
        .post(&owner)
        .bearer_auth(&boot.token)
        .json(&json!({
            "kind":"pickup", "title":"会议资料", "minutes":15,
            "sources":[source.source_id], "links":["https://example.org/notes"],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = page["id"].as_str().unwrap();
    let visit = format!("{owner}/visit/{id}");
    let view: Value = client
        .get(&visit)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(view["title"], "会议资料");
    assert!(view["id"].is_null() && view["files"][0]["path"].is_null());
    let download = format!("{visit}/files/{}", view["files"][0]["id"].as_str().unwrap());
    for guest in [&client, &Client::builder().no_proxy().build().unwrap()] {
        let response = guest
            .get(&download)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert!(response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;"));
        assert_eq!(response.text().await.unwrap(), contents);
    }
    tokio::fs::write(&file_path, "已更新的文件长于原文件")
        .await
        .unwrap();
    assert_eq!(
        client.get(&download).send().await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    client
        .post(format!("{owner}/{id}/close"))
        .bearer_auth(&boot.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(
        client.get(visit).send().await.unwrap().status(),
        StatusCode::GONE
    );
    assert_eq!(
        client.get(download).send().await.unwrap().status(),
        StatusCode::GONE
    );
    app.stop();
}

#[tokio::test]
async fn incomplete_submission_can_retry_and_successful_retry_returns_same_receipt() {
    let (directory, app, boot, client) = start().await;
    let owner = format!("{}/api/scan", boot.address);
    let page: Value = client
        .post(&owner)
        .bearer_auth(&boot.token)
        .json(&json!({
            "kind":"dropoff", "title":"收作业", "minutes":15,
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = page["id"].as_str().unwrap();
    let upload = format!("{owner}/visit/{id}/files/00000000000000000000000000000000");
    let submit = |body| {
        client
            .put(&upload)
            .query(&[("name", "作业.txt"), ("size", "3")])
            .body(body)
    };
    let saved = directory.path().join("received");
    assert_eq!(
        submit("ab").send().await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert!(tokio::fs::read_dir(&saved)
        .await
        .unwrap()
        .next_entry()
        .await
        .unwrap()
        .is_none());
    for contents in ["abc", "xyz"] {
        let receipt: Value = submit(contents)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(receipt["name"], "作业.txt");
        assert_eq!(receipt["sender"], "匿名");
        assert_eq!(
            tokio::fs::read(saved.join("作业.txt")).await.unwrap(),
            b"abc"
        );
    }
    let list: Value = client
        .get(&owner)
        .bearer_auth(&boot.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["submissions"].as_array().unwrap().len(), 1);
    app.stop();
}
