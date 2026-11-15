use crate::{identity, model::SignedEnvelope, store::Store};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, PublicKeyData,
    SanType,
};
use rusqlite::OptionalExtension;
use rustls::{
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        WebPkiServerVerifier,
    },
    crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider},
    pki_types::{pem::PemObject, CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const BINDING_PREFIX: &str = "urn:linchuan:tls-identity:";

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    certificate_pem: String,
    private_key_pem: String,
}

pub(crate) struct LocalTls {
    pub certificate_pem: String,
    pub fingerprint: String,
    issuer: Issuer<'static, KeyPair>,
    root: CertificateDer<'static>,
    names: tokio::sync::Mutex<(Vec<String>, u64)>,
    pub server: RustlsConfig,
}

impl LocalTls {
    pub async fn load(
        store: &Store,
        key: &ed25519_dalek::SigningKey,
        id: &str,
        names: Vec<String>,
    ) -> anyhow::Result<Self> {
        let expected = PeerIdentity {
            id: id.into(),
            public_key: hex::encode(key.verifying_key().to_bytes()),
        };
        let key = key.clone();
        let id = id.to_owned();
        let stored = store.run(move |connection, _| {
            let value: Option<String> = connection.query_row("SELECT value FROM app_settings WHERE key='tls_identity'", [], |row| row.get(0)).optional()?;
            if let Some(value) = value { return Ok(serde_json::from_str::<StoredIdentity>(&value)?); }
            let pair = KeyPair::generate()?;
            let binding = identity::sign(&key, &id, json!({"type":"tls-ca-v1","public_key":hex::encode(pair.subject_public_key_info())}));
            let mut params = CertificateParams::default();
            params.distinguished_name.push(DnType::CommonName, format!("Linchuan {} Local CA", id));
            params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
            params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
            params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
            params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(3650);
            params.subject_alt_names.push(SanType::URI(format!("{BINDING_PREFIX}{}", hex::encode(serde_json::to_vec(&binding)?)).try_into()?));
            params.name_constraints = Some(NameConstraints {
                permitted_subtrees: vec![
                    GeneralSubtree::DnsName("localhost".into()),
                    GeneralSubtree::DnsName("local".into()),
                    GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([10,0,0,0],8)),
                    GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([172,16,0,0],12)),
                    GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([192,168,0,0],16)),
                    GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([127,0,0,0],8)),
                    GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([169,254,0,0],16)),
                ], excluded_subtrees: vec![],
            });
            let certificate = params.self_signed(&pair)?;
            let stored = StoredIdentity { certificate_pem: certificate.pem(), private_key_pem: pair.serialize_pem() };
            connection.execute("INSERT INTO app_settings(key,value) VALUES('tls_identity',?1)", [serde_json::to_string(&stored)?])?;
            Ok(stored)
        }).await?;
        let root = CertificateDer::from_pem_slice(stored.certificate_pem.as_bytes())?;
        let binding = ca_identity(&root)?;
        anyhow::ensure!(
            binding.host_id == expected.id && binding.public_key == expected.public_key,
            "本机 TLS 证书身份无效"
        );
        let pair = KeyPair::from_pem(&stored.private_key_pem)?;
        let (_, parsed) = x509_parser::parse_x509_certificate(&root)
            .map_err(|_| anyhow::anyhow!("CA 证书格式错误"))?;
        anyhow::ensure!(
            pair.subject_public_key_info() == parsed.public_key().raw,
            "本机 TLS 私钥不匹配"
        );
        let issuer = Issuer::from_ca_cert_pem(&stored.certificate_pem, pair)?;
        let (certificates, private_key) = leaf(&issuer, &root, &names)?;
        let server = RustlsConfig::from_config(server_config(certificates, private_key)?);
        Ok(Self {
            fingerprint: hex::encode(Sha256::digest(&root)),
            certificate_pem: stored.certificate_pem,
            issuer,
            root,
            names: tokio::sync::Mutex::new((names, crate::model::now())),
            server,
        })
    }

    pub async fn refresh(&self, names: Vec<String>) -> anyhow::Result<()> {
        let mut old = self.names.lock().await;
        if old.0 != names || crate::model::now().saturating_sub(old.1) > 7 * 24 * 3600 {
            let (certificates, key) = leaf(&self.issuer, &self.root, &names)?;
            self.server
                .reload_from_config(server_config(certificates, key)?);
            *old = (names, crate::model::now());
        }
        Ok(())
    }
}

