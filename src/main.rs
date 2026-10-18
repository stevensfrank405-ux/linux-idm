#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod desktop;
mod engine;
mod media;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
struct Preferences {
    downloads: PathBuf,
    connections: u8,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            downloads: dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("OpenDownload"),
            connections: 4,
        }
    }
}
fn run() -> Result<(), engine::Error> {
    let mut state = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .ok_or("Cannot find your application data directory")?
        .join("opendownload/queue.json");
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    // Choose the settings file before applying explicit CLI overrides.
    for pair in arguments.windows(2) {
        if pair[0] == "--state" {
            state = pair[1].clone().into();
        }
    }
    let settings_path = state.with_file_name("settings.json");
    let mut preferences: Preferences = match std::fs::read(&settings_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Preferences::default(),
        Err(error) => return Err(error.into()),
    };
    let mut port = 8765;
    let mut headless = false;
    let mut args = arguments.into_iter();
    while let Some(argument) = args.next() {
        if argument == "--headless" {
            headless = true;
            continue;
        }
        if argument == "--version" {
            println!("OpenDownload {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        if argument == "--help" {
            println!("OpenDownload desktop application\nOptions: --headless --downloads PATH --state PATH --port NUMBER --connections 1..16");
            return Ok(());
        }
        let value = args.next().ok_or("Expected argument value")?;
        match argument.as_str() {
            "--downloads" => preferences.downloads = value.into(),
            "--state" => state = value.into(),
            "--port" => port = value.parse()?,
            "--connections" => preferences.connections = value.parse()?,
            _ => return Err(format!("Unknown option: {argument}").into()),
        }
    }
    if !(1..=16).contains(&preferences.connections) {
        return Err("Connections must be 1 through 16".into());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let app = runtime.block_on(engine::start(preferences.downloads.clone(), state, port, preferences.connections))
        .map_err(|error| format!("Cannot start OpenDownload: {error}. If port {port} is busy, close the other instance first."))?;
    if headless {
        runtime.block_on(async {
            #[cfg(unix)]
            {
                let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
                tokio::select! { result = tokio::signal::ctrl_c() => result, _ = terminate.recv() => Ok(()) }
            }
            #[cfg(not(unix))]
            tokio::signal::ctrl_c().await
        })?;
    } else {
        let desktop = desktop::Desktop::new(
            app.clone(),
            runtime.handle().clone(),
            preferences,
            settings_path,
        );
        eframe::run_native(
            "OpenDownload",
            desktop::options(),
            Box::new(move |cc| {
                desktop::style(&cc.egui_ctx);
                Ok(Box::new(desktop))
            }),
        )
        .map_err(|error| error.to_string())?;
    }
    runtime.block_on(app.pause_all())?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        if !std::env::args().any(|arg| arg == "--headless") {
            desktop::show_error(error.to_string());
        }
        std::process::exit(1);
    }
}
