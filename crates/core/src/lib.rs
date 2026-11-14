mod api;
mod discovery;
mod model;
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
        let config_path = options.directory.join("device.json");
        let mut config: Config = match tokio::fs::read(&config_path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config {
                id: uid(),
                name: options.name.clone().unwrap_or_else(default_name),
            },
            Err(e) => return Err(e.into()),
        };
        if let Some(name) = options.name {
            config.name = clean_name(&name)?;
        }
        tokio::fs::write(&config_path, serde_json::to_vec_pretty(&config)?).await?;
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
            config: Mutex::new(config),
            directory: options.directory,
            work,
            _run_lock: run_lock,
            port,
            token: secret(),
            code: format!("{random:06}"),
            state: Mutex::new(State::default()),
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
    pub async fn info(&self) -> Info {
        let config = self.0.config.lock().await.clone();
        Info {
            id: config.id,
            name: config.name,
            platform: platform().into(),
            version: VERSION.into(),
            protocol: 1,
            addresses: addresses(self.0.port),
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
        if let Some(session) = state.sessions.get_mut(token) {
            if now().saturating_sub(session.last_seen) <= RETENTION {
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
        state
            .sessions
            .retain(|_, s| now().saturating_sub(s.last_seen) < RETENTION);
        state
            .peers
            .retain(|_, p| now().saturating_sub(p.last_seen) < 90);
        state
            .attempts
            .retain(|_, (start, _)| now().saturating_sub(*start) < 120);
        let stale: Vec<_> = state
            .transfers
            .iter()
            .filter(|(_, t)| now().saturating_sub(t.view.created) > RETENTION)
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(t) = state.transfers.remove(&id) {
                state.reserved = state.reserved.saturating_sub(t.reserved);
                if t.incoming_remote {
                    paths.push(self.0.work.join(&id));
                }
            }
        }
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
                state.reserved = state
                    .reserved
                    .saturating_sub(d.files.iter().map(|f| f.meta.size).sum());
                paths.push(self.0.work.join(&id));
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
    let mut ips: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) if ip.is_private() || ip.is_link_local() => {
                Some(format!("http://{ip}:{port}"))
            }
            _ => None,
        })
        .collect();
    ips.sort();
    ips.dedup();
    if ips.is_empty() {
        ips.push(format!("http://127.0.0.1:{port}"));
    }
    ips
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
