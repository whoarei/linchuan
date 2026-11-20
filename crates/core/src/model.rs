use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
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
#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub address: String,
    #[serde(default)]
    pub last_seen: u64,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub host_device_id: String,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
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
    pub public_key: String,
    pub capabilities: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct Bootstrap {
    pub address: String,
    pub token: String,
    pub pairing_code: String,
    pub browser_https: Option<BrowserAccess>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct BrowserAccess {
    pub address: String,
    pub pairing_code: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub last_seen: u64,
    pub expires: u64,
    pub secure: bool,
}

impl Session {
    pub fn online(&self) -> bool {
        self.expires > now() && now().saturating_sub(self.last_seen) < 45
    }
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
    #[serde(default)]
    pub source_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileStorage {
    #[default]
    Cache,
    Original {
        modified: Option<u64>,
        #[serde(default)]
        identity: Option<String>,
    },
    Receiving,
    Browser,
    Received {
        directory: String,
        name: String,
    },
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StoredFile {
    pub meta: FileMeta,
    pub path: PathBuf,
    pub complete: bool,
    pub uploading: bool,
    #[serde(default)]
    pub storage: FileStorage,
    #[serde(default)]
    pub downloaded: bool,
}
impl StoredFile {
    pub fn cached(&self) -> bool {
        self.storage == FileStorage::Cache
    }
    pub fn available(&self) -> bool {
        self.complete
            && !matches!(self.storage, FileStorage::Browser | FileStorage::Receiving)
            && (self.path.to_string_lossy().starts_with("content://") || self.path.is_file())
    }
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
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub created_at: u64,
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
            "completed" | "rejected" | "cancelled" | "failed" | "interrupted"
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
    pub file: StoredFile,
    pub transfer_id: String,
    pub track: bool,
    pub expires: u64,
}
#[derive(Clone, Default)]
pub struct State {
    pub groups: HashMap<String, Group>,
    pub conversations: HashMap<String, Conversation>,
    pub composition: HashMap<String, String>,
    pub profiles: HashMap<String, Device>,
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
    #[serde(default)]
    pub name: String,
}
#[derive(Deserialize)]
pub struct DraftRequest {
    #[serde(default)]
    pub files: Vec<FileInput>,
    #[serde(default)]
    pub streaming: bool,
}
#[derive(Serialize)]
pub struct DraftResponse {
    pub id: String,
    pub files: Vec<FileMeta>,
}
#[derive(Deserialize)]
pub struct SendRequest {
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub text: String,
}
#[derive(Serialize, Deserialize)]
pub struct Offer {
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub identity: Option<SignedEnvelope>,
    #[serde(default)]
    pub membership: Option<SignedEnvelope>,
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

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SignedEnvelope {
    pub host_id: String,
    pub public_key: String,
    pub payload: serde_json::Value,
    pub signature: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct GroupMember {
    pub device: Device,
    pub invite_id: String,
    pub role: String,
    pub state: String,
    pub sequence: u64,
    pub event: Option<SignedEnvelope>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub revision: u64,
    pub created_at: u64,
    pub snapshot: Option<SignedEnvelope>,
    pub members: Vec<GroupMember>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Conversation {
    pub id: String,
    pub principal_id: String,
    pub kind: String,
    pub peer_id: Option<String>,
    pub group_id: Option<String>,
    pub last_read_at: u64,
    pub created_at: u64,
}
impl Conversation {
    pub fn direct(principal: &str, peer: &str) -> Self {
        Self {
            id: format!("direct:{principal}:{peer}"),
            principal_id: principal.into(),
            kind: "direct".into(),
            peer_id: Some(peer.into()),
            group_id: None,
            last_read_at: 0,
            created_at: now_ms(),
        }
    }
    pub fn group(principal: &str, group: &str) -> Self {
        Self {
            id: format!("group:{principal}:{group}"),
            principal_id: principal.into(),
            kind: "group".into(),
            peer_id: None,
            group_id: Some(group.into()),
            last_read_at: 0,
            created_at: now_ms(),
        }
    }
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Clone, Serialize)]
pub struct ConversationView {
    #[serde(flatten)]
    pub conversation: Conversation,
    pub name: String,
    pub peer_kind: Option<String>,
    pub has_draft: bool,
    pub preview: String,
    pub last_message_at: u64,
    pub unread: u64,
}
#[derive(Clone, Serialize)]
pub struct MessageFile {
    #[serde(flatten)]
    pub meta: FileMeta,
    pub available: bool,
    pub transfer_id: String,
    pub original: bool,
}
#[derive(Clone, Serialize)]
pub struct MessageView {
    pub id: String,
    pub position: u64,
    pub sender_id: String,
    pub sender_name: String,
    pub text: String,
    pub created_at: u64,
    pub direction: String,
    pub files: Vec<MessageFile>,
    pub deliveries: Vec<TransferView>,
}

impl State {
    pub fn link_conversations(&mut self, owner: &str) {
        for session in self.sessions.values() {
            let conversation = Conversation::direct(owner, &session.id);
            self.conversations
                .entry(conversation.id.clone())
                .or_insert(conversation);
        }
        let locals: std::collections::HashSet<_> = self
            .sessions
            .values()
            .map(|s| s.id.clone())
            .chain(std::iter::once(owner.into()))
            .collect();
        for t in self.transfers.values() {
            for (id, name) in [
                (&t.view.sender_id, &t.view.sender_name),
                (&t.view.recipient_id, &t.view.recipient_name),
            ] {
                self.profiles.entry(id.clone()).or_insert_with(|| Device {
                    id: id.clone(),
                    name: name.clone(),
                    kind: if id.starts_with("legacy-") {
                        "legacy".into()
                    } else {
                        "app".into()
                    },
                    host_device_id: id.clone(),
                    ..Default::default()
                });
            }
            for (principal, peer) in [
                (&t.view.sender_id, &t.view.recipient_id),
                (&t.view.recipient_id, &t.view.sender_id),
            ] {
                if locals.contains(principal) {
                    let c = if let Some(group) = &t.view.group_id {
                        Conversation::group(principal, group)
                    } else {
                        Conversation::direct(principal, peer)
                    };
                    self.conversations.entry(c.id.clone()).or_insert(c);
                }
            }
        }
    }
}
