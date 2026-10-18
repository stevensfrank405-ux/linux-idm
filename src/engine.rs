use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, sync::Mutex};

pub(crate) type Error = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Job {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) status: String,
    pub(crate) downloaded: u64,
    pub(crate) total: Option<u64>,
    pub(crate) speed: f64,
    pub(crate) error: String,
    validator: Option<String>,
    #[serde(default)]
    pub(crate) segments: u8,
    #[serde(default)]
    pub(crate) directory: Option<PathBuf>,
    #[serde(default)]
    pub(crate) media: bool,
    #[serde(default)]
    pub(crate) allow_html: bool,
    #[serde(default)]
    pub(crate) phase: String,
}
pub(crate) struct Data {
    pub(crate) jobs: BTreeMap<String, Job>,
    active: HashSet<String>,
}
pub(crate) struct App {
    pub(crate) data: Mutex<Data>,
    pub(crate) root: PathBuf,
    state: PathBuf,
    pub(crate) token: String,
    pub(crate) port: u16,
    pub(crate) client: reqwest::Client,
    pub(crate) connections: std::sync::atomic::AtomicU8,
}
impl App {
    pub(crate) async fn save(&self, data: &Data) -> Result<(), Error> {
        let temporary = self.state.with_extension("tmp");
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).await?;
        file.write_all(&serde_json::to_vec(&data.jobs)?).await?;
        file.sync_all().await?;
        tokio::fs::rename(temporary, &self.state).await?;
        Ok(())
    }
    async fn resume(self: &Arc<Self>, id: &str) -> Result<(), Error> {
        // A pause cancels workers asynchronously (including the Windows process tree).
        // Accept a fast resume only after those workers release their partial files.
        let mut data = loop {
            let data = self.data.lock().await;
            let stopping = data.active.contains(id)
                && data.jobs.get(id).is_some_and(|job| job.status == "paused");
            if !stopping {
                break data;
            }
            drop(data);
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        if data.active.contains(id) {
            return Ok(());
        }
        let job = data.jobs.get_mut(id).ok_or("Unknown download")?;
        if job.status == "completed" {
            return Ok(());
        }
        job.status = "downloading".into();
        job.error.clear();
        job.speed = 0.;
        job.phase.clear();
        self.save(&data).await?;
        data.active.insert(id.into());
        let app = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let result = app.transfer(&id).await;
            let mut data = app.data.lock().await;
            if let Some(job) = data.jobs.get_mut(&id) {
                if let Err(error) = result {
                    if job.status == "downloading" {
                        job.status = "failed".into();
                        job.error = error.to_string();
                    }
                }
                job.speed = 0.;
            }
            data.active.remove(&id);
            if let Err(error) = app.save(&data).await {
                eprintln!("Queue persistence failed: {error}");
            }
        });
        Ok(())
    }
    async fn transfer(&self, id: &str) -> Result<(), Error> {
        if self.data.lock().await.jobs[id].media {
            return self.media_transfer(id).await;
        }
        if self.connections.load(std::sync::atomic::Ordering::Relaxed) > 1
            && self.segmented(id).await?
        {
            return Ok(());
        }
        if self.data.lock().await.jobs[id].media {
            return self.media_transfer(id).await;
        }
        self.sequential(id).await
    }
    async fn segmented(&self, id: &str) -> Result<bool, Error> {
        let original = self.data.lock().await.jobs[id].clone();
        let root = original.directory.as_deref().unwrap_or(&self.root);
        let probe = self
            .client
            .get(&original.url)
            .header("Accept-Encoding", "identity")
            .header("Range", "bytes=0-0")
            .send()
            .await?
            .error_for_status()?;
        if is_html(&probe) && !original.allow_html {
            self.data.lock().await.jobs.get_mut(id).unwrap().media = true;
            return Ok(false);
        }
        if probe.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Ok(false);
        }
        let range = probe
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let total = range
            .strip_prefix("bytes 0-0/")
            .and_then(|v| v.parse::<u64>().ok());
        // Strong validators prevent combining bytes from different file versions.
        let validator = probe
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.starts_with('"') && v.ends_with('"'))
            .map(str::to_owned);
        let (Some(total), Some(validator)) = (total, validator) else {
            return Ok(false);
        };
        if total < 1024 * 1024 {
            return Ok(false);
        }
        drop(probe);
        // A previous sequential partial cannot be reused after adopting a new validator.
        let _ = tokio::fs::remove_file(root.join(format!("{}.part", original.name))).await;
        let count = self.connections.load(std::sync::atomic::Ordering::Relaxed);
        let reuse = original.segments == count
            && original.total == Some(total)
            && original.validator.as_ref() == Some(&validator);
        let mut offsets = Vec::new();
        for part in 0..count {
            let path = root.join(format!("{id}.segment-{part}"));
            let length = total * (u64::from(part) + 1) / u64::from(count)
                - total * u64::from(part) / u64::from(count);
            let existing = tokio::fs::metadata(&path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            if !reuse || existing > length {
                let _ = tokio::fs::remove_file(&path).await;
                offsets.push(0);
            } else {
                offsets.push(existing);
            }
        }
        {
            let mut data = self.data.lock().await;
            let job = data.jobs.get_mut(id).unwrap();
            job.total = Some(total);
            job.validator = Some(validator.clone());
            job.segments = count;
            job.downloaded = offsets.iter().sum();
            self.save(&data).await?;
        }
        let initial: u64 = offsets.iter().sum();
        let started = Instant::now();
        let source_url = &original.url;
        let futures = offsets.into_iter().enumerate().map(|(part, offset)| {
            let validator = &validator;
            async move {
                let begin = total * part as u64 / u64::from(count);
                let end = total * (part as u64 + 1) / u64::from(count) - 1;
                if offset == end - begin + 1 {
                    return Ok::<bool, Error>(true);
                }
                let response = self
                    .client
                    .get(source_url)
                    .header("Accept-Encoding", "identity")
                    .header("Range", format!("bytes={}-{end}", begin + offset))
                    .header("If-Range", validator)
                    .send()
                    .await?
                    .error_for_status()?;
                let expected = format!("bytes {}-{end}/{total}", begin + offset);
                if response.headers().get("etag").and_then(|v| v.to_str().ok())
                    != Some(validator.as_str())
                    || response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                    || response
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        != Some(expected.as_str())
                {
                    return Ok(false);
                }
                let mut output = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(root.join(format!("{id}.segment-{part}")))
                    .await?;
                let mut stream = response.bytes_stream();
                let mut received = offset;
                let mut saved = Instant::now();
                loop {
                    if self.data.lock().await.jobs[id].status != "downloading" {
                        return Ok(true);
                    }
                    let next = tokio::select! { next = stream.next() => next, _ = tokio::time::sleep(Duration::from_millis(100)) => continue };
                    let Some(chunk) = next else {
                        break;
                    };
                    let chunk = chunk?;
                    if received + chunk.len() as u64 > end - begin + 1 {
                        return Err("Segment exceeded its range".into());
                    }
                    output.write_all(&chunk).await?;
                    received += chunk.len() as u64;
                    let mut data = self.data.lock().await;
                    let job = data.jobs.get_mut(id).unwrap();
                    job.downloaded += chunk.len() as u64;
                    job.speed = (job.downloaded - initial) as f64
                        / started.elapsed().as_secs_f64().max(0.001);
                    if saved.elapsed() > Duration::from_secs(1) {
                        self.save(&data).await?;
                        saved = Instant::now();
                    }
                }
                output.sync_all().await?;
                if received != end - begin + 1 {
                    return Err("Incomplete segment; retry to resume".into());
                }
                Ok(true)
            }
        });
        // Wait for every worker before returning, including on errors or fallback.
        let results = futures_util::future::join_all(futures).await;
        let mut supported = true;
        for result in results {
            supported &= result?;
        }
        if self.data.lock().await.jobs[id].status != "downloading" {
            return Ok(true);
        }
        if !supported {
            return Ok(false);
        }
        let partial = root.join(format!("{}.part", original.name));
        let mut output = tokio::fs::File::create(&partial).await?;
        for part in 0..count {
            let mut input =
                tokio::fs::File::open(root.join(format!("{id}.segment-{part}"))).await?;
            tokio::io::copy(&mut input, &mut output).await?;
        }
        output.sync_all().await?;
        drop(output);
        let mut data = self.data.lock().await;
        if data.jobs[id].status != "downloading" {
            return Ok(true);
        }
        tokio::fs::hard_link(&partial, root.join(&original.name)).await?;
        tokio::fs::remove_file(partial).await?;
        let job = data.jobs.get_mut(id).unwrap();
        job.status = "completed".into();
        job.speed = 0.;
        for part in 0..count {
            let _ = tokio::fs::remove_file(root.join(format!("{id}.segment-{part}"))).await;
        }
        // Readers must never observe completion before its durable checkpoint.
        self.save(&data).await?;
        Ok(true)
    }
    async fn sequential(&self, id: &str) -> Result<(), Error> {
        self.data.lock().await.jobs.get_mut(id).unwrap().segments = 0;
        let job = self.data.lock().await.jobs[id].clone();
        let root = job.directory.as_deref().unwrap_or(&self.root);
        let partial = root.join(format!("{}.part", job.name));
        let mut offset = tokio::fs::metadata(&partial)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let mut request = self
            .client
            .get(&job.url)
            .header("Accept-Encoding", "identity");
        if offset > 0 {
            if let Some(validator) = &job.validator {
                request = request
                    .header("Range", format!("bytes={offset}-"))
                    .header("If-Range", validator);
            } else {
                offset = 0;
            }
        }
        let response = request.send().await?.error_for_status()?;
        if is_html(&response) && !job.allow_html {
            self.data.lock().await.jobs.get_mut(id).unwrap().media = true;
            return self.media_transfer(id).await;
        }
        let total = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let range = response
                .headers()
                .get("content-range")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");
            let total = range
                .rsplit('/')
                .next()
                .and_then(|n| n.parse::<u64>().ok())
                .ok_or("Invalid resume total")?;
            if total == 0 || range != format!("bytes {offset}-{}/{total}", total - 1) {
                return Err("Invalid resume range".into());
            }
            Some(total)
        } else {
            offset = 0;
            response.content_length()
        };
        let validator = response
            .headers()
            .get("etag")
            .or_else(|| response.headers().get("last-modified"))
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        {
            let mut data = self.data.lock().await;
            let job = data.jobs.get_mut(id).unwrap();
            job.total = total;
            job.downloaded = offset;
            job.validator = validator;
        }
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(offset > 0)
            .truncate(offset == 0)
            .open(&partial)
            .await?;
        let started = Instant::now();
        let mut saved = Instant::now();
        let mut received = 0;
        let mut stream = response.bytes_stream();
        loop {
            if self.data.lock().await.jobs[id].status != "downloading" {
                return Ok(());
            }
            let chunk = tokio::select! { next = stream.next() => next, _ = tokio::time::sleep(Duration::from_millis(100)) => continue };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk?;
            output.write_all(&chunk).await?;
            received += chunk.len() as u64;
            let mut data = self.data.lock().await;
            let job = data.jobs.get_mut(id).unwrap();
            job.downloaded = offset + received;
            job.speed = received as f64 / started.elapsed().as_secs_f64().max(0.001);
            if saved.elapsed() > Duration::from_secs(1) {
                self.save(&data).await?;
                saved = Instant::now();
            }
        }
        output.flush().await?;
        output.sync_all().await?;
        drop(output);
        let mut data = self.data.lock().await;
        let job = data.jobs.get_mut(id).unwrap();
        if job.status != "downloading" {
            return Ok(());
        }
        if total.is_some_and(|n| n != job.downloaded) {
            return Err("Incomplete transfer; retry to resume".into());
        }
        tokio::fs::hard_link(&partial, root.join(&job.name)).await?;
        tokio::fs::remove_file(partial).await?;
        job.status = "completed".into();
        job.speed = 0.;
        for part in 0..16 {
            let _ = tokio::fs::remove_file(root.join(format!("{id}.segment-{part}"))).await;
        }
        self.save(&data).await?;
        Ok(())
    }
}
fn authorized(app: &App, headers: &HeaderMap, token: bool) -> bool {
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    (host == format!("127.0.0.1:{}", app.port) || host == format!("localhost:{}", app.port))
        && (!token || headers.get("x-token").and_then(|h| h.to_str().ok()) == Some(&app.token))
}
fn is_html(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            let mime = v.split(';').next().unwrap_or("").trim();
            mime.eq_ignore_ascii_case("text/html")
                || mime.eq_ignore_ascii_case("application/xhtml+xml")
        })
}
pub(crate) fn safe_name(value: &str) -> String {
    let name: String = value
        .chars()
        .filter(|c| !c.is_control() && !['/', '\\', ':', '*', '?', '"', '<', '>', '|'].contains(c))
        .scan(0, |length, character| {
            *length += character.len_utf8();
            (*length <= 160).then_some(character)
        })
        .collect();
    let name = name.trim_end_matches(['.', ' ']);
    if name.is_empty() {
        return "download".into();
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        format!("download-{name}")
    } else {
        name.into()
    }
}
fn failure(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({"error": message.to_string()})),
    )
        .into_response()
}
async fn page(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !authorized(&app, &headers, false) {
        return failure(StatusCode::FORBIDDEN, "Invalid host");
    }
    let mut response =
        Html(include_str!("../index.html").replace("__TOKEN__", &app.token)).into_response();
    response
        .headers_mut()
        .insert("x-frame-options", "DENY".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}
async fn list(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !authorized(&app, &headers, true) {
        return failure(StatusCode::FORBIDDEN, "Unauthorized");
    }
    Json(
        app.data
            .lock()
            .await
            .jobs
            .values()
            .cloned()
            .collect::<Vec<_>>(),
    )
    .into_response()
}
#[derive(Deserialize)]
struct Add {
    pub(crate) url: String,
    #[serde(default)]
    kind: String,
}
#[derive(Deserialize)]
struct Action {
    pub(crate) id: String,
}
async fn add(State(app): State<Arc<App>>, headers: HeaderMap, Json(input): Json<Add>) -> Response {
    if !authorized(&app, &headers, true) {
        return failure(StatusCode::FORBIDDEN, "Unauthorized");
    }
    let url = match reqwest::Url::parse(&input.url) {
        Ok(url)
            if ["http", "https"].contains(&url.scheme())
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            url
        }
        _ => {
            return failure(
                StatusCode::BAD_REQUEST,
                "Enter an HTTP(S) URL without credentials",
            )
        }
    };
    if !["", "auto", "video", "file"].contains(&input.kind.as_str()) {
        return failure(
            StatusCode::BAD_REQUEST,
            "Choose auto, video, or file download mode",
        );
    }
    let name = safe_name(
        url.path_segments()
            .and_then(|mut s| s.next_back())
            .unwrap_or("download"),
    );
    let media = input.kind == "video" || (input.kind != "file" && crate::media::is_media_url(&url));
    let mut data = app.data.lock().await;
    let original = name.clone();
    let mut name = name;
    let mut n = 1;
    while data.jobs.values().any(|j| j.name == name)
        || app.root.join(&name).exists()
        || app.root.join(format!("{name}.part")).exists()
    {
        name = format!("{n}-{original}");
        n += 1;
    }
    let id = uuid::Uuid::new_v4().to_string();
    data.jobs.insert(
        id.clone(),
        Job {
            id: id.clone(),
            name,
            url: url.to_string(),
            status: "paused".into(),
            downloaded: 0,
            total: None,
            speed: 0.,
            error: String::new(),
            validator: None,
            segments: 0,
            directory: Some(app.root.clone()),
            media,
            allow_html: input.kind == "file",
            phase: String::new(),
        },
    );
    if let Err(error) = app.save(&data).await {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    drop(data);
    match app.resume(&id).await {
        Ok(()) => Json(serde_json::json!({"ok":true,"id":id})).into_response(),
        Err(error) => failure(StatusCode::BAD_REQUEST, error),
    }
}
async fn pause(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> Response {
    if !authorized(&app, &headers, true) {
        return failure(StatusCode::FORBIDDEN, "Unauthorized");
    }
    let mut data = app.data.lock().await;
    let Some(job) = data.jobs.get_mut(&input.id) else {
        return failure(StatusCode::NOT_FOUND, "Unknown download");
    };
    if job.status == "downloading" {
        job.status = "paused".into();
        job.speed = 0.;
    }
    match app.save(&data).await {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(error) => failure(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}
async fn resume(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> Response {
    if !authorized(&app, &headers, true) {
        return failure(StatusCode::FORBIDDEN, "Unauthorized");
    }
    match app.resume(&input.id).await {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(error) => failure(StatusCode::BAD_REQUEST, error),
    }
}
pub(crate) async fn start(
    root: PathBuf,
    state: PathBuf,
    port: u16,
    connections: u8,
) -> Result<Arc<App>, Error> {
    // Bind before touching queue state so a second instance cannot become another writer.
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    tokio::fs::create_dir_all(&root).await?;
    if let Some(parent) = state.parent().filter(|p| !p.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut jobs: BTreeMap<String, Job> = match tokio::fs::read(&state).await {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(e.into()),
    };
    for job in jobs.values_mut() {
        if job.status == "downloading" {
            job.status = "paused".into();
        }
        job.speed = 0.;
        if job.directory.is_none() {
            job.directory = Some(root.clone());
        }
    }
    let token_path = state.with_extension("bridge-key");
    let token = match tokio::fs::read_to_string(&token_path).await {
        Ok(value) => {
            uuid::Uuid::parse_str(value.trim())?;
            value.trim().to_owned()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let value = uuid::Uuid::new_v4().to_string();
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&token_path).await?;
            file.write_all(value.as_bytes()).await?;
            value
        }
        Err(e) => return Err(e.into()),
    };
    let app = Arc::new(App {
        data: Mutex::new(Data {
            jobs,
            active: HashSet::new(),
        }),
        root,
        state,
        port,
        connections: std::sync::atomic::AtomicU8::new(connections),
        token,
        client: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(30))
            .user_agent("OpenDownload/0.4")
            .build()?,
    });
    {
        let data = app.data.lock().await;
        app.save(&data).await?;
    }
    let router = Router::new()
        .route("/", get(page))
        .route("/api/downloads", get(list))
        .route("/api/add", post(add))
        .route("/api/pause", post(pause))
        .route("/api/resume", post(resume))
        .route("/api/remove", post(remove))
        .layer(axum::extract::DefaultBodyLimit::max(16384))
        .with_state(app.clone());
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("Browser bridge stopped: {error}");
        }
    });
    println!("OpenDownload bridge listening on port {port}");
    Ok(app)
}

async fn remove(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<Action>,
) -> Response {
    if !authorized(&app, &headers, true) {
        return failure(StatusCode::FORBIDDEN, "Unauthorized");
    }
    let mut data = app.data.lock().await;
    if !data
        .jobs
        .get(&input.id)
        .is_some_and(|j| j.status == "completed")
    {
        return failure(
            StatusCode::BAD_REQUEST,
            "Only completed entries can be removed; files are kept",
        );
    }
    data.jobs.remove(&input.id);
    match app.save(&data).await {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(error) => failure(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

impl App {
    pub(crate) fn snapshot(&self) -> Option<(Vec<Job>, HashSet<String>)> {
        self.data
            .try_lock()
            .ok()
            .map(|data| (data.jobs.values().cloned().collect(), data.active.clone()))
    }
    pub(crate) async fn pause_all(&self) -> Result<(), Error> {
        let mut data = self.data.lock().await;
        for job in data.jobs.values_mut() {
            if job.status == "downloading" {
                job.status = "paused".into();
                job.speed = 0.;
            }
        }
        self.save(&data).await?;
        drop(data);
        for _ in 0..30 {
            if self.data.lock().await.active.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }
}
