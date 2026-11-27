use crate::{
    api::{attachment, token, validate_files, Error, Result},
    model::*,
    receive_conflicts::{name_key, numbered, ReceiveTarget},
    receiving::safe_name,
    App,
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{io::AsyncReadExt, sync::Semaphore, time::Instant};
use tokio_util::{io::ReaderStream, sync::CancellationToken};

const MAX_ACTIVE: usize = 16;
const MAX_PAGES: usize = 64;
const MAX_SUBMISSIONS: usize = 256;

pub(crate) struct Pages {
    pages: Mutex<HashMap<String, Arc<Page>>>,
    uploads: Arc<Semaphore>,
    planning: tokio::sync::Mutex<()>,
    targets: Mutex<HashSet<(String, String)>>,
}

impl Default for Pages {
    fn default() -> Self {
        Self {
            pages: Mutex::default(),
            uploads: Arc::new(Semaphore::new(4)),
            planning: tokio::sync::Mutex::default(),
            targets: Mutex::default(),
        }
    }
}

impl Pages {
    fn get(&self, id: &str) -> Result<Arc<Page>> {
        self.pages
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(inactive_error)
    }

    pub(crate) fn sweep(&self) {
        self.pages
            .lock()
            .unwrap()
            .retain(|_, page| now().saturating_sub(page.view.expires_at) < RETENTION);
    }

    async fn claim_target(
        &self,
        app: &App,
        directory: &str,
        name: &str,
    ) -> Result<(ReceiveTarget, TargetClaim<'_>)> {
        let original = safe_name(name);
        let _planning = self.planning.lock().await;
        for n in 0..10_000 {
            let name = if n == 0 {
                original.clone()
            } else {
                numbered(&original, n)
            };
            let key = (directory.to_owned(), name_key(&name));
            if self.targets.lock().unwrap().contains(&key)
                || app.inspect_entry(directory, &name).await?.is_some()
            {
                continue;
            }
            self.targets.lock().unwrap().insert(key.clone());
            return Ok((
                ReceiveTarget {
                    directory: directory.into(),
                    name,
                    replace: None,
                    previous_location: None,
                },
                TargetClaim { pages: self, key },
            ));
        }
        Err(Error::conflict("同名文件过多，请更换接收目录"))
    }
}

struct TargetClaim<'a> {
    pages: &'a Pages,
    key: (String, String),
}

impl Drop for TargetClaim<'_> {
    fn drop(&mut self) {
        self.pages.targets.lock().unwrap().remove(&self.key);
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Pickup,
    Dropoff,
}

#[derive(Deserialize)]
pub(crate) struct Create {
    kind: Kind,
    title: String,
    #[serde(default)]
    note: String,
    minutes: u64,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    links: Vec<String>,
}

#[derive(Clone, Serialize)]
pub(crate) struct View {
    kind: Kind,
    title: String,
    note: String,
    expires_at: u64,
    files: Vec<FileMeta>,
    links: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct OwnerView {
    #[serde(flatten)]
    page: View,
    id: String,
    active: bool,
    created_at: u64,
    directory: Option<String>,
    submissions: Vec<Receipt>,
}

#[derive(Clone, Serialize)]
pub(crate) struct Receipt {
    name: String,
    sender: String,
    size: u64,
    submitted_at: u64,
}

pub(crate) struct Page {
    id: String,
    view: View,
    created_at: u64,
    deadline: Instant,
    closed: CancellationToken,
    files: Vec<StoredFile>,
    directory: Option<String>,
    submissions: Mutex<HashMap<String, Option<Receipt>>>,
}

pub(crate) fn inactive_error() -> Error {
    Error::gone("此扫码页面已关闭或已过期")
}

impl Page {
    pub(crate) fn ensure_active(&self) -> Result<()> {
        if self.closed.is_cancelled() || Instant::now() >= self.deadline {
            return Err(inactive_error());
        }
        Ok(())
    }

    pub(crate) async fn inactive(&self) {
        tokio::select! {
            _ = self.closed.cancelled() => {},
            _ = tokio::time::sleep_until(self.deadline) => {},
        }
    }

