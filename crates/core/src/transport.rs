use crate::{model::*, App};
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio_util::io::ReaderStream;

pub async fn send(app: App, id: String, address: String) {
    let result = run(&app, &id, &address).await;
    if let Err(error) = result {
        let mut state = app.0.state.lock().await;
        if let Some(t) = state.transfers.get_mut(&id) {
            if !t.view.terminal() {
                t.view.status = "failed".into();
                t.view.error = Some(error.to_string());
            }
        }
        if let Err(e) = app.commit(&mut state).await {
            eprintln!("Cannot persist delivery failure: {e}");
        }
        drop(state);
        app.notify();
    }
}
async fn update(app: &App, id: &str, status: &str, bytes: u64) -> bool {
    let mut state = app.0.state.lock().await;
    if let Some(t) = state.transfers.get_mut(id) {
        if !t.view.terminal() {
            let old = t.view.status.clone();
            t.view.status = status.into();
            t.view.bytes = bytes;
            if (old != status
                || matches!(status, "completed" | "rejected" | "cancelled" | "failed"))
                && app.commit(&mut state).await.is_err()
            {
                return false;
            }
            drop(state);
            app.notify();
            return true;
        }
    }
    false
}
async fn active(app: &App, id: &str) -> bool {
    app.0
        .state
        .lock()
        .await
        .transfers
        .get(id)
        .is_some_and(|t| !t.view.terminal())
}
async fn run(app: &App, id: &str, address: &str) -> anyhow::Result<()> {
    crate::api::local_address(address).map_err(|_| anyhow::anyhow!("设备地址无效"))?;
    let transfer = app
        .0
        .state
        .lock()
        .await
        .transfers
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("传输已过期"))?;
    if transfer
        .files
        .iter()
        .any(|f| f.meta.size > 2 * 1024 * 1024 * 1024)
    {
        let peer: serde_json::Value = app
            .0
            .client
            .get(format!("{address}/api/info"))
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::ensure!(
            peer["capabilities"]
                .as_array()
                .is_some_and(|values| values.iter().any(|v| v == "dynamic-storage-v1")),
            "对方版本仅支持单文件 2 GiB，请先升级接收端"
        );
    }
    let mut offer = Offer {
        protocol: 1,
        message_id: transfer.view.message_id.clone(),
        group_id: transfer.view.group_id.clone(),
        sender_id: transfer.view.sender_id.clone(),
        target_id: transfer.view.recipient_id.clone(),
        created_at: transfer.view.created_at,
        sender_name: transfer.view.sender_name.clone(),
        text: transfer.view.text.clone(),
        files: transfer.view.files.clone(),
        identity: None,
        membership: None,
    };
    if let Some(group_id) = &offer.group_id {
        let state = app.0.state.lock().await;
        let group = state
            .groups
            .get(group_id)
            .ok_or_else(|| anyhow::anyhow!("群不存在"))?;
        let member = group
            .members
            .iter()
            .find(|m| m.device.id == offer.sender_id && m.state == "joined")
            .ok_or_else(|| anyhow::anyhow!("尚未加入群聊"))?;
        offer.membership = member.event.clone();
    }
    offer.identity = Some(app.signed(crate::identity::offer_payload(&offer)).await);
    let response = app
        .0
        .client
        .post(format!("{address}/api/peer/offer"))
        .json(&offer)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("无法连接接收设备，请检查网络和防火墙"))?;
    anyhow::ensure!(
        response.status().is_success(),
        "接收设备拒绝了请求，可能接收队列或空间已满"
    );
    let remote: OfferResponse = response.json().await?;
    let endpoint = format!("{address}/api/peer/transfers/{}", remote.id);
    let result=async{
        let start=Instant::now();
        loop{
            if !active(app,id).await{return Ok(());}
            anyhow::ensure!(start.elapsed()<Duration::from_secs(300),"等待接收确认超时，请重新发送");
            let response=app.0.client.get(&endpoint).bearer_auth(&remote.token).timeout(Duration::from_secs(5)).send().await?;
            anyhow::ensure!(response.status().is_success(),"接收请求已过期");
            let status:TransferView=response.json().await?;
            match status.status.as_str(){
                "accepted"=>break,
                "completed"=>{update(app,id,"completed",status.total).await;return Ok(());},
                "rejected"|"cancelled"|"failed"|"interrupted"=>{update(app,id,&status.status,0).await;return Ok(());},
                _=>tokio::time::sleep(Duration::from_millis(400)).await,
            }
        }
        let mut bytes=0;
        if !update(app,id,"transferring",bytes).await{return Ok(());}
        for file in &transfer.files{
            if !active(app,id).await{return Ok(());}
            let _permit=app.0.outgoing.acquire().await?;
            let input=app.open_source(file).await.map_err(anyhow::Error::msg)?;
            let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let uploaded = progress.clone();
            let stream = ReaderStream::new(input).map(move |chunk| {
                if let Ok(bytes) = &chunk { uploaded.fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed); }
                chunk
            });
            let upload=app.0.client.put(format!("{endpoint}/files/{}",file.meta.id)).bearer_auth(&remote.token).header("content-length",file.meta.size)
                .body(reqwest::Body::wrap_stream(stream)).send();
            tokio::pin!(upload);
            let mut last_bytes = 0;
            let mut last_progress = Instant::now();
            let response=loop{tokio::select!{
                response=&mut upload=>break response?,
                _=tokio::time::sleep(Duration::from_millis(300))=>{if !active(app,id).await{return Ok(());}
                    let count = progress.load(std::sync::atomic::Ordering::Relaxed);
                    if count > last_bytes { last_bytes = count; last_progress = Instant::now(); }
                    anyhow::ensure!(last_progress.elapsed() < crate::storage::IDLE_TIMEOUT, "连续 5 分钟无文件传输进度，请重新发送");
                    update(app,id,"transferring",bytes+count).await;}
            }};
            anyhow::ensure!(response.status().is_success(),"文件传输失败，对方可能已取消或磁盘空间不足");
            // Check again after streaming; never acknowledge a changed original as complete.
            let _=app.open_source(file).await.map_err(anyhow::Error::msg)?;
            bytes+=file.meta.size;
            if !update(app,id,"transferring",bytes).await{return Ok(());}
        }
        if !active(app,id).await{return Ok(());}
        let response=app.0.client.post(format!("{endpoint}/complete")).bearer_auth(&remote.token).timeout(Duration::from_secs(10)).send().await?;
        anyhow::ensure!(response.status().is_success(),"接收方未确认传输完成");
        update(app,id,"completed",bytes).await;
        Ok::<_,anyhow::Error>(())
    }.await;
    let completed = app
        .0
        .state
        .lock()
        .await
        .transfers
        .get(id)
        .is_some_and(|t| t.view.status == "completed");
    if !completed {
        let _ = app
            .0
            .client
            .post(format!("{endpoint}/cancel"))
            .bearer_auth(&remote.token)
            .timeout(Duration::from_secs(3))
            .send()
            .await;
    }
    result
}
