use crate::{model::*, App};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
const SERVICE: &str = "_linchuan._tcp.local.";
pub async fn announce(app: &App) -> anyhow::Result<()> {
    let info = app.info().await;
    let ips: Vec<_> = crate::addresses(app.0.port)
        .iter()
        .filter_map(|s| {
            url::Url::parse(s)
                .ok()?
                .host_str()?
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .collect();
    let properties = [
        ("id", info.id.as_str()),
        ("name", info.name.as_str()),
        ("platform", info.platform.as_str()),
        ("protocol", "1"),
        ("public_key", info.public_key.as_str()),
        ("capabilities", "groups-v1,signed-offers-v1,history-v1"),
    ];
    let service = ServiceInfo::new(
        SERVICE,
        &format!("linchuan-{}", &info.id[..8]),
        &format!("linchuan-{}.local.", &info.id[..8]),
        ips.as_slice(),
        app.0.port,
        &properties[..],
    )?;
    if let Some(daemon) = app
        .0
        .mdns
        .lock()
        .map_err(|_| anyhow::anyhow!("discovery lock"))?
        .as_ref()
    {
        daemon.register(service.enable_addr_auto())?;
    }
    Ok(())
}
pub async fn start(app: App) -> anyhow::Result<()> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SERVICE)?;
    *app.0
        .mdns
        .lock()
        .map_err(|_| anyhow::anyhow!("discovery lock"))? = Some(daemon);
    announce(&app).await?;
    let heartbeat = app.clone();
    tokio::spawn(async move {
        let mut stop = app.0.stop.subscribe();
        loop {
            tokio::select! {
                _=stop.changed()=>break,
                event=receiver.recv_async()=>{
                    let Ok(event)=event else{break};if let ServiceEvent::ServiceResolved(service)=event{
                        if service.get_property_val_str("protocol")!=Some("1"){continue;}
                        let Some(id)=service.get_property_val_str("id")else{continue};if id==app.0.device_id||uuid::Uuid::parse_str(id).is_err(){continue;}
                        let mut ips:Vec<_>=service.get_addresses().iter().filter_map(|ip|match ip{std::net::IpAddr::V4(ip)if ip.is_private()||ip.is_link_local()||ip.is_loopback()=>Some(*ip),_=>None}).collect();ips.sort_by_key(|ip|(!crate::same_subnet(*ip),*ip));let Some(ip)=ips.first()else{continue};
                        let device=Device{id:id.into(),name:service.get_property_val_str("name").unwrap_or("邻传设备").chars().take(64).collect(),platform:service.get_property_val_str("platform").unwrap_or("设备").into(),address:format!("http://{ip}:{}",service.get_port()),last_seen:now(),kind:"app".into(),host_device_id:id.into(),public_key:service.get_property_val_str("public_key").unwrap_or("").into(),capabilities:service.get_property_val_str("capabilities").unwrap_or("").split(',').filter(|s|!s.is_empty()).map(String::from).collect()};
                        let _=app.remember_peer(device).await;
                    }
                }
            }
        }
    });
    tokio::spawn(async move {
        let mut stop = heartbeat.0.stop.subscribe();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {_=stop.changed()=>break,_=interval.tick()=>{
                let peers:Vec<_>=heartbeat.0.state.lock().await.peers.values().cloned().collect();
                for mut peer in peers{if let Ok(response)=heartbeat.0.client.get(format!("{}/api/info",peer.address)).timeout(std::time::Duration::from_secs(2)).send().await{if let Ok(info)=response.json::<serde_json::Value>().await{if info["id"]==peer.id{
                    peer.name=info["name"].as_str().unwrap_or(&peer.name).into();peer.last_seen=now();peer.public_key=info["public_key"].as_str().unwrap_or("").into();peer.capabilities=serde_json::from_value(info["capabilities"].clone()).unwrap_or_default();let _=heartbeat.remember_peer(peer).await;
                }}}}
            }}
        }
    });
    Ok(())
}
