use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_STORAGE: u64 = 10 * 1024 * 1024 * 1024;
pub const RETENTION: u64 = 24 * 60 * 60;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub address: String,
    #[serde(default)]
    pub last_seen: u64,
    #[serde(default = "default_kind")]
    pub kind: String,
}
fn default_kind() -> String {
    "app".into()
}
#[derive(Clone, Serialize)]
pub struct Info {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub version: String,
    pub protocol: u8,
    pub addresses: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct Bootstrap {
    pub address: String,
    pub token: String,
    pub pairing_code: String,
}
#[derive(Clone)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub last_seen: u64,
}
#[derive(Clone)]
pub struct Principal {
    pub id: String,
    pub name: String,
    pub owner: bool,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FileMeta {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FileInput {
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub mime: String,
}
#[derive(Clone)]
pub struct StoredFile {
    pub meta: FileMeta,
    pub path: PathBuf,
    pub complete: bool,
    pub uploading: bool,
}
#[derive(Clone)]
pub struct Draft {
    pub id: String,
    pub owner: String,
    pub files: Vec<StoredFile>,
    pub created: u64,
    pub sent: bool,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TransferView {
    pub id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub recipient_id: String,
    pub recipient_name: String,
    pub text: String,
    pub files: Vec<FileMeta>,
    pub status: String,
    pub bytes: u64,
    pub total: u64,
    pub created: u64,
    pub error: Option<String>,
}
impl TransferView {
    pub fn terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "rejected" | "cancelled" | "failed"
        )
    }
}
#[derive(Clone)]
pub struct Transfer {
    pub view: TransferView,
    pub files: Vec<StoredFile>,
    pub peer_token: Option<String>,
    pub incoming_remote: bool,
    pub reserved: u64,
}
#[derive(Clone)]
pub struct Ticket {
    pub path: PathBuf,
    pub name: String,
    pub expires: u64,
}
#[derive(Default)]
pub struct State {
    pub sessions: HashMap<String, Session>,
    pub peers: HashMap<String, Device>,
    pub drafts: HashMap<String, Draft>,
    pub transfers: HashMap<String, Transfer>,
    pub tickets: HashMap<String, Ticket>,
    pub reserved: u64,
    pub attempts: HashMap<String, (u64, u32)>,
}
#[derive(Deserialize)]
pub struct PairRequest {
    pub code: String,
    pub name: String,
}
#[derive(Deserialize)]
pub struct DraftRequest {
    #[serde(default)]
    pub files: Vec<FileInput>,
}
#[derive(Serialize)]
pub struct DraftResponse {
    pub id: String,
    pub files: Vec<FileMeta>,
}
#[derive(Deserialize)]
pub struct SendRequest {
    pub targets: Vec<String>,
    #[serde(default)]
    pub text: String,
}
#[derive(Serialize, Deserialize)]
pub struct Offer {
    pub protocol: u8,
    pub sender_name: String,
    pub text: String,
    pub files: Vec<FileMeta>,
}
#[derive(Serialize, Deserialize)]
pub struct OfferResponse {
    pub id: String,
    pub token: String,
}
