use crate::{model::*, App};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
const SERVICE: &str = "_linchuan._tcp.local.";

pub async fn announce(app: &App) -> anyhow::Result<()> {
    let info = app.info().await;
    let ips: Vec<_> = info
        .addresses
        .iter()
        .filter_map(|address| {
            url::Url::parse(address)
                .ok()?
                .host_str()?
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .collect();
    let capabilities = info.capabilities.join(",");
    let properties = [
        ("id", info.id.as_str()),
        ("name", info.name.as_str()),
        ("platform", info.platform.as_str()),
        ("protocol", "1"),
        ("scheme", "http"),
        ("public_key", info.public_key.as_str()),
        ("capabilities", capabilities.as_str()),
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
            let event = tokio::select! { _ = stop.changed() => break, event = receiver.recv_async() => event };
            let Ok(event) = event else { break };
            let ServiceEvent::ServiceResolved(service) = event else {
                continue;
            };
            if service.get_property_val_str("protocol") != Some("1")
                || service
                    .get_property_val_str("scheme")
                    .is_some_and(|scheme| scheme != "http")
            {
                continue;
            }
            let Some(id) = service.get_property_val_str("id") else {
                continue;
            };
            if id == app.0.device_id || uuid::Uuid::parse_str(id).is_err() {
                continue;
            }
            let public_key = service.get_property_val_str("public_key").unwrap_or("");
            if !hex::decode(public_key).is_ok_and(|key| key.len() == 32) {
                continue;
            }
            let existing = app.0.state.lock().await.profiles.get(id).cloned();
            if existing.as_ref().is_some_and(|device| {
                !device.public_key.is_empty() && device.public_key != public_key
            }) {
                continue;
            }
            let mut ips: Vec<_> = service
                .get_addresses()
                .iter()
                .filter_map(|ip| match ip {
                    std::net::IpAddr::V4(ip)
                        if ip.is_private() || ip.is_link_local() || ip.is_loopback() =>
                    {
                        Some(*ip)
                    }
                    _ => None,
                })
                .collect();
            ips.sort_by_key(|ip| (!crate::same_subnet(*ip), *ip));
            let Some(ip) = ips.first() else { continue };
            let hint = Device {
                id: id.into(),
                host_device_id: id.into(),
                public_key: public_key.into(),
                address: format!("http://{ip}:{}", service.get_port()),
                ..Device::default()
            };
            // Advertisements only locate a candidate. Persist names, keys and capabilities
            // after checking the info response against the advertised identity.
            if let Ok(device) = app.inspect_peer(&hint.address, Some(&hint)).await {
                let _ = app.remember_peer(device).await;
            }
        }
    });
    tokio::spawn(async move {
        let mut stop = heartbeat.0.stop.subscribe();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! { _ = stop.changed() => break, _ = interval.tick() => {
                let peers: Vec<_> = heartbeat.0.state.lock().await.peers.values().cloned().collect();
                for peer in peers {
                    if let Ok(device) = heartbeat.inspect_peer(&peer.address, Some(&peer)).await {
                        let _ = heartbeat.remember_peer(device).await;
                    }
                }
            }}
        }
    });
    Ok(())
}
