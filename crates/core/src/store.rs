use crate::{identity, model::*};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Store {
    inner: Arc<Mutex<Database>>,
    root: PathBuf,
}
struct Database {
    conn: Connection,
    saved: HashMap<String, String>,
}
impl Store {
    pub fn open(
        root: &Path,
        fallback: Config,
        override_name: Option<String>,
    ) -> anyhow::Result<(Self, Config, String)> {
        let path = root.join("linchuan.db");
        let mut conn = Connection::open(&path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        anyhow::ensure!(version <= 3, "数据库版本高于当前程序支持的版本，请升级邻传");
        if version < 2 {
            let tx = conn.transaction()?;
            if version == 0 {
                tx.execute_batch(include_str!("../migrations/001_initial.sql"))?;
            }
            tx.execute_batch(include_str!("../migrations/002_received_files.sql"))?;
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
        }
        if version < 3 {
            let tx = conn.transaction()?;
            tx.execute_batch(include_str!("../migrations/003_file_sources.sql"))?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit()?;
        }
        let get = |key: &str| -> rusqlite::Result<Option<String>> {
            conn.query_row("SELECT value FROM app_settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
        };
        let config = Config {
            id: get("device_id")?.unwrap_or(fallback.id),
            name: override_name
                .or(get("device_name")?)
                .unwrap_or(fallback.name),
        };
        let key = get("identity_secret")?.unwrap_or_else(identity::create_key);
        let tx = conn.transaction()?;
        for (k, v) in [
            ("device_id", config.id.as_str()),
            ("device_name", config.name.as_str()),
            ("identity_secret", key.as_str()),
        ] {
            tx.execute("INSERT INTO app_settings VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![k,v])?;
        }
        tx.execute("UPDATE deliveries SET status='interrupted',error='应用重启，传输已中断，请重新发送',updated_at=?1 WHERE status IN ('pending','accepted','transferring')",[now_ms()])?;
        tx.execute(
            "DELETE FROM browser_sessions WHERE expires_at<=?1",
            [now_ms()],
        )?;
        tx.commit()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok((
            Self {
                inner: Arc::new(Mutex::new(Database {
                    conn,
                    saved: HashMap::new(),
                })),
                root: root.into(),
            },
            config,
            key,
        ))
    }
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection, &Path) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let inner = self.inner.clone();
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = inner.lock().map_err(|_| anyhow::anyhow!("数据库锁异常"))?;
            f(&mut db.conn, &root)
        })
        .await?
    }
    pub async fn save(&self, state: State) -> anyhow::Result<()> {
        let inner = self.inner.clone();
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let mut db=inner.lock().map_err(|_|anyhow::anyhow!("数据库锁异常"))?;
            let mut changed=HashMap::new();
            // Diff metadata in memory; only changed rows are written inside the transaction.
            for (id,t) in &state.transfers { let value=serde_json::to_string(&(&t.view,t.incoming_remote,t.files.iter().map(|f|(f,f.available())).collect::<Vec<_>>()))?; if db.saved.get(id)!=Some(&value) {changed.insert(id.clone(),value);} }
            let tx=db.conn.transaction()?;
            if let Some(local)=state.profiles.values().find(|d|d.kind=="local"){tx.execute("UPDATE app_settings SET value=?1 WHERE key='device_name'",[&local.name])?;}
            for device in state.profiles.values() {upsert_device(&tx,device)?;}
            for group in state.groups.values() {
                for m in &group.members {if !state.profiles.contains_key(&m.device.id){upsert_device(&tx,&m.device)?;}}
                tx.execute("INSERT INTO groups VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,revision=excluded.revision,snapshot_json=excluded.snapshot_json,updated_at=CASE WHEN groups.revision<excluded.revision THEN excluded.updated_at ELSE groups.updated_at END",params![group.id,group.name,group.owner_id,group.revision,serde_json::to_string(&group.snapshot)?,group.created_at,now_ms()])?;
                for m in &group.members {tx.execute("INSERT INTO group_members VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(group_id,device_id) DO UPDATE SET invite_id=excluded.invite_id,role=excluded.role,state=excluded.state,sequence=excluded.sequence,event_json=excluded.event_json,invited_at=CASE WHEN group_members.invite_id<>excluded.invite_id THEN excluded.invited_at ELSE group_members.invited_at END,joined_at=CASE WHEN group_members.invite_id<>excluded.invite_id THEN excluded.joined_at ELSE COALESCE(group_members.joined_at,excluded.joined_at) END,left_at=CASE WHEN group_members.invite_id<>excluded.invite_id THEN excluded.left_at ELSE COALESCE(group_members.left_at,excluded.left_at) END",params![group.id,m.device.id,m.invite_id,m.role,m.state,m.sequence,serde_json::to_string(&m.event)?,now_ms(),if m.state=="joined"{Some(now_ms())}else{None},if m.state=="left"{Some(now_ms())}else{None}])?;}
            }
            tx.execute("DELETE FROM browser_sessions",[])?;
            for (hash,session) in &state.sessions {tx.execute("INSERT INTO browser_sessions VALUES(?1,?2,?3,?4) ON CONFLICT(token_hash) DO UPDATE SET last_seen_at=excluded.last_seen_at",params![hash,session.id,session.expires*1000,session.last_seen*1000])?;}
            for c in state.conversations.values() {tx.execute("INSERT INTO conversations(id,principal_id,kind,peer_id,group_id,last_read_at,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET last_read_at=MAX(last_read_at,excluded.last_read_at)",params![c.id,c.principal_id,c.kind,c.peer_id,c.group_id,c.last_read_at,c.created_at])?;}
            let local:HashSet<_>=state.sessions.values().map(|s|s.id.as_str()).chain(state.profiles.values().filter(|d|d.kind=="local").map(|d|d.id.as_str())).collect();
            for id in changed.keys() {
                let t=&state.transfers[id];let v=&t.view;
                let existing:Option<(String,String,Option<String>)>=tx.query_row("SELECT sender_id,text,group_id FROM messages WHERE id=?1",[&v.message_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                if let Some((sender,text,group))=existing {anyhow::ensure!(sender==v.sender_id&&text==v.text&&group==v.group_id,"消息标识冲突，拒绝覆盖历史");}
                tx.execute("INSERT OR IGNORE INTO messages VALUES(?1,?2,?3,?4,?5,?6,?7)",params![v.message_id,v.sender_id,v.sender_name,v.group_id,v.text,v.created_at,now_ms()])?;
                for (index,f) in v.files.iter().enumerate(){
                    let previous:Option<(String,u64,String)>=tx.query_row("SELECT name,size,mime FROM attachments WHERE message_id=?1 AND file_id=?2",params![v.message_id,f.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                    if let Some(old)=previous {anyhow::ensure!(old==(f.name.clone(),f.size,f.mime.clone()),"附件清单与历史消息冲突");}
                    tx.execute("INSERT OR IGNORE INTO attachments VALUES(?1,?2,?3,?4,?5,?6)",params![v.message_id,f.id,f.name,f.size,f.mime,index])?;
                }
                tx.execute("INSERT INTO deliveries VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET status=excluded.status,bytes=excluded.bytes,error=excluded.error,updated_at=excluded.updated_at",params![v.id,v.message_id,v.recipient_id,v.recipient_name,v.status,v.bytes,v.total,v.error,t.incoming_remote,v.created_at,now_ms()])?;
                for f in &t.files {
                    let mut persisted=f.clone();
                    if f.cached(){persisted.path=f.path.strip_prefix(&root)?.into();}
                    let encoded=serde_json::to_string(&persisted)?;
                    tx.execute("INSERT INTO file_sources VALUES(?1,?2,?3) ON CONFLICT(delivery_id,file_id) DO UPDATE SET source_json=excluded.source_json",params![v.id,f.meta.id,encoded])?;
                    if f.cached() {
                        let relative=f.path.strip_prefix(&root)?.to_str().ok_or_else(||anyhow::anyhow!("附件路径无效"))?;
                        anyhow::ensure!(relative.starts_with("files/")||relative.starts_with("files\\"),"持久缓存必须位于 files 目录");
                        let blob=identity::token_hash(relative);let availability=if f.available(){"available"}else{"missing"};
                        tx.execute("INSERT INTO file_blobs VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET state=CASE WHEN file_blobs.state='expired' AND excluded.state='missing' THEN 'expired' ELSE excluded.state END",params![blob,relative,f.meta.size,availability,now_ms()])?;
                        tx.execute("INSERT INTO delivery_files VALUES(?1,?2,?3,?4) ON CONFLICT(delivery_id,file_id) DO UPDATE SET complete=excluded.complete",params![v.id,f.meta.id,blob,f.complete])?;
                        if v.status == "completed" && f.complete && state.profiles.get(&v.recipient_id).is_some_and(|d|d.kind=="local") {
                            tx.execute("INSERT OR IGNORE INTO saved_files(message_id,file_id,transfer_id,name,status,updated_at) SELECT ?1,?2,?3,?4,'pending',?5 WHERE EXISTS(SELECT 1 FROM app_settings WHERE key='receive_directory') AND (SELECT received_at FROM messages WHERE id=?1)>=CAST((SELECT value FROM app_settings WHERE key='receive_since') AS INTEGER)",params![v.message_id,f.meta.id,v.id,f.meta.name,now_ms()])?;
                        }
                    } else if let FileStorage::Received {directory,name}=&f.storage {
                        if f.complete && state.profiles.get(&v.recipient_id).is_some_and(|d|d.kind=="local") {
                            tx.execute("INSERT INTO saved_files(message_id,file_id,transfer_id,name,directory,location,status,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'saved',?7) ON CONFLICT(message_id,file_id) DO UPDATE SET name=excluded.name,directory=excluded.directory,location=excluded.location,status='saved',error=NULL,updated_at=excluded.updated_at WHERE saved_files.status<>'saved'",params![v.message_id,f.meta.id,v.id,name,directory,f.path.to_string_lossy(),now_ms()])?;
                        }
                    }
                    if f.complete && matches!(f.storage,FileStorage::Received{..}) {
                        tx.execute("DELETE FROM receive_outputs WHERE delivery_id=?1 AND file_id=?2 AND published=1",params![v.id,f.meta.id])?;
                    }
                }
                for (principal,peer,direction) in [(&v.sender_id,&v.recipient_id,"outgoing"),(&v.recipient_id,&v.sender_id,"incoming")] {
                    if !local.contains(principal.as_str()){continue;}
                    let c=if let Some(group)=&v.group_id {Conversation::group(principal,group)}else{Conversation::direct(principal,peer)};
                    tx.execute("INSERT INTO conversations(id,principal_id,kind,peer_id,group_id,last_message_at,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET last_message_at=MAX(last_message_at,excluded.last_message_at)",params![c.id,c.principal_id,c.kind,c.peer_id,c.group_id,v.created_at,c.created_at])?;
                    tx.execute("INSERT OR IGNORE INTO conversation_messages(conversation_id,message_id,direction,read_at) VALUES(?1,?2,?3,?4)",params![c.id,v.message_id,direction,if direction=="outgoing"{Some(now_ms())}else{None}])?;
                }
            }
            for (id,text) in &state.composition {tx.execute("INSERT INTO conversation_drafts VALUES(?1,?2,?3) ON CONFLICT(conversation_id) DO UPDATE SET text=excluded.text,updated_at=excluded.updated_at",params![id,text,now_ms()])?;}
            tx.commit()?;db.saved.extend(changed);Ok(())
        }).await?
    }
    pub async fn load(&self, owner: String) -> anyhow::Result<State> {
        self.run(move |conn,root| {
            let mut state=State::default();
            let mut q=conn.prepare("SELECT id,name,platform,kind,host_device_id,COALESCE(public_key,''),COALESCE(last_address,''),capabilities FROM devices")?;
            for item in q.query_map([],|r|Ok(Device{id:r.get(0)?,name:r.get(1)?,platform:r.get(2)?,kind:r.get(3)?,host_device_id:r.get(4)?,public_key:r.get(5)?,address:r.get(6)?,capabilities:serde_json::from_str(&r.get::<_,String>(7)?).unwrap_or_default(),last_seen:0}))? {
                let d=item?;if d.id!=owner&&d.kind=="app"{state.peers.insert(d.id.clone(),d.clone());}state.profiles.insert(d.id.clone(),d);
            }
            let mut q=conn.prepare("SELECT s.token_hash,s.device_id,d.name,s.expires_at FROM browser_sessions s JOIN devices d ON d.id=s.device_id WHERE expires_at>?1")?;
            for item in q.query_map([now_ms()],|r|Ok((r.get::<_,String>(0)?,Session{id:r.get(1)?,name:r.get(2)?,last_seen:0,expires:r.get::<_,u64>(3)?/1000})))? {let(k,v)=item?;state.sessions.insert(k,v);}
            let mut q=conn.prepare("SELECT id,name,owner_id,revision,created_at,snapshot_json FROM groups")?;
            for item in q.query_map([],|r|Ok(Group{id:r.get(0)?,name:r.get(1)?,owner_id:r.get(2)?,revision:r.get(3)?,created_at:r.get(4)?,snapshot:serde_json::from_str(&r.get::<_,String>(5)?).unwrap_or(None),members:Vec::new()}))? {
                let mut g=item?;let mut m=conn.prepare("SELECT device_id,invite_id,role,state,sequence,event_json FROM group_members WHERE group_id=?1 ORDER BY device_id")?;
                for member in m.query_map([&g.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,u64>(4)?,r.get::<_,Option<String>>(5)?)))? {let(id,invite_id,role,status,sequence,event)=member?;if let Some(d)=state.profiles.get(&id){g.members.push(GroupMember{device:d.clone(),invite_id,role,state:status,sequence,event:event.and_then(|s|serde_json::from_str(&s).ok()).flatten()});}}
                state.groups.insert(g.id.clone(),g);
            }
            for c in query_conversations(conn,None)? {state.conversations.insert(c.id.clone(),c);}
            let mut q=conn.prepare("SELECT conversation_id,text FROM conversation_drafts")?;for item in q.query_map([],|r|Ok((r.get(0)?,r.get(1)?)))? {let(k,v)=item?;state.composition.insert(k,v);}
            let ids:Vec<String>=conn.prepare("SELECT id FROM deliveries ORDER BY updated_at DESC LIMIT 500")?.query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
            for id in ids {if let Some(t)=query_transfer(conn,root,&id)? {state.transfers.insert(id,t);}}
            let paths:Vec<(String,u64,bool)>=conn.prepare("SELECT relative_path,size,EXISTS(SELECT 1 FROM delivery_files f WHERE f.blob_id=file_blobs.id AND f.complete=1) FROM file_blobs")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
            for (path,size,complete) in paths {let absolute=root.join(&path);let _=std::fs::remove_file(absolute.with_extension("part"));if !complete{let _=std::fs::remove_file(&absolute);}
                if absolute.is_file(){state.reserved+=size;}else{conn.execute("UPDATE file_blobs SET state=CASE WHEN state='expired' THEN state ELSE 'missing' END WHERE relative_path=?1",[path])?;}}
            Ok(state)
        }).await
    }
    pub async fn transfer(&self, id: String) -> anyhow::Result<Option<Transfer>> {
        self.run(move |c, r| query_transfer(c, r, &id)).await
    }
    pub async fn message_exists(&self, id: String, recipient: String) -> anyhow::Result<bool> {
        self.run(move |c, _| {
            Ok(c.query_row(
                "SELECT 1 FROM deliveries WHERE message_id=?1 AND recipient_id=?2",
                params![id, recipient],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_some())
        })
        .await
    }
    pub async fn conversations(&self, principal: String) -> anyhow::Result<Vec<ConversationView>> {
        self.run(move |conn,_| {
            let cs=query_conversations(conn,Some(&principal))?;let mut views=Vec::new();
            for c in cs {
                let (name,peer_kind):(String,Option<String>)=if let Some(peer)=&c.peer_id {conn.query_row("SELECT name,kind FROM devices WHERE id=?1",[peer],|r|Ok((r.get(0)?,Some(r.get(1)?))))?}else{(conn.query_row("SELECT name FROM groups WHERE id=?1",[c.group_id.as_ref().unwrap()],|r|r.get(0))?,None)};
                let has_draft:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM conversation_drafts WHERE conversation_id=?1 AND text<>'')",[&c.id],|r|r.get(0))?;
                let latest:Option<(String,u64,u64)>=conn.query_row("SELECT m.text,m.received_at,(SELECT COUNT(*) FROM attachments a WHERE a.message_id=m.id) FROM conversation_messages cm JOIN messages m ON m.id=cm.message_id WHERE cm.conversation_id=?1 ORDER BY cm.position DESC LIMIT 1",[&c.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                let unread:u64=conn.query_row("SELECT COUNT(*) FROM conversation_messages WHERE conversation_id=?1 AND direction='incoming' AND read_at IS NULL",[&c.id],|r|r.get(0))?;
                let (text,at,count)=latest.unwrap_or_default();let preview=if text.is_empty()&&count>0{format!("[文件] {count} 个附件")}else{text.chars().take(100).collect()};
                views.push(ConversationView{conversation:c,name,peer_kind,has_draft,preview,last_message_at:at,unread});
            }
            views.sort_by(|a,b|b.last_message_at.cmp(&a.last_message_at).then(a.conversation.id.cmp(&b.conversation.id)));Ok(views)
        }).await
    }
    pub async fn timeline(
        &self,
        principal: String,
        id: String,
        before: u64,
        limit: usize,
    ) -> anyhow::Result<Vec<MessageView>> {
        self.run(move |conn,root|{
            let authorized=conn.query_row("SELECT 1 FROM conversations WHERE id=?1 AND principal_id=?2",params![id,principal],|r|r.get::<_,i64>(0)).optional()?.is_some();anyhow::ensure!(authorized,"会话不存在或无权访问");
            let mut q=conn.prepare("SELECT cm.position,m.id,m.sender_id,m.sender_name,m.text,m.created_at,cm.direction FROM conversation_messages cm JOIN messages m ON m.id=cm.message_id WHERE cm.conversation_id=?1 AND cm.position<?2 ORDER BY cm.position DESC LIMIT ?3")?;
            let rows:Vec<_>=q.query_map(params![id,before.min(i64::MAX as u64),limit.min(100)],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,u64>(5)?,r.get::<_,String>(6)?)))?.collect::<rusqlite::Result<_>>()?;
            let mut result=Vec::new();
            for(position,id,sender_id,sender_name,text,created_at,direction)in rows{
                let ids:Vec<String>=conn.prepare("SELECT d.id FROM deliveries d JOIN messages m ON m.id=d.message_id WHERE d.message_id=?1 AND (m.sender_id=?2 OR d.recipient_id=?2) ORDER BY CASE WHEN d.status='completed' THEN 0 ELSE 1 END,d.created_at,d.id")?.query_map(params![id,principal],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
                let mut deliveries=Vec::new();let mut files=Vec::new();
                for delivery in ids {if let Some(t)=query_transfer(conn,root,&delivery)?{if files.is_empty(){files=t.files.iter().map(|f|MessageFile{meta:f.meta.clone(),available:f.available()&&(sender_id==principal||t.view.status=="completed"||(t.view.recipient_id==principal&&matches!(t.view.status.as_str(),"accepted"|"transferring"))),transfer_id:t.view.id.clone(),original:matches!(f.storage,FileStorage::Original{..})}).collect();}deliveries.push(t.view);}}
                result.push(MessageView{id,position,sender_id,sender_name,text,created_at,direction,files,deliveries});
            }
            result.reverse();Ok(result)
        }).await
    }
    pub async fn mark_read(
        &self,
        principal: String,
        id: String,
        through: u64,
    ) -> anyhow::Result<()> {
        self.run(move|c,_|{c.execute("UPDATE conversation_messages SET read_at=?1 WHERE read_at IS NULL AND conversation_id=?2 AND position<=?3 AND conversation_id IN (SELECT id FROM conversations WHERE principal_id=?4)",params![now_ms(),id,through,principal])?;c.execute("UPDATE conversations SET last_read_at=?1 WHERE id=?2 AND principal_id=?3",params![now_ms(),id,principal])?;Ok(())}).await
    }
}
fn upsert_device(conn: &Connection, d: &Device) -> anyhow::Result<()> {
    conn.execute("INSERT INTO devices VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10) ON CONFLICT(id) DO UPDATE SET name=excluded.name,platform=excluded.platform,kind=excluded.kind,host_device_id=excluded.host_device_id,public_key=CASE WHEN excluded.public_key='' THEN devices.public_key ELSE excluded.public_key END,last_address=CASE WHEN excluded.last_address='' THEN devices.last_address ELSE excluded.last_address END,capabilities=excluded.capabilities,updated_at=excluded.updated_at,last_seen_at=MAX(last_seen_at,excluded.last_seen_at)",params![d.id,d.name,d.platform,if d.kind=="local"||d.kind=="host"{"app"}else{&d.kind},if d.host_device_id.is_empty(){&d.id}else{&d.host_device_id},d.public_key,serde_json::to_string(&d.capabilities)?,d.address,now_ms(),d.last_seen*1000])?;
    Ok(())
}
fn query_conversations(
    conn: &Connection,
    principal: Option<&str>,
) -> anyhow::Result<Vec<Conversation>> {
    let mut q=conn.prepare("SELECT id,principal_id,kind,peer_id,group_id,last_read_at,created_at FROM conversations WHERE ?1 IS NULL OR principal_id=?1")?;
    let rows = q
        .query_map([principal], |r| {
            Ok(Conversation {
                id: r.get(0)?,
                principal_id: r.get(1)?,
                kind: r.get(2)?,
                peer_id: r.get(3)?,
                group_id: r.get(4)?,
                last_read_at: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
pub(crate) fn query_transfer(
    conn: &Connection,
    root: &Path,
    id: &str,
) -> anyhow::Result<Option<Transfer>> {
    let view=conn.query_row("SELECT d.id,m.id,m.group_id,m.created_at,m.sender_id,m.sender_name,d.recipient_id,d.recipient_name,m.text,d.status,d.bytes,d.total,m.created_at,d.error,d.incoming_remote FROM deliveries d JOIN messages m ON m.id=d.message_id WHERE d.id=?1",[id],|r|Ok((TransferView{id:r.get(0)?,message_id:r.get(1)?,group_id:r.get(2)?,created_at:r.get(3)?,sender_id:r.get(4)?,sender_name:r.get(5)?,recipient_id:r.get(6)?,recipient_name:r.get(7)?,text:r.get(8)?,status:r.get(9)?,bytes:r.get(10)?,total:r.get(11)?,created:r.get::<_,u64>(12)?/1000,error:r.get(13)?,files:Vec::new()},r.get::<_,bool>(14)?))).optional()?;
    let Some((mut view, remote)) = view else {
        return Ok(None);
    };
    let mut q=conn.prepare("SELECT a.file_id,a.name,a.size,a.mime,b.relative_path,f.complete FROM attachments a JOIN delivery_files f ON f.file_id=a.file_id AND f.delivery_id=?1 JOIN file_blobs b ON b.id=f.blob_id WHERE a.message_id=?2 ORDER BY a.position")?;
    let rows: Vec<_> = q
        .query_map(params![id, view.message_id], |r| {
            Ok((
                FileMeta {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    size: r.get(2)?,
                    mime: r.get(3)?,
                },
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut files = Vec::new();
    for (meta, path, complete) in rows {
        anyhow::ensure!(
            Path::new(&path)
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_))),
            "数据库附件路径异常"
        );
        files.push(StoredFile {
            meta,
            path: root.join(path),
            complete,
            uploading: false,
            storage: FileStorage::Cache,
            downloaded: false,
        });
    }
    let mut q = conn.prepare("SELECT source_json FROM file_sources WHERE delivery_id=?1")?;
    for row in q.query_map([id], |r| r.get::<_, String>(0))? {
        let mut file: StoredFile = serde_json::from_str(&row?)?;
        file.uploading = false;
        if file.cached() {
            if file.path.is_relative() {
                anyhow::ensure!(
                    file.path
                        .components()
                        .all(|p| matches!(p, std::path::Component::Normal(_))),
                    "缓存路径异常"
                );
                file.path = root.join(&file.path);
            }
            anyhow::ensure!(file.path.starts_with(root.join("files")), "缓存路径异常");
        }
        if let Some(old) = files.iter_mut().find(|f| f.meta.id == file.meta.id) {
            *old = file;
        } else {
            files.push(file);
        }
    }
    let mut q = conn
        .prepare("SELECT source_json FROM receive_outputs WHERE delivery_id=?1 AND published=1")?;
    for row in q.query_map([id], |r| r.get::<_, String>(0))? {
        let file: StoredFile = serde_json::from_str(&row?)?;
        if let Some(old) = files.iter_mut().find(|f| f.meta.id == file.meta.id) {
            if !old.complete {
                *old = file;
            }
        } else {
            files.push(file);
        }
    }
    let order: Vec<String> = conn
        .prepare("SELECT file_id FROM attachments WHERE message_id=?1 ORDER BY position")?
        .query_map([&view.message_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    files.sort_by_key(|f| {
        order
            .iter()
            .position(|id| id == &f.meta.id)
            .unwrap_or(usize::MAX)
    });
    for f in &mut files {
        if matches!(f.storage, FileStorage::Received { .. }) {
            if let Some((location,directory,name))=conn.query_row("SELECT location,directory,name FROM saved_files WHERE message_id=?1 AND file_id=?2 AND status='saved'",params![view.message_id,f.meta.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()? {
                f.path=location.into();f.storage=FileStorage::Received{directory,name};
            }
        }
    }
    view.files = files.iter().map(|f| f.meta.clone()).collect();
    Ok(Some(Transfer {
        view,
        files,
        peer_token: None,
        incoming_remote: remote,
        reserved: 0,
    }))
}
