mod api;
mod conversations;
mod discovery;
mod groups;
mod identity;
mod model;
mod store;
mod transport;
use model::*;
pub use model::{Bootstrap, Info, VERSION};
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{broadcast, Mutex, Semaphore};

pub struct Options {
    pub directory: PathBuf,
    pub name: Option<String>,
    pub discovery: bool,
}
impl Options {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            name: None,
            discovery: true,
        }
    }
}
struct Inner {
    config: Mutex<Config>,
    device_id: String,
    identity: ed25519_dalek::SigningKey,
    store: store::Store,
    committed: std::sync::Mutex<State>,
    directory: PathBuf,
    work: PathBuf,
    _run_lock: std::fs::File,
    pub port: u16,
    token: String,
    code: String,
    pub state: Mutex<State>,
    events: broadcast::Sender<()>,
    client: reqwest::Client,
    streams: Semaphore,
    mdns: std::sync::Mutex<Option<mdns_sd::ServiceDaemon>>,
    stop: tokio::sync::watch::Sender<bool>,
}
#[derive(Clone)]
pub struct App(Arc<Inner>);
impl App {
    pub async fn start(options: Options) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&options.directory).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&options.directory, std::fs::Permissions::from_mode(0o700))
                .await?;
        }
        let fallback = if !options.directory.join("linchuan.db").exists() {
            match tokio::fs::read(options.directory.join("device.json")).await {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config {
                    id: uid(),
                    name: default_name(),
                },
                Err(e) => return Err(e.into()),
            }
        } else {
            Config {
                id: uid(),
                name: default_name(),
            }
        };
        let directory = options.directory.clone();
        let name = options.name.map(|n| clean_name(&n)).transpose()?;
        let (store, config, key) =
            tokio::task::spawn_blocking(move || store::Store::open(&directory, fallback, name))
                .await??;
        let identity = identity::signing_key(&key)?;
        let mut restored = store.load(config.id.clone()).await?;
        restored.profiles.insert(
            config.id.clone(),
            Device {
                id: config.id.clone(),
                name: config.name.clone(),
                platform: platform().into(),
                kind: "local".into(),
                host_device_id: config.id.clone(),
                public_key: hex::encode(identity.verifying_key().to_bytes()),
                capabilities: capabilities(),
                address: String::new(),
                last_seen: now(),
            },
        );
        tokio::fs::create_dir_all(options.directory.join("files")).await?;
        let temporary = options.directory.join("temporary");
        tokio::fs::create_dir_all(&temporary).await?;
        // An active instance must never lose its files when another instance starts.
        for entry in std::fs::read_dir(&temporary)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Ok(lock) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(entry.path().join(".active"))
            {
                if lock.try_lock().is_ok() {
                    drop(lock);
                    let _ = tokio::fs::remove_dir_all(entry.path()).await;
                }
            }
        }
        let work = temporary.join(uid());
        tokio::fs::create_dir_all(&work).await?;
        let run_lock = std::fs::File::create(work.join(".active"))?;
        run_lock.try_lock()?;
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
        let port = listener.local_addr()?.port();
        let (events, _) = broadcast::channel(128);
        let (stop, _) = tokio::sync::watch::channel(false);
        let random = uuid::Uuid::new_v4().as_u128() % 1_000_000;
        let app = Self(Arc::new(Inner {
            device_id: config.id.clone(),
            identity,
            store,
            committed: std::sync::Mutex::new(restored.clone()),
            config: Mutex::new(config),
            directory: options.directory,
            work,
            _run_lock: run_lock,
            port,
            token: secret(),
            code: format!("{random:06}"),
            state: Mutex::new(restored),
            events,
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()?,
            streams: Semaphore::new(8),
            mdns: std::sync::Mutex::new(None),
            stop,
        }));
        {
            let mut state = app.0.state.lock().await;
            app.commit(&mut state).await?;
        }
        {
            let mut state = app.0.state.lock().await;
            if let Some(local) = state.profiles.get_mut(&app.0.device_id) {
                local.address = addresses(app.0.port)[0].clone();
            }
            app.commit(&mut state).await?;
        }
        let router = api::router(app.clone());
        let mut shutdown = app.0.stop.subscribe();
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = shutdown.changed().await;
            })
            .await;
        });
        if options.discovery {
            if let Err(error) = discovery::start(app.clone()).await {
                eprintln!("Device discovery unavailable: {error}");
            }
        }
        let sweeper = app.clone();
        tokio::spawn(async move {
            let mut stop = sweeper.0.stop.subscribe();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = stop.changed() => break,
                    _ = interval.tick() => sweeper.sweep().await,
                }
            }
        });
        Ok(app)
    }
    async fn commit(&self, state: &mut State) -> Result<(), api::Error> {
        for transfer in state.transfers.values_mut() {
            if transfer.incoming_remote && transfer.view.terminal() && transfer.reserved > 0 {
                let used: u64 = transfer
                    .files
                    .iter()
                    .filter(|f| f.complete && f.path.is_file())
                    .map(|f| f.meta.size)
                    .sum();
                state.reserved = state
                    .reserved
                    .saturating_sub(transfer.reserved.saturating_sub(used));
                transfer.reserved = used;
            }
        }
        state.link_conversations(&self.0.device_id);
        match self.0.store.save(state.clone()).await {
            Ok(()) => {
                // Active tasks stay cached; old completed history remains queryable in SQLite.
                if state.transfers.len() > 500 {
                    let mut finished: Vec<_> = state
                        .transfers
                        .values()
                        .filter(|t| t.view.terminal())
                        .map(|t| (t.view.created_at, t.view.id.clone()))
                        .collect();
                    finished.sort();
                    for (_, id) in finished.into_iter().take(state.transfers.len() - 500) {
                        state.transfers.remove(&id);
                    }
                }
                *self
                    .0
                    .committed
                    .lock()
                    .map_err(|_| api::Error::bad("存储状态异常"))? = state.clone();
                Ok(())
            }
            Err(error) => {
                let drafts = std::mem::take(&mut state.drafts);
                let tickets = std::mem::take(&mut state.tickets);
                let attempts = std::mem::take(&mut state.attempts);
                let mut restored = self
                    .0
                    .committed
                    .lock()
                    .map_err(|_| api::Error::bad("存储状态异常"))?
                    .clone();
                let old_budget: u64 = restored
                    .drafts
                    .values()
                    .filter(|d| !d.sent)
                    .flat_map(|d| &d.files)
                    .map(|f| f.meta.size)
                    .sum();
                let new_budget: u64 = drafts
                    .values()
                    .filter(|d| !d.sent)
                    .flat_map(|d| &d.files)
                    .map(|f| f.meta.size)
                    .sum();
                restored.reserved = restored.reserved.saturating_sub(old_budget) + new_budget;
                restored.drafts = drafts;
                restored.tickets = tickets;
                restored.attempts = attempts;
                *state = restored;
                Err(api::Error::storage(error.to_string()))
            }
        }
    }
    async fn cache_transfer(&self, id: &str) -> Result<(), api::Error> {
        if self.0.state.lock().await.transfers.contains_key(id) {
            return Ok(());
        }
        if let Some(t) = self
            .0
            .store
            .transfer(id.into())
            .await
            .map_err(|e| api::Error::storage(e.to_string()))?
        {
            self.0.state.lock().await.transfers.insert(id.into(), t);
        }
        Ok(())
    }
    async fn signed(&self, payload: serde_json::Value) -> SignedEnvelope {
        identity::sign(&self.0.identity, &self.0.device_id, payload)
    }
    async fn remember_peer(&self, device: Device) -> Result<(), api::Error> {
        let mut state = self.0.state.lock().await;
        if let Some(existing) = state.profiles.get(&device.id) {
            if !existing.public_key.is_empty() && existing.public_key != device.public_key {
                return Err(api::Error::bad("设备身份密钥发生变化，拒绝覆盖"));
            }
        }
        state.profiles.insert(device.id.clone(), device.clone());
        state.peers.insert(device.id.clone(), device);
        self.commit(&mut state).await?;
        drop(state);
        self.notify();
        Ok(())
    }
    pub async fn info(&self) -> Info {
        let config = self.0.config.lock().await.clone();
        Info {
            id: config.id,
            name: config.name,
            platform: platform().into(),
            version: VERSION.into(),
            protocol: 1,
            addresses: addresses(self.0.port),
            public_key: hex::encode(self.0.identity.verifying_key().to_bytes()),
            capabilities: capabilities(),
        }
    }
    pub fn bootstrap(&self) -> Bootstrap {
        Bootstrap {
            address: format!("http://127.0.0.1:{}", self.0.port),
            token: self.0.token.clone(),
            pairing_code: self.0.code.clone(),
        }
    }
    pub fn stop(&self) {
        let _ = self.0.stop.send(true);
        if let Ok(daemon) = self.0.mdns.lock() {
            if let Some(daemon) = daemon.as_ref() {
                let _ = daemon.shutdown();
            }
        }
    }
    fn notify(&self) {
        let _ = self.0.events.send(());
    }
    async fn principal(&self, token: &str) -> Result<Principal, api::Error> {
        if token == self.0.token {
            let c = self.0.config.lock().await;
            return Ok(Principal {
                id: c.id.clone(),
                name: c.name.clone(),
                owner: true,
            });
        }
        let mut state = self.0.state.lock().await;
        if let Some(session) = state.sessions.get_mut(&identity::token_hash(token)) {
            if now() < session.expires {
                session.last_seen = now();
                return Ok(Principal {
                    id: session.id.clone(),
                    name: session.name.clone(),
                    owner: false,
                });
            }
        }
        Err(api::Error::unauthorized())
    }
    async fn sweep(&self) {
        let mut paths = Vec::new();
        let mut state = self.0.state.lock().await;
        state.tickets.retain(|_, t| t.expires > now());
        state.sessions.retain(|_, s| now() < s.expires);
        state.peers.retain(|_, _| true);
        state
            .attempts
            .retain(|_, (start, _)| now().saturating_sub(*start) < 120);
        let stale: Vec<_> = state
            .drafts
            .iter()
            .filter(|(_, d)| {
                now().saturating_sub(d.created) > if d.sent { RETENTION } else { 3600 }
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(d) = state.drafts.remove(&id) {
                state.reserved = state.reserved.saturating_sub(if d.sent {
                    0
                } else {
                    d.files.iter().map(|f| f.meta.size).sum()
                });
                if !d.sent {
                    paths.push(self.0.work.join(&id));
                }
            }
        }
        drop(state);
        for path in paths {
            let _ = tokio::fs::remove_dir_all(path).await;
        }
    }
}
fn platform() -> &'static str {
    if cfg!(target_os = "android") {
        "Android"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    }
}
fn default_name() -> String {
    format!("{} · {}", platform(), &uid()[..4])
}
fn addresses(port: u16) -> Vec<String> {
    let mut candidates: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| {
            let name = interface.name.to_ascii_lowercase();
            if ["rmnet", "ccmni", "pdp_ip"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                return None;
            }
            let std::net::IpAddr::V4(ip) = interface.ip() else {
                return None;
            };
            if !(ip.is_private() || ip.is_link_local()) {
                return None;
            }
            let rank = if name.contains("wlan") || name.contains("wi-fi") || name.contains("wifi") {
                0
            } else if name.starts_with("eth") || name.starts_with("en") || name.contains("ethernet")
            {
                1
            } else {
                2
            };
            Some((rank, format!("http://{ip}:{port}")))
        })
        .collect();
    candidates.sort();
    let mut addresses: Vec<_> = candidates.into_iter().map(|(_, address)| address).collect();
    addresses.dedup();
    if addresses.is_empty() {
        addresses.push(format!("http://127.0.0.1:{port}"));
    }
    addresses
}
fn same_subnet(ip: std::net::Ipv4Addr) -> bool {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .iter()
        .any(|interface| {
            if let if_addrs::IfAddr::V4(address) = &interface.addr {
                let mask = u32::from(address.netmask);
                (u32::from(address.ip) & mask) == (u32::from(ip) & mask)
            } else {
                false
            }
        })
}

fn clean_name(value: &str) -> anyhow::Result<String> {
    let value: String = value.trim().chars().filter(|c| !c.is_control()).collect();
    anyhow::ensure!(
        !value.is_empty() && value.chars().count() <= 64,
        "名称需为 1–64 个字符"
    );
    Ok(value)
}
fn safe_filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value != "."
        && value != ".."
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '\0'))
        && !Path::new(value).is_absolute()
}

fn capabilities() -> Vec<String> {
    vec![
        "groups-v1".into(),
        "signed-offers-v1".into(),
        "history-v1".into(),
    ]
}

async fn sync_directory(path: PathBuf) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
            .await
            .map_err(std::io::Error::other)??;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
