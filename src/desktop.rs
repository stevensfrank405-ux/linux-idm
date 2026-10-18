use crate::{
    engine::{App, Job},
    Preferences,
};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::Ordering, mpsc, Arc},
    time::Duration,
};
const GREEN: Color32 = Color32::from_rgb(139, 229, 187);

pub(crate) fn options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        #[cfg(target_os = "windows")]
        renderer: eframe::Renderer::Wgpu,
        #[cfg(not(target_os = "windows"))]
        renderer: eframe::Renderer::Glow,
        #[cfg(target_os = "windows")]
        wgpu_options: windows_gpu_options(),
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100., 740.])
            .with_min_inner_size([740., 520.])
            .with_app_id("opendownload")
            .with_icon(icon()),
        ..Default::default()
    }
}
// DirectX supports WARP software adapters on machines without OpenGL 2.0 drivers.
#[cfg(target_os = "windows")]
fn windows_gpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
    let mut setup = WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    if std::env::var("OPENDOWNLOAD_SOFTWARE_RENDERING").as_deref() == Ok("1") {
        setup.native_adapter_selector = Some(Arc::new(|adapters, surface| {
            adapters
                .iter()
                .find(|adapter| {
                    adapter.get_info().device_type == wgpu::DeviceType::Cpu
                        && surface.is_none_or(|surface| adapter.is_surface_supported(surface))
                })
                .cloned()
                .ok_or_else(|| "No compatible DirectX software adapter is available".to_owned())
        }));
    }
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