    fn owner_view(&self) -> OwnerView {
        let mut submissions: Vec<_> = self
            .submissions
            .lock()
            .unwrap()
            .values()
            .filter_map(Clone::clone)
            .collect();
        submissions.sort_by_key(|entry| std::cmp::Reverse(entry.submitted_at));
        OwnerView {
            page: self.view.clone(),
            id: self.id.clone(),
            active: self.ensure_active().is_ok(),
            created_at: self.created_at,
            directory: self.directory.clone(),
            submissions,
        }
    }
}

async fn owner(app: &App, headers: &HeaderMap) -> Result<()> {
    if !app.principal(token(headers)?).await?.owner {
        return Err(Error::forbidden());
    }
    Ok(())
}

pub(crate) async fn list(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<Vec<OwnerView>>> {
    owner(&app, &headers).await?;
    let pages: Vec<_> = app.0.scan.pages.lock().unwrap().values().cloned().collect();
    let mut pages: Vec<_> = pages.iter().map(|page| page.owner_view()).collect();
    pages.sort_by_key(|page| std::cmp::Reverse(page.created_at));
    Ok(Json(pages))
}

pub(crate) async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<Create>,
) -> Result<Json<OwnerView>> {
    owner(&app, &headers).await?;
    let title = crate::clean_name(&request.title).map_err(|e| Error::bad(e.to_string()))?;
    let note = request.note.trim().to_owned();
    if note.chars().count() > 2000 || !(1..=1440).contains(&request.minutes) {
        return Err(Error::bad("说明或有效期超出范围"));
    }
    if request.sources.len() > 64 || request.links.len() > 12 {
        return Err(Error::bad("每页最多添加 64 个文件和 12 个链接"));
    }
    let links: Vec<_> = request
        .links
        .iter()
        .map(|link| {
            url::Url::parse(link.trim())
                .ok()
                .filter(|parsed| {
                    matches!(parsed.scheme(), "http" | "https")
                        && parsed.host_str().is_some()
                        && parsed.username().is_empty()
                        && parsed.password().is_none()
                        && parsed.as_str().len() <= 2048
                })
                .map(|parsed| parsed.to_string())
                .ok_or_else(|| Error::bad("请输入有效的 HTTP 或 HTTPS 链接"))
        })
        .collect::<Result<_>>()?;
    let (files, directory) = match request.kind {
        Kind::Pickup => {
            if request.sources.is_empty() && links.is_empty() {
                return Err(Error::bad("请至少添加一个文件或链接"));
            }
            let sources = app.0.sources.lock().unwrap();
            let files = request
                .sources
                .iter()
                .map(|id| {
                    let mut file = sources
                        .get(id)
                        .filter(|(_, registered)| now().saturating_sub(*registered) < RETENTION)
                        .map(|(file, _)| file.clone())
                        .ok_or_else(|| Error::bad("原文件引用已失效，请重新选择"))?;
                    file.meta.id = uid();
                    Ok(file)
                })
                .collect::<Result<Vec<_>>>()?;
            (files, None)
        }
        Kind::Dropoff => {
            if !request.sources.is_empty() || !links.is_empty() {
                return Err(Error::bad("交件页只能收集文件"));
            }
            (Vec::new(), Some(app.target_directory().await?))
        }
    };
    let duration = Duration::from_secs(request.minutes * 60);
    let page = Arc::new(Page {
        id: secret(),
        view: View {
            kind: request.kind,
            title,
            note,
            expires_at: now() + duration.as_secs(),
            files: files.iter().map(|file| file.meta.clone()).collect(),
            links,
        },
        created_at: now_ms(),
        deadline: Instant::now() + duration,
        closed: CancellationToken::new(),
        files,
        directory,
        submissions: Mutex::default(),
    });
    let mut pages = app.0.scan.pages.lock().unwrap();
    if pages
        .values()
        .filter(|page| page.ensure_active().is_ok())
        .count()
        >= MAX_ACTIVE
    {
        return Err(Error::conflict("有效页面已达上限，请先关闭不用的页面"));
    }
    if pages.len() >= MAX_PAGES {
        if let Some(id) = pages
            .values()
            .filter(|page| page.ensure_active().is_err())
            .min_by_key(|page| page.created_at)
            .map(|page| page.id.clone())
        {
            pages.remove(&id);
        }
    }
    let view = page.owner_view();
    pages.insert(page.id.clone(), page);
    Ok(Json(view))
}

pub(crate) async fn close(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<OwnerView>> {
    owner(&app, &headers).await?;
    let page = app.0.scan.get(&id)?;
    page.closed.cancel();
    Ok(Json(page.owner_view()))
}

impl App {
    pub fn scan_receive_directory(&self, id: &str) -> std::result::Result<String, String> {
        self.0
            .scan
            .get(id)
            .map_err(|e| e.to_string())?
            .directory
            .clone()
            .ok_or_else(|| "找不到收件目录".into())
    }
}

pub(crate) async fn visit(State(app): State<App>, Path(key): Path<String>) -> Result<Json<View>> {
    let page = app.0.scan.get(&key)?;
    page.ensure_active()?;
    Ok(Json(page.view.clone()))
}

pub(crate) async fn download(
    State(app): State<App>,
    Path((key, id)): Path<(String, String)>,
) -> Result<Response> {
    let page = app.0.scan.get(&key)?;
    page.ensure_active()?;
    let file = page
        .files
        .iter()
        .find(|file| file.meta.id == id)
        .ok_or_else(Error::missing)?;
    let (name, length) = (file.meta.name.clone(), file.meta.size);
    let permit = app
        .0
        .streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::conflict("同时下载过多，请稍后重试"))?;
    let input = app.open_source(file).await.map_err(Error::bad)?;
    page.ensure_active()?;
    let stream = futures_util::stream::try_unfold(
        (ReaderStream::new(input.take(length)), page, permit),
        |(mut reader, page, permit)| async move {
            let chunk = tokio::select! {
                biased;
                _ = page.inactive() => return Err(std::io::Error::other("取件页已关闭")),
                chunk = reader.next() => chunk,
            };
            Ok(chunk
                .transpose()?
                .map(|chunk| (chunk, (reader, page, permit))))
        },
    );
    attachment(Body::from_stream(stream), length, &name)
}

#[derive(Deserialize)]
pub(crate) struct Submission {
    name: String,
    size: u64,
    #[serde(default)]
    sender: String,
}

struct Pending {
    page: Arc<Page>,
    id: String,
}

impl Drop for Pending {
    fn drop(&mut self) {
        let mut submissions = self.page.submissions.lock().unwrap();
        if submissions.get(&self.id).is_some_and(Option::is_none) {
            submissions.remove(&self.id);
        }
    }
}

pub(crate) async fn submit(
    State(app): State<App>,
    Path((key, id)): Path<(String, String)>,
    Query(request): Query<Submission>,
    body: Body,
) -> Result<Json<Receipt>> {
    let page = app.0.scan.get(&key)?;
    page.ensure_active()?;
    let directory = page.directory.clone().ok_or_else(Error::missing)?;
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::bad("交件编号无效"));
    }
    let sender = if request.sender.trim().is_empty() {
        "匿名".into()
    } else {
        crate::clean_name(&request.sender).map_err(|e| Error::bad(e.to_string()))?
    };
    let mime = mime_guess::from_path(&request.name)
        .first_or_octet_stream()
        .to_string();
    let input = FileInput {
        name: request.name,
        size: request.size,
        mime,
        source_id: None,
    };
    validate_files(std::slice::from_ref(&input))?;
    let permit = app
        .0
        .scan
        .uploads
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::conflict("同时交件人数较多，请稍后重试"))?;
    let stream = app
        .0
        .streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::conflict("同时传输过多，请稍后重试"))?;
    {
        let mut submissions = page.submissions.lock().unwrap();
        if let Some(receipt) = submissions.get(&id) {
            return receipt
                .clone()
                .map(Json)
                .ok_or_else(|| Error::conflict("此文件正在提交"));
        }
        if submissions.len() >= MAX_SUBMISSIONS {
            return Err(Error::conflict("此页面已达到收件上限"));
        }
        page.ensure_active()?;
        submissions.insert(id.clone(), None);
    }
    let pending = Pending { page, id };
    // Keep the destination cleanup running when the browser aborts its HTTP request.
    let receipt = tokio::spawn(async move {
        let (_permit, _stream) = (permit, stream);
        let (target, _claim) = app
            .0
            .scan
            .claim_target(&app, &directory, &input.name)
            .await?;
        let id = uid();
        let reservation = app
            .0
            .space
            .reserve(vec![(id.clone(), directory.clone(), input.size)])
            .await
            .map_err(Error::conflict)?;
        let progress = reservation.progress(&id, &directory);
        pending.page.ensure_active()?;
        let file = StoredFile {
            receive_target: Some(target),
            meta: FileMeta {
                id,
                name: input.name,
                size: input.size,
                mime: input.mime,
            },
            path: Default::default(),
            complete: false,
            uploading: true,
            storage: FileStorage::Receiving,
            downloaded: false,
        };
        let saved = app
            .receive_scan_file(&pending.page, &file, body, progress)
            .await?;
        let FileStorage::Received { name, .. } = saved.storage else {
            return Err(Error::storage("接收位置未记录"));
        };
        let receipt = Receipt {
            name,
            sender,
            size: input.size,
            submitted_at: now_ms(),
        };
        pending
            .page
            .submissions
            .lock()
            .unwrap()
            .insert(pending.id.clone(), Some(receipt.clone()));
        Ok(receipt)
    })
    .await
    .map_err(|e| Error::storage(e.to_string()))??;
    Ok(Json(receipt))
}
