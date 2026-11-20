use crate::{model::BrowserAccess, App};
use axum_server::tls_rustls::RustlsConfig;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
};

const CONFIG_FILE: &str = "browser-https.json";

#[derive(Deserialize, Serialize)]
struct Config {
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(flatten)]
    pem: Pem,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Pem {
    Files {
        certificate: PathBuf,
        key: PathBuf,
    },
    Inline {
        certificate_pem: String,
        key_pem: String,
    },
}

impl Config {
    async fn tls(&self, directory: &Path) -> anyhow::Result<RustlsConfig> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = match &self.pem {
            Pem::Files { certificate, key } => {
                RustlsConfig::from_pem_file(directory.join(certificate), directory.join(key))
                    .await?
            }
            Pem::Inline {
                certificate_pem,
                key_pem,
            } => {
                RustlsConfig::from_pem(
                    certificate_pem.as_bytes().to_vec(),
                    key_pem.as_bytes().to_vec(),
                )
                .await?
            }
        };
        Ok(tls)
    }

    fn host(&self) -> anyhow::Result<url::Host> {
        let host = url::Host::parse(self.host.trim())?;
        anyhow::ensure!(
            !host.to_string().is_empty(),
            "请输入证书对应的域名或 IP 地址"
        );
        Ok(host)
    }
}

pub(crate) struct Server {
    listener: TcpListener,
    tls: RustlsConfig,
    pub access: BrowserAccess,
}

impl Server {
    pub async fn load(directory: &Path, http_code: &str) -> anyhow::Result<Option<Self>> {
        let bytes = match tokio::fs::read(directory.join(CONFIG_FILE)).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let config: Config = serde_json::from_slice(&bytes)?;
        let host = config.host()?;
        let tls = config.tls(directory).await?;
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, config.port)).await?;
        let address = format!("https://{host}:{}", listener.local_addr()?.port());
        let mut pairing_code = crate::pairing_code();
        while pairing_code == http_code {
            pairing_code = crate::pairing_code();
        }
        Ok(Some(Self {
            listener: listener.into_std()?,
            tls,
            access: BrowserAccess {
                address,
                pairing_code,
            },
        }))
    }

    pub fn start(self, app: App) -> anyhow::Result<()> {
        let handle = axum_server::Handle::new();
        let server = axum_server::from_tcp_rustls(self.listener, self.tls)?.handle(handle.clone());
        let router = crate::api::router(app.clone(), true);
        let mut shutdown = app.0.stop.subscribe();
        tokio::spawn(async move {
            tokio::select! {
                _ = server.serve(router.into_make_service_with_connect_info::<SocketAddr>()) => {},
                _ = shutdown.changed() => handle.shutdown(),
            }
        });
        Ok(())
    }
}

impl App {
    pub async fn save_browser_https(
        &self,
        host: String,
        port: u16,
        certificate_pem: String,
        key_pem: String,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.0.assistance.is_some(), "当前设备不支持屏幕共享");
        anyhow::ensure!(
            host.len() <= 253 && certificate_pem.len() <= 128 * 1024 && key_pem.len() <= 32 * 1024,
            "HTTPS 配置文件过大"
        );
        let config = Config {
            host,
            port,
            pem: Pem::Inline {
                certificate_pem,
                key_pem,
            },
        };
        config
            .host()
            .map_err(|_| anyhow::anyhow!("请输入证书对应的域名或 IP 地址"))?;
        config
            .tls(&self.0.directory)
            .await
            .map_err(|_| anyhow::anyhow!("无法使用这组证书和私钥，请检查 PEM 格式和配对关系"))?;
        let directory = self.0.directory.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut file = tempfile::NamedTempFile::new_in(&directory)?;
            serde_json::to_writer(&mut file, &config)?;
            file.flush()?;
            file.as_file().sync_all()?;
            file.persist(directory.join(CONFIG_FILE))?;
            Ok(())
        })
        .await??;
        Ok(())
    }
}