fn server_config(
    certificates: Vec<Vec<u8>>,
    key: Vec<u8>,
) -> anyhow::Result<Arc<rustls::ServerConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            certificates.into_iter().map(CertificateDer::from).collect(),
            rustls::pki_types::PrivatePkcs8KeyDer::from(key).into(),
        )?;
    Ok(Arc::new(config))
}

fn leaf(
    issuer: &Issuer<'_, KeyPair>,
    root: &CertificateDer<'_>,
    names: &[String],
) -> anyhow::Result<(Vec<Vec<u8>>, Vec<u8>)> {
    let pair = KeyPair::generate()?;
    let mut params = CertificateParams::new(names.to_vec())?;
    params
        .distinguished_name
        .push(DnType::CommonName, "Linchuan LAN connection");
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(30);
    let certificate = params.signed_by(&pair, issuer)?;
    Ok((
        vec![certificate.der().to_vec(), root.to_vec()],
        pair.serialize_der(),
    ))
}

fn ca_identity(root: &CertificateDer<'_>) -> anyhow::Result<SignedEnvelope> {
    let (remaining, certificate) = x509_parser::parse_x509_certificate(root)
        .map_err(|_| anyhow::anyhow!("CA 证书格式错误"))?;
    anyhow::ensure!(remaining.is_empty(), "CA 证书含额外数据");
    let names = certificate
        .subject_alternative_name()?
        .ok_or_else(|| anyhow::anyhow!("缺少设备身份绑定"))?;
    let mut bindings = names.value.general_names.iter().filter_map(|name| {
        if let x509_parser::extensions::GeneralName::URI(uri) = name {
            uri.strip_prefix(BINDING_PREFIX)
        } else {
            None
        }
    });
    let encoded = bindings
        .next()
        .ok_or_else(|| anyhow::anyhow!("缺少设备身份绑定"))?;
    anyhow::ensure!(bindings.next().is_none(), "证书身份绑定重复");
    let binding: SignedEnvelope = serde_json::from_slice(&hex::decode(encoded)?)?;
    identity::verify(&binding)?;
    anyhow::ensure!(
        binding.payload
            == json!({"type":"tls-ca-v1","public_key":hex::encode(certificate.public_key().raw)}),
        "CA 公钥与设备签名不匹配"
    );
    Ok(binding)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerIdentity {
    pub id: String,
    pub public_key: String,
}

#[derive(Debug)]
struct DeviceVerifier {
    expected: Option<PeerIdentity>,
    observed: Arc<Mutex<Option<PeerIdentity>>>,
    provider: Arc<CryptoProvider>,
}
impl ServerCertVerifier for DeviceVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let invalid = || {
            rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            )
        };
        if intermediates.len() != 1 {
            return Err(invalid());
        }
        let root = &intermediates[0];
        let binding = ca_identity(root).map_err(|_| invalid())?;
        let identity = PeerIdentity {
            id: binding.host_id,
            public_key: binding.public_key,
        };
        if self
            .expected
            .as_ref()
            .is_some_and(|expected| expected != &identity)
        {
            return Err(invalid());
        }
        let (_, ca) = x509_parser::parse_x509_certificate(root).map_err(|_| invalid())?;
        let seconds = i64::try_from(now.as_secs()).map_err(|_| invalid())?;
        let at = x509_parser::time::ASN1Time::from_timestamp(seconds).map_err(|_| invalid())?;
        if !ca.validity().is_valid_at(at) {
            return Err(invalid());
        }
        let mut roots = RootCertStore::empty();
        roots.add(root.clone()).map_err(|_| invalid())?;
        let verifier =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), self.provider.clone())
                .build()
                .map_err(|_| invalid())?;
        let verified = verifier.verify_server_cert(end_entity, &[], name, ocsp, now)?;
        let mut observed = self.observed.lock().map_err(|_| invalid())?;
        if observed
            .as_ref()
            .is_some_and(|previous| previous != &identity)
        {
            return Err(invalid());
        }
        *observed = Some(identity);
        Ok(verified)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) struct PeerClient {
    pub client: reqwest::Client,
    observed: Arc<Mutex<Option<PeerIdentity>>>,
}
impl PeerClient {
    pub fn new(expected: Option<PeerIdentity>) -> anyhow::Result<Self> {
        let observed = Arc::new(Mutex::new(None));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DeviceVerifier {
                expected,
                observed: observed.clone(),
                provider,
            }))
            .with_no_client_auth();
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(config)
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self { client, observed })
    }
    pub fn identity(&self) -> anyhow::Result<PeerIdentity> {
        self.observed
            .lock()
            .map_err(|_| anyhow::anyhow!("证书状态不可用"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("尚未校验对方证书"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Options};

    async fn app() -> (tempfile::TempDir, App) {
        let directory = tempfile::tempdir().unwrap();
        let mut options = Options::new(directory.path().into());
        options.discovery = false;
        let app = App::start(options).await.unwrap();
        (directory, app)
    }
    fn verifier(app: &App) -> DeviceVerifier {
        DeviceVerifier {
            expected: Some(PeerIdentity {
                id: app.0.device_id.clone(),
                public_key: hex::encode(app.0.identity.verifying_key().to_bytes()),
            }),
            observed: Arc::new(Mutex::new(None)),
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }

    #[tokio::test]
    async fn valid_chain_still_rejects_the_wrong_address_and_an_expired_leaf() {
        let (_directory, app) = app().await;
        let local = &app.0.tls;
        let (chain, _) = leaf(&local.issuer, &local.root, &["127.0.0.1".into()]).unwrap();
        let valid = CertificateDer::from(chain[0].clone());
        let root = [local.root.clone()];
        let name = ServerName::try_from("127.0.0.1").unwrap();
        assert!(verifier(&app)
            .verify_server_cert(&valid, &root, &name, &[], UnixTime::now())
            .is_ok());
        assert!(verifier(&app)
            .verify_server_cert(
                &valid,
                &root,
                &ServerName::try_from("127.0.0.2").unwrap(),
                &[],
                UnixTime::now()
            )
            .is_err());
        let pair = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(2);
        params.not_after = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let expired = params.signed_by(&pair, &local.issuer).unwrap();
        assert!(verifier(&app)
            .verify_server_cert(expired.der(), &root, &name, &[], UnixTime::now())
            .is_err());
        app.stop();
    }

    #[tokio::test]
    async fn a_copied_identity_binding_cannot_authenticate_another_ca_key() {
        let (_directory, app) = app().await;
        let binding = ca_identity(&app.0.tls.root).unwrap();
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.subject_alt_names.push(SanType::URI(
            format!(
                "{BINDING_PREFIX}{}",
                hex::encode(serde_json::to_vec(&binding).unwrap())
            )
            .try_into()
            .unwrap(),
        ));
        let forged = params.self_signed(&KeyPair::generate().unwrap()).unwrap();
        assert!(ca_identity(forged.der()).is_err());
        // A valid key binding does not override the CA's own expiry.
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(2);
        params.not_after = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        let expired = params.self_signed(app.0.tls.issuer.key()).unwrap();
        assert!(ca_identity(expired.der()).is_ok());
        let (chain, _) = leaf(&app.0.tls.issuer, &app.0.tls.root, &["127.0.0.1".into()]).unwrap();
        assert!(verifier(&app)
            .verify_server_cert(
                &CertificateDer::from(chain[0].clone()),
                &[expired.der().clone()],
                &ServerName::try_from("127.0.0.1").unwrap(),
                &[],
                UnixTime::now()
            )
            .is_err());
        app.stop();
    }

    #[tokio::test]
    async fn long_running_service_renews_leaf_without_changing_browser_trust() {
        let (_directory, app) = app().await;
        let root = reqwest::Certificate::from_pem(app.0.tls.certificate_pem.as_bytes()).unwrap();
        let client = reqwest::Client::builder()
            .add_root_certificate(root)
            .tls_info(true)
            .no_proxy()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let address = format!("{}/api/info", app.bootstrap().https_address);
        let before = client.get(&address).send().await.unwrap();
        let old_leaf = before
            .extensions()
            .get::<reqwest::tls::TlsInfo>()
            .unwrap()
            .peer_certificate()
            .unwrap()
            .to_vec();
        let fingerprint = app.0.tls.fingerprint.clone();
        let names = {
            let mut state = app.0.tls.names.lock().await;
            state.1 = crate::model::now() - 8 * 24 * 3600;
            state.0.clone()
        };
        app.0.tls.refresh(names).await.unwrap();
        let after = client.get(&address).send().await.unwrap();
        let new_leaf = after
            .extensions()
            .get::<reqwest::tls::TlsInfo>()
            .unwrap()
            .peer_certificate()
            .unwrap();
        assert_ne!(old_leaf, new_leaf);
        assert_eq!(app.0.tls.fingerprint, fingerprint);
        let mut changed_names = crate::tls_names(&app.0.device_id);
        changed_names.push("network-change.local".into());
        app.0.tls.refresh(changed_names).await.unwrap();
        let port = url::Url::parse(&app.bootstrap().https_address)
            .unwrap()
            .port()
            .unwrap();
        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(app.0.tls.certificate_pem.as_bytes()).unwrap(),
            )
            .resolve(
                "network-change.local",
                std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            )
            .no_proxy()
            .build()
            .unwrap();
        assert!(client
            .get(format!("https://network-change.local:{port}/api/info"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        assert_eq!(app.0.tls.fingerprint, fingerprint);
        app.stop();
    }

    #[tokio::test]
    async fn endpoint_substitution_is_rejected_before_sending_text_or_credentials() {
        let (_sender_directory, sender) = app().await;
        let (_receiver_directory, receiver) = app().await;
        let (_impostor_directory, impostor) = app().await;
        let device = sender
            .inspect_peer(&receiver.bootstrap().https_address, None)
            .await
            .unwrap();
        let target = device.id.clone();
        sender.remember_peer(device).await.unwrap();
        {
            let mut state = sender.0.state.lock().await;
            state.peers.get_mut(&target).unwrap().address = impostor.bootstrap().https_address;
            state.profiles.get_mut(&target).unwrap().address = impostor.bootstrap().https_address;
        }
        let bootstrap = sender.bootstrap();
        let client = reqwest::Client::new();
        let draft: serde_json::Value = client
            .post(format!("{}/api/drafts", bootstrap.address))
            .bearer_auth(&bootstrap.token)
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
                bootstrap.address,
                draft["id"].as_str().unwrap()
            ))
            .bearer_auth(&bootstrap.token)
            .json(&json!({"targets":[target],"text":"不能泄露给冒充设备"}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let mut rejected = false;
        for _ in 0..100 {
            rejected = sender
                .0
                .state
                .lock()
                .await
                .transfers
                .values()
                .any(|transfer| transfer.view.status == "failed");
            if rejected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(rejected);
        assert!(impostor.0.state.lock().await.transfers.is_empty());
        assert!(receiver.0.state.lock().await.transfers.is_empty());
        sender.stop();
        receiver.stop();
        impostor.stop();
    }
}
