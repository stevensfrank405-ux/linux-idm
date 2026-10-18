//! Video pages are resolved by yt-dlp; Rust still owns queueing, lifecycle and UI.
use crate::engine::{safe_name, App, Error};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
};

pub(crate) fn is_media_url(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    [
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "dailymotion.com",
        "twitch.tv",
        "soundcloud.com",
        "facebook.com",
        "instagram.com",
        "tiktok.com",
    ]
    .iter()
    .any(|site| host == *site || host.ends_with(&format!(".{site}")))
        || [".m3u8", ".mpd"]
            .iter()
            .any(|extension| url.path().to_ascii_lowercase().ends_with(extension))
}
fn tool(name: &str) -> PathBuf {
    let executable = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut directories = Vec::new();
    if let Some(path) = std::env::var_os("OPENDOWNLOAD_TOOLS_DIR") {
        directories.push(PathBuf::from(path));
    }
    if let Ok(path) = std::env::current_exe() {
        if let Some(parent) = path.parent() {
            directories.push(parent.join("tools"));
        }
    }
    #[cfg(target_os = "linux")]
    directories.push(PathBuf::from("/usr/lib/opendownload/tools"));
    directories
        .into_iter()
        .map(|path| path.join(&executable))
        .find(|path| path.is_file())
        .unwrap_or_else(|| executable.into())
}
fn background(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW: no console flashing on Windows.
    #[cfg(unix)]
    command.process_group(0);
}
async fn stop(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        {
            let mut command = Command::new("taskkill");
            background(&mut command);
            let _ = command
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{pid}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}
impl App {
    pub(crate) async fn media_transfer(&self, id: &str) -> Result<(), Error> {
        let original = self.data.lock().await.jobs[id].clone();
        let root = original.directory.as_deref().unwrap_or(&self.root);
        let work = root.join(format!(".opendownload-{id}"));
        tokio::fs::create_dir_all(&work).await?;
        let work = tokio::fs::canonicalize(work).await?;
        {
            let mut data = self.data.lock().await;
            let job = data.jobs.get_mut(id).unwrap();
            job.phase = "Extracting video information".into();
            job.segments = 0;
            job.speed = 0.;
            self.save(&data).await?;
        }
        let mut command = Command::new(tool("yt-dlp"));
        background(&mut command);
        command
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.args([
            "--ignore-config",
            "--no-cache-dir",
            "--no-remote-components",
            "--no-playlist",
            "--playlist-items",
            "1",
            "--no-simulate",
            "--newline",
            "--progress",
            "--progress-delta",
            "0.2",
            "--socket-timeout",
            "20",
            "--retries",
            "3",
            "--fragment-retries",
            "3",
            "--continue",
            "--no-overwrites",
            "--no-colors",
            "--windows-filenames",
            "--fixup",
            "never",
            "--match-filters",
            "!is_live",
            "-f",
            "bv*+ba/b",
            "--merge-output-format",
            "mkv",
            "--output",
        ]);
        command.arg(work.join("media.%(ext)s"));
        command.args([
            "--concurrent-fragments",
            &self
                .connections
                .load(std::sync::atomic::Ordering::Relaxed)
                .to_string(),
        ]);
        command.args(["--js-runtimes", &format!("node:{}", tool("node").display())]);
        let ffmpeg = tool("ffmpeg");
        if ffmpeg.is_absolute() {
            command
                .arg("--ffmpeg-location")
                .arg(ffmpeg.parent().unwrap());
        }
        command.args(["--print", "before_dl:__META__%(title)j", "--print", "after_move:__FILE__%(filepath)j", "--progress-template", "download:__PROGRESS__{\"file\":%(progress.filename)j,\"downloaded\":%(progress.downloaded_bytes|0)j,\"total\":%(progress.total_bytes,progress.total_bytes_estimate|0)j,\"speed\":%(progress.speed|0)j}", "--progress-template", "postprocess:__POST__%(progress.status)j", "--", &original.url]);
        let mut child = command.spawn().map_err(|error| format!("Cannot start the video helper ({error}). Install the complete OpenDownload 0.4 package, which includes yt-dlp and Node.js."))?;
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let stderr = child.stderr.take().unwrap();
        // Drain stderr concurrently and retain a bounded tail, including the actual extractor error.
        let errors = tokio::spawn(async move {
            let mut input = BufReader::new(stderr);
            let mut buffer = [0_u8; 4096];
            let mut tail = Vec::new();
            while let Ok(count) = input.read(&mut buffer).await {
                if count == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..count]);
                if tail.len() > 16384 {
                    tail.drain(..tail.len() - 16384);
                }
            }
            String::from_utf8_lossy(&tail).into_owned()
        });
        let mut title = "Video".to_owned();
        let mut final_file = None;
        let mut parts: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        let mut activity = Instant::now();
        let mut saved = Instant::now();
        let mut read_error = None;
        loop {
            if self.data.lock().await.jobs[id].status != "downloading" {
                stop(&mut child).await;
                let _ = errors.await;
                return Ok(());
            }
            let line = tokio::select! {
                value = output.next_line() => value,
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if activity.elapsed() > Duration::from_secs(1200) { read_error = Some("Video helper stopped responding; retry to resume".to_owned()); break; }
                    continue;
                }
            };
            let line = match line {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => {
                    read_error = Some(error.to_string());
                    break;
                }
            };
            activity = Instant::now();
            if let Some(value) = line.strip_prefix("__META__") {
                if let Ok(value) = serde_json::from_str::<String>(value) {
                    title = safe_name(&value);
                }
                let mut data = self.data.lock().await;
                let job = data.jobs.get_mut(id).unwrap();
                job.name = title.clone();
                job.phase = "Downloading video/audio".into();
            } else if let Some(value) = line.strip_prefix("__FILE__") {
                if let Ok(value) = serde_json::from_str::<String>(value) {
                    final_file = Some(PathBuf::from(value));
                }
            } else if let Some(value) = line.strip_prefix("__PROGRESS__") {
                if let Ok(value) = serde_json::from_str::<Value>(value) {
                    let downloaded = value["downloaded"].as_u64().unwrap_or(0);
                    let total = value["total"].as_u64().unwrap_or(0);
                    parts.insert(
                        value["file"].as_str().unwrap_or("media").into(),
                        (downloaded, total),
                    );
                    let mut data = self.data.lock().await;
                    let job = data.jobs.get_mut(id).unwrap();
                    job.downloaded = parts.values().map(|v| v.0).sum();
                    job.total = parts
                        .values()
                        .all(|v| v.1 > 0)
                        .then(|| parts.values().map(|v| v.1).sum());
                    job.speed = value["speed"].as_f64().unwrap_or(0.).max(0.);
                    job.phase = "Downloading video/audio".into();
                    if saved.elapsed() > Duration::from_secs(1) {
                        if let Err(error) = self.save(&data).await {
                            read_error = Some(error.to_string());
                            break;
                        }
                        saved = Instant::now();
                    }
                }
            } else if line.starts_with("__POST__") {
                self.data.lock().await.jobs.get_mut(id).unwrap().phase =
                    "Merging audio and video".into();
            }
        }
        if read_error.is_some() {
            stop(&mut child).await;
        }
        let result = child.wait().await?;
        let diagnostic = errors.await.unwrap_or_default();
        if self.data.lock().await.jobs[id].status != "downloading" {
            return Ok(());
        }
        if let Some(error) = read_error {
            return Err(error.into());
        }
        if !result.success() {
            return Err(format!("Video download failed. {}", diagnostic.trim()).into());
        }
        let file = final_file.ok_or("No downloadable video found. This page may be unsupported, private, restricted, or a live broadcast.")?;
        let file = tokio::fs::canonicalize(file).await?;
        if file.parent() != Some(work.as_path()) {
            return Err("Video helper returned a file outside its download directory".into());
        }
        let extension = file
            .extension()
            .and_then(|v| v.to_str())
            .filter(|v| {
                !v.is_empty() && v.len() < 12 && v.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .ok_or("Invalid video file extension")?;
        if ["html", "htm", "json", "part", "ytdl"].contains(&extension) {
            return Err("The website returned a page instead of video data".into());
        }
        let size = tokio::fs::metadata(&file).await?.len();
        if size == 0 {
            return Err("The video helper produced an empty file".into());
        }
        let mut data = self.data.lock().await;
        if data.jobs[id].status != "downloading" {
            return Ok(());
        }
        let original_name = format!("{title}.{extension}");
        let mut name = original_name.clone();
        let mut number = 1;
        while root.join(&name).exists()
            || data
                .jobs
                .values()
                .any(|job| job.id != id && job.name == name)
        {
            name = format!("{number}-{original_name}");
            number += 1;
        }
        tokio::fs::hard_link(&file, root.join(&name)).await?;
        let job = data.jobs.get_mut(id).unwrap();
        job.name = name;
        job.total = Some(size);
        job.downloaded = size;
        job.speed = 0.;
        job.phase = "Download complete".into();
        job.status = "completed".into();
        self.save(&data).await?;
        drop(data);
        let _ = tokio::fs::remove_dir_all(work).await;
        Ok(())
    }
}
