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

type Error = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Serialize, Deserialize)]
struct Job {
    id: String,
    name: String,
    url: String,
    status: String,
    downloaded: u64,
    total: Option<u64>,
    speed: f64,
    error: String,
    validator: Option<String>,
    #[serde(default)]
    segments: u8,
}
struct Data {
    jobs: BTreeMap<String, Job>,
    active: HashSet<String>,
}
struct App {
    data: Mutex<Data>,
    root: PathBuf,
    state: PathBuf,
    token: String,
    port: u16,
    client: reqwest::Client,
    connections: u8,
}
impl App {
    async fn save(&self, data: &Data) -> Result<(), Error> {
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
        let mut data = self.data.lock().await;
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
        if self.connections > 1 && self.segmented(id).await? {
            return Ok(());
        }
        self.sequential(id).await
    }
    async fn segmented(&self, id: &str) -> Result<bool, Error> {
        let original = self.data.lock().await.jobs[id].clone();
        let probe = self
            .client
            .get(&original.url)
            .header("Accept-Encoding", "identity")
            .header("Range", "bytes=0-0")
            .send()
            .await?
            .error_for_status()?;
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
        let _ = tokio::fs::remove_file(self.root.join(format!("{}.part", original.name))).await;
        let count = self.connections;
        let reuse = original.segments == count
            && original.total == Some(total)
            && original.validator.as_ref() == Some(&validator);
        let mut offsets = Vec::new();
        for part in 0..count {
            let path = self.root.join(format!("{id}.segment-{part}"));
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
                    .open(self.root.join(format!("{id}.segment-{part}")))
                    .await?;
                let mut stream = response.bytes_stream();
                let mut received = offset;
                let mut saved = Instant::now();
                loop {
                    if self.data.lock().await.jobs[id].status != "downloading" {
                        return Ok(true);
                    }
                    let Some(chunk) =
                        tokio::time::timeout(Duration::from_secs(30), stream.next()).await?
                    else {
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
        let partial = self.root.join(format!("{}.part", original.name));
        let mut output = tokio::fs::File::create(&partial).await?;
        for part in 0..count {
            let mut input =
                tokio::fs::File::open(self.root.join(format!("{id}.segment-{part}"))).await?;
            tokio::io::copy(&mut input, &mut output).await?;
        }
        output.sync_all().await?;
        drop(output);
        let mut data = self.data.lock().await;
        if data.jobs[id].status != "downloading" {
            return Ok(true);
        }
        tokio::fs::hard_link(&partial, self.root.join(&original.name)).await?;
        tokio::fs::remove_file(partial).await?;
        data.jobs.get_mut(id).unwrap().status = "completed".into();
        for part in 0..count {
            let _ = tokio::fs::remove_file(self.root.join(format!("{id}.segment-{part}"))).await;
        }
        Ok(true)
    }
    async fn sequential(&self, id: &str) -> Result<(), Error> {
        self.data.lock().await.jobs.get_mut(id).unwrap().segments = 0;
        let job = self.data.lock().await.jobs[id].clone();
        let partial = self.root.join(format!("{}.part", job.name));
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
            let chunk = tokio::time::timeout(Duration::from_secs(30), stream.next()).await?;
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
        tokio::fs::hard_link(&partial, self.root.join(&job.name)).await?;
        tokio::fs::remove_file(partial).await?;
        job.status = "completed".into();
        for part in 0..16 {
            let _ = tokio::fs::remove_file(self.root.join(format!("{id}.segment-{part}"))).await;
        }
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
    url: String,
}
#[derive(Deserialize)]
struct Action {
    id: String,
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
    let name: String = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("download")
        .chars()
        .filter(|c| !c.is_control() && !['/', '\\'].contains(c))
        .take(160)
        .collect();
    let name = if name.is_empty() || name == "." || name == ".." {
        "download".into()
    } else {
        name
    };
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
        },
    );
    if let Err(error) = app.save(&data).await {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    drop(data);
    match app.resume(&id).await {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
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
#[tokio::main]
async fn main() -> Result<(), Error> {
    let home = PathBuf::from(std::env::var("HOME")?);
    let mut root = home.join("Downloads/OpenDownload");
    let mut state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("opendownload/queue.json");
    let mut port = 8765;
    let mut connections: u8 = 4;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("Expected argument value")?;
        match arg.as_str() {
            "--downloads" => root = value.into(),
            "--state" => state = value.into(),
            "--port" => port = value.parse()?,
            "--connections" => {
                connections = value.parse()?;
                if !(1..=16).contains(&connections) {
                    return Err("Connections must be 1 through 16".into());
                }
            }
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
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
    }
    let app = Arc::new(App {
        data: Mutex::new(Data {
            jobs,
            active: HashSet::new(),
        }),
        root,
        state,
        port,
        connections,
        token: uuid::Uuid::new_v4().to_string(),
        client: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(30))
            .user_agent("OpenDownload/0.2")
            .build()?,
    });
    let router = Router::new()
        .route("/", get(page))
        .route("/api/downloads", get(list))
        .route("/api/add", post(add))
        .route("/api/pause", post(pause))
        .route("/api/resume", post(resume))
        .layer(axum::extract::DefaultBodyLimit::max(16384))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!("OpenDownload running on port {port}");
    axum::serve(listener, router).await?;
    Ok(())
}