fn icon() -> egui::IconData {
    let mut rgba = vec![0; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let arrow = ((27..37).contains(&x) && (12..40).contains(&y))
                || ((32..49).contains(&y) && x >= y - 17 && x <= 81 - y)
                || ((15..49).contains(&x) && (51..56).contains(&y));
            let color = if arrow {
                [139, 229, 187, 255]
            } else {
                [22, 27, 34, 255]
            };
            rgba[(y * 64 + x) * 4..(y * 64 + x + 1) * 4].copy_from_slice(&color);
        }
    }
    egui::IconData {
        width: 64,
        height: 64,
        rgba,
    }
}
pub(crate) fn style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.style_mut(|s| s.visuals.override_text_color = Some(Color32::from_rgb(225, 233, 241)));
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(12., 12.);
        s.spacing.button_padding = egui::vec2(14., 9.);
        s.visuals.panel_fill = Color32::from_rgb(20, 25, 32);
        s.visuals.selection.bg_fill = Color32::from_rgb(39, 80, 61);
    });
}
pub(crate) struct Desktop {
    app: Arc<App>,
    runtime: tokio::runtime::Handle,
    url: String,
    filter: String,
    message: String,
    jobs: Vec<Job>,
    active: HashSet<String>,
    sender: mpsc::Sender<Result<String, String>>,
    receiver: mpsc::Receiver<Result<String, String>>,
    preferences: Preferences,
    settings_path: PathBuf,
    folder: String,
    settings: bool,
    pairing: bool,
    pending_add: bool,
}
impl Desktop {
    pub(crate) fn new(
        app: Arc<App>,
        runtime: tokio::runtime::Handle,
        preferences: Preferences,
        settings_path: PathBuf,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let folder = preferences.downloads.display().to_string();
        Self {
            app,
            runtime,
            preferences,
            settings_path,
            folder,
            url: String::new(),
            filter: "All downloads".into(),
            message: String::new(),
            jobs: Vec::new(),
            active: HashSet::new(),
            sender,
            receiver,
            settings: false,
            pairing: false,
            pending_add: false,
        }
    }
    fn action(&self, path: &'static str, body: serde_json::Value) {
        let app = self.app.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let response = app
                    .client
                    .post(format!("http://127.0.0.1:{}/api/{path}", app.port))
                    .header("X-Token", &app.token)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let status = response.status();
                let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
                if !status.is_success() {
                    return Err(value["error"]
                        .as_str()
                        .unwrap_or("Request failed")
                        .to_owned());
                }
                Ok(if path == "add" {
                    "Download added".into()
                } else {
                    String::new()
                })
            }
            .await;
            let _ = sender.send(result);
        });
    }
    fn open(&mut self, path: impl AsRef<std::path::Path>) {
        if let Err(error) = open::that(path.as_ref()) {
            self.message = format!("Cannot open folder: {error}");
        }
    }
    fn companion_folder(&self) -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            let path = exe.parent().unwrap().join("extensions");
            if path.exists() {
                return path;
            }
        }
        let installed = PathBuf::from("/usr/share/opendownload/extensions");
        if installed.exists() {
            installed
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("extension")
        }
    }
    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings;
        egui::Window::new("Settings").open(&mut open).resizable(false).show(ctx,|ui|{
            ui.label("Connections per file"); ui.add(egui::Slider::new(&mut self.preferences.connections,1..=16));
            ui.label("Download folder"); ui.add(egui::TextEdit::singleline(&mut self.folder).desired_width(430.));
            ui.small("Folder changes apply after restarting. Existing partial files remain in the previous folder.");
            if ui.button("Save settings").clicked(){
                let path=PathBuf::from(self.folder.trim());
                if !path.is_absolute(){self.message="Choose an absolute download folder path.".into();return;}
                let result=(||->Result<(),crate::engine::Error>{
                    std::fs::create_dir_all(&path)?;
                    self.preferences.downloads=path;
                    let temporary=self.settings_path.with_extension("tmp");
                    std::fs::write(&temporary,serde_json::to_vec_pretty(&self.preferences)?)?;
                    std::fs::rename(temporary,&self.settings_path)?;Ok(())
                })();
                match result {Ok(())=>{self.app.connections.store(self.preferences.connections,Ordering::Relaxed);self.message="Settings saved. Folder changes take effect after restarting.".into();},Err(error)=>self.message=error.to_string()}
            }
        });
        self.settings = open;
    }
    fn pairing_window(&mut self, ctx: &egui::Context) {
        let mut open = self.pairing;
        egui::Window::new("Browser companion").open(&mut open).resizable(false).show(ctx,|ui|{
            ui.heading("Connect your browser");ui.label("1. Install the companion extension from its folder.");
            if ui.button("Open extension folder").clicked(){self.open(self.companion_folder());}
            ui.label("2. Open the extension popup and paste this pairing code.");
            if ui.button("Copy pairing code").clicked(){ctx.copy_text(self.app.token.clone());self.message="Pairing code copied. Keep it private.".into();}
            ui.label("3. Choose Save pairing in the extension popup.");
            ui.small("Pairing is remembered after restart. The app must be running to receive links.");
            ui.small("Chrome/Edge/Brave: Load unpacked → chromium folder. Firefox: load the Firefox ZIP.");
            if self.app.port!=8765 {ui.colored_label(Color32::YELLOW,"The extension requires the default port, 8765.");}
        });
        self.pairing = open;
    }
}
fn bytes(value: u64) -> String {
    let mut value = value as f64;
    let mut unit = "B";
    for next in ["KB", "MB", "GB", "TB"] {
        if value < 1024. {
            break;
        }
        value /= 1024.;
        unit = next;
    }
    format!("{value:.1} {unit}")
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(200));
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::L,
            ))
        }) {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("download-url")));
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::B,
            ))
        }) {
            self.pairing = !self.pairing;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Comma,
            ))
        }) {
            self.settings = !self.settings;
        }

        while let Ok(result) = self.receiver.try_recv() {
            self.pending_add = false;
            self.message = match result {
                Ok(message) => message,
                Err(error) => error,
            };
        }
        if let Some((jobs, active)) = self.app.snapshot() {
            self.jobs = jobs;
            self.active = active;
        }
        egui::SidePanel::left("navigation")
            .exact_width(210.)
            .show(ctx, |ui| {
                ui.add_space(22.);
                ui.heading(RichText::new("OpenDownload").color(GREEN));
                ui.small("FREE. OPEN. YOURS.");
                ui.add_space(30.);
                for label in [
                    "All downloads",
                    "Downloading",
                    "Paused",
                    "Completed",
                    "Failed",
                ] {
                    let count = self
                        .jobs
                        .iter()
                        .filter(|j| {
                            label == "All downloads" || j.status.eq_ignore_ascii_case(label)
                        })
                        .count();
                    if ui
                        .selectable_label(self.filter == label, format!("{label}  {count}"))
                        .clicked()
                    {
                        self.filter = label.into();
                    }
                }
                ui.add_space(30.);
                if ui.button("Downloads folder").clicked() {
                    self.open(self.app.root.clone());
                }
                if ui.button("Browser companion").clicked() {
                    self.pairing = true;
                }
                if ui.button("Settings").clicked() {
                    self.settings = true;
                }
                ui.add_space(24.);
                ui.small(format!("Version {} · Rust", env!("CARGO_PKG_VERSION")));
                ui.small("Windows + Linux");
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!("Saving to {}", self.app.root.display()));
                if !self.message.is_empty() {
                    ui.separator();
                    ui.label(&self.message);
                }
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(15.);
            ui.label(RichText::new("A LITTLE LESS WAITING").small().color(GREEN));
            ui.heading(RichText::new("Your downloads, in control.").size(28.));
            ui.label("Your files, without a subscription.");
            ui.add_space(18.);
            ui.horizontal(|ui| {
                let speed = self.jobs.iter().map(|j| j.speed.max(0.) as u64).sum();
                ui.label(
                    RichText::new(format!("{}/s", bytes(speed)))
                        .size(21.)
                        .color(GREEN),
                );
                ui.separator();
                ui.label(format!("{} active", self.active.len()));
                ui.separator();
                ui.label(format!(
                    "{} completed",
                    self.jobs.iter().filter(|j| j.status == "completed").count()
                ));
            });
            ui.add_space(18.);
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.url)
                        .id(egui::Id::new("download-url"))
                        .hint_text("Paste a file URL (https://…)")
                        .desired_width((ui.available_width() - 150.).max(200.)),
                );
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui
                    .add_enabled(
                        !self.pending_add,
                        egui::Button::new("+ Add download").fill(Color32::from_rgb(39, 80, 61)),
                    )
                    .clicked()
                    || enter)
                    && !self.pending_add
                {
                    if self.url.trim().is_empty() {
                        self.message = "Paste a direct HTTP or HTTPS file URL.".into();
                    } else {
                        self.action("add", serde_json::json!({"url":self.url.trim()}));
                        self.pending_add = true;
                        self.url.clear();
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Pause all").clicked() {
                    for job in self.jobs.iter().filter(|j| j.status == "downloading") {
                        self.action("pause", serde_json::json!({"id":job.id}));
                    }
                }
                if ui.button("Resume all").clicked() {
                    for job in self
                        .jobs
                        .iter()
                        .filter(|j| j.status == "paused" && !self.active.contains(&j.id))
                    {
                        self.action("resume", serde_json::json!({"id":job.id}));
                    }
                }
            });
            ui.add_space(12.);
            ui.heading(&self.filter);
            let jobs: Vec<_> = self
                .jobs
                .iter()
                .filter(|j| {
                    self.filter == "All downloads" || j.status.eq_ignore_ascii_case(&self.filter)
                })
                .cloned()
                .collect();
            egui::ScrollArea::vertical().show(ui, |ui| {
                if jobs.is_empty() {
                    ui.add_space(45.);
                    ui.label(RichText::new("Nothing here yet").size(22.));
                    ui.label("Add a direct file URL or send one from your browser.");
                }
                for job in jobs {
                    egui::Frame::group(ui.style())
                        .inner_margin(16.)
                        .show(ui, |ui| {
                            ui.set_min_width((ui.available_width() - 12.).max(100.));
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&job.name).strong().size(16.));
                                if job.status == "completed" {
                                    if ui.button("Show folder").clicked() {
                                        self.open(
                                            job.directory
                                                .clone()
                                                .unwrap_or_else(|| self.app.root.clone()),
                                        );
                                    }
                                    if ui.button("Remove from list").clicked() {
                                        self.action("remove", serde_json::json!({"id":job.id}));
                                    }
                                } else if job.status == "downloading" {
                                    if ui.button("Pause").clicked() {
                                        self.action("pause", serde_json::json!({"id":job.id}));
                                    }
                                } else {
                                    let busy = self.active.contains(&job.id);
                                    if ui
                                        .add_enabled(
                                            !busy,
                                            egui::Button::new(if busy {
                                                "Stopping…"
                                            } else if job.status == "failed" {
                                                "Retry"
                                            } else {
                                                "Resume"
                                            }),
                                        )
                                        .clicked()
                                    {
                                        self.action("resume", serde_json::json!({"id":job.id}));
                                    }
                                }
                            });
                            let total = job
                                .total
                                .map(bytes)
                                .unwrap_or_else(|| "Unknown size".into());
                            ui.small(format!(
                                "{} · {} / {} · {}/s{}",
                                job.status,
                                bytes(job.downloaded),
                                total,
                                bytes(job.speed.max(0.) as u64),
                                if job.segments > 1 {
                                    format!(" · {} segments", job.segments)
                                } else {
                                    String::new()
                                }
                            ));
                            let fraction = job
                                .total
                                .filter(|n| *n > 0)
                                .map(|n| job.downloaded as f32 / n as f32)
                                .unwrap_or(0.)
                                .clamp(0., 1.);
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .fill(GREEN)
                                    .show_percentage(),
                            );
                            if !job.error.is_empty() {
                                ui.colored_label(Color32::from_rgb(255, 169, 169), &job.error);
                            }
                        });
                    ui.add_space(8.);
                }
            });
        });
        if self.settings {
            self.settings_window(ctx);
        }
        if self.pairing {
            self.pairing_window(ctx);
        }
    }
}
pub(crate) fn show_error(message: String) {
    struct ErrorWindow(String);
    impl eframe::App for ErrorWindow {
        fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("OpenDownload could not start");
                ui.label(&self.0);
                if ui.button("Close").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        }
    }
    let _ = eframe::run_native(
        "OpenDownload — startup error",
        options(),
        Box::new(move |cc| {
            style(&cc.egui_ctx);
            Ok(Box::new(ErrorWindow(message)))
        }),
    );
}
