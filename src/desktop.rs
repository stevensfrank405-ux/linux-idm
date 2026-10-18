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
const PROGRESS: Color32 = Color32::from_rgb(41, 110, 77);

pub(crate) fn options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        #[cfg(target_os = "windows")]
        renderer: eframe::Renderer::Wgpu,
        #[cfg(not(target_os = "windows"))]
        renderer: eframe::Renderer::Glow,
        #[cfg(target_os = "windows")]
        wgpu_options: windows_gpu_options(),
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([870., 520.])
            .with_min_inner_size([700., 410.])
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
        s.spacing.item_spacing = egui::vec2(8., 7.);
        s.spacing.button_padding = egui::vec2(10., 6.);
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
    popups: HashSet<String>,
    known: HashSet<String>,
    selected: Option<String>,
    mode: String,
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
        let known = app
            .snapshot()
            .map(|(jobs, _)| jobs.into_iter().map(|job| job.id).collect())
            .unwrap_or_default();
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
            popups: HashSet::new(),
            known,
            selected: None,
            mode: "auto".into(),
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
    fn job_buttons(&mut self, ui: &mut egui::Ui, job: &Job) {
        if job.status == "downloading" {
            if ui.button("Pause").clicked() {
                self.action("pause", serde_json::json!({"id":job.id}));
            }
        } else if job.status != "completed" {
            let stopping = self.active.contains(&job.id);
            if ui
                .add_enabled(
                    !stopping,
                    egui::Button::new(if stopping {
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
        } else {
            if ui.button("Open file").clicked() {
                self.open(
                    job.directory
                        .as_deref()
                        .unwrap_or(&self.app.root)
                        .join(&job.name),
                );
            }
            if ui.button("Remove entry").clicked() {
                self.action("remove", serde_json::json!({"id":job.id}));
                self.popups.remove(&job.id);
            }
        }
        if ui.button("Open folder").clicked() {
            let folder = job
                .directory
                .as_deref()
                .unwrap_or(&self.app.root)
                .to_path_buf();
            self.open(folder);
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
fn fraction(job: &Job) -> f32 {
    if job.status == "completed" {
        1.
    } else {
        job.total
            .filter(|n| *n > 0)
            .map(|n| job.downloaded as f32 / n as f32)
            .unwrap_or(0.)
            .clamp(0., 1.)
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
            for job in &jobs {
                if self.known.insert(job.id.clone()) {
                    self.popups.insert(job.id.clone());
                    self.selected = Some(job.id.clone());
                }
            }
            self.jobs = jobs;
            self.active = active;
        }
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(RichText::new("OpenDownload").color(GREEN));
                ui.separator();
                if ui.button("Resume all").clicked() {
                    for job in self
                        .jobs
                        .iter()
                        .filter(|job| job.status == "paused" && !self.active.contains(&job.id))
                    {
                        self.action("resume", serde_json::json!({"id":job.id}));
                    }
                }
                if ui.button("Pause all").clicked() {
                    for job in self.jobs.iter().filter(|job| job.status == "downloading") {
                        self.action("pause", serde_json::json!({"id":job.id}));
                    }
                }
                if ui.button("Browser").clicked() {
                    self.pairing = true;
                }
                if ui.button("Options").clicked() {
                    self.settings = true;
                }
                if ui.button("Folder").clicked() {
                    self.open(self.app.root.clone());
                }
            });
            ui.horizontal(|ui| {
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.url)
                        .id(egui::Id::new("download-url"))
                        .hint_text("Paste a file URL or video page link…")
                        .desired_width((ui.available_width() - 240.).max(200.)),
                );
                egui::ComboBox::from_id_salt("download-kind")
                    .selected_text(match self.mode.as_str() {
                        "video" => "Video",
                        "file" => "File",
                        _ => "Auto",
                    })
                    .width(65.)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.mode, "auto".into(), "Auto");
                        ui.selectable_value(&mut self.mode, "video".into(), "Video");
                        ui.selectable_value(&mut self.mode, "file".into(), "File (including HTML)");
                    });
                let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui
                    .add_enabled(
                        !self.pending_add,
                        egui::Button::new("+ Add URL").fill(Color32::from_rgb(39, 80, 61)),
                    )
                    .clicked()
                    || enter)
                    && !self.pending_add
                {
                    if self.url.trim().is_empty() {
                        self.message = "Paste a file or video page URL.".into();
                    } else {
                        self.action(
                            "add",
                            serde_json::json!({"url":self.url.trim(),"kind":self.mode}),
                        );
                        self.pending_add = true;
                        self.url.clear();
                    }
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let speed = self.jobs.iter().map(|job| job.speed.max(0.) as u64).sum();
                ui.small(format!(
                    "{} downloads · {} active · {}/s",
                    self.jobs.len(),
                    self.active.len(),
                    bytes(speed)
                ));
                if !self.message.is_empty() {
                    ui.separator();
                    ui.small(&self.message);
                }
            });
        });
        egui::SidePanel::left("navigation")
            .exact_width(145.)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(10.);
                ui.strong("Categories");
                ui.separator();
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
                        .filter(|job| {
                            label == "All downloads" || job.status.eq_ignore_ascii_case(label)
                        })
                        .count();
                    if ui
                        .selectable_label(self.filter == label, format!("{label} ({count})"))
                        .clicked()
                    {
                        self.filter = label.into();
                    }
                }
                ui.add_space(12.);
                ui.small(format!("Free · v{}", env!("CARGO_PKG_VERSION")));
            });
        // The progress view is a real OS window, separate from the manager.
        if let Some(job) = self
            .jobs
            .iter()
            .find(|job| Some(&job.id) == self.selected.as_ref())
            .cloned()
        {
            egui::TopBottomPanel::bottom("selection-actions").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Show progress").clicked() {
                        self.popups.insert(job.id.clone());
                    }
                    self.job_buttons(ui, &job);
                });
            });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            let jobs: Vec<_> = self.jobs.iter().filter(|job| self.filter == "All downloads" || job.status.eq_ignore_ascii_case(&self.filter)).cloned().collect();
            egui::ScrollArea::both().auto_shrink([false,false]).show(ui, |ui| {
                egui::Grid::new("downloads-table").striped(true).min_row_height(30.).min_col_width(65.).show(ui, |ui| {
                    for label in ["File name", "Size", "Status / progress", "Speed"] { ui.strong(label); }
                    ui.end_row();
                    for job in jobs {
                        let label = egui::RichText::new(&job.name);
                        let row = ui.add_sized([235.,30.], egui::Button::selectable(self.selected.as_deref()==Some(&job.id), label).truncate());
                        if row.clicked() { self.selected = Some(job.id.clone()); }
                        if row.double_clicked() { self.popups.insert(job.id.clone()); }
                        row.on_hover_text(&job.name);
                        ui.label(job.total.map(bytes).unwrap_or_else(|| "Unknown".into()));
                        let text = if job.status == "downloading" && job.total.is_some_and(|n| n>0) { format!("{:.1}%", fraction(&job)*100.) } else { job.status.clone() };
                        ui.add_sized([125.,18.], egui::ProgressBar::new(fraction(&job)).text(text).fill(PROGRESS));
                        ui.label(if job.speed>0. {format!("{}/s", bytes(job.speed as u64))} else {"—".into()});
                        ui.end_row();
                    }
                });
                if self.jobs.is_empty() {
                    ui.add_space(30.);
                    ui.label("Paste a file or video link above to begin.");
                    ui.small("Each download opens its own progress window. Double-click a row to reopen it.");
                }
            });
        });
        let popups: Vec<_> = self
            .jobs
            .iter()
            .filter(|job| self.popups.contains(&job.id))
            .cloned()
            .collect();
        for job in popups {
            let viewport = egui::ViewportId::from_hash_of(("download-progress", &job.id));
            let mut close = false;
            ctx.show_viewport_immediate(
                viewport,
                egui::ViewportBuilder::default()
                    .with_title(format!("Download — {}", job.name))
                    .with_inner_size([560., 390.])
                    .with_min_inner_size([460., 340.])
                    .with_icon(icon()),
                |ctx, _| {
                    close = ctx.input(|input| input.viewport().close_requested());
                    ctx.request_repaint_after(Duration::from_millis(200));
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.heading(if job.status == "completed" {
                            "Download complete"
                        } else {
                            "Download progress"
                        });
                        ui.add_space(6.);
                        ui.label(RichText::new(&job.name).strong());
                        ui.separator();
                        egui::Grid::new(("transfer-details", &job.id))
                            .num_columns(2)
                            .show(ui, |ui| {
                                ui.label("Status");
                                ui.label(if job.phase.is_empty() || job.status != "downloading" {
                                    &job.status
                                } else {
                                    &job.phase
                                });
                                ui.end_row();
                                ui.label("Downloaded");
                                ui.label(format!(
                                    "{} / {}",
                                    bytes(job.downloaded),
                                    job.total.map(bytes).unwrap_or_else(|| "Unknown".into())
                                ));
                                ui.end_row();
                                ui.label("Transfer rate");
                                ui.label(format!("{}/s", bytes(job.speed.max(0.) as u64)));
                                ui.end_row();
                                ui.label("Time remaining");
                                ui.label(
                                    if job.speed > 0.
                                        && job.total.is_some_and(|n| n >= job.downloaded)
                                    {
                                        format!(
                                            "{} seconds",
                                            ((job.total.unwrap() - job.downloaded) as f64
                                                / job.speed)
                                                .ceil()
                                                as u64
                                        )
                                    } else {
                                        "—".into()
                                    },
                                );
                                ui.end_row();
                                ui.label("Connections");
                                ui.label(if job.media {
                                    "Video extractor / parallel fragments".into()
                                } else {
                                    format!("{}", job.segments.max(1))
                                });
                                ui.end_row();
                            });
                        ui.add_space(10.);
                        ui.add(
                            egui::ProgressBar::new(fraction(&job))
                                .show_percentage()
                                .fill(PROGRESS),
                        );
                        ui.add_space(8.);
                        egui::ScrollArea::vertical().max_height(75.).show(ui, |ui| {
                            ui.small(format!("URL: {}", job.url));
                            ui.small(format!(
                                "Save to: {}",
                                job.directory.as_deref().unwrap_or(&self.app.root).display()
                            ));
                            if !job.error.is_empty() {
                                ui.colored_label(Color32::from_rgb(255, 169, 169), &job.error);
                            }
                        });
                        ui.separator();
                        ui.horizontal(|ui| {
                            self.job_buttons(ui, &job);
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        });
                    });
                },
            );
            if close {
                self.popups.remove(&job.id);
            }
        }
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
