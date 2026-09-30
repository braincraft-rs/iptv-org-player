use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use crate::db::{self, Counts, Db};
use crate::player::{Player, PlayerStatus, StreamSpec};
use crate::sync::{self, SyncMsg};
use crate::ui::browser::ChannelBrowser;

/// Public HLS test stream used by the M1 playback spike.
const TEST_URL: &str = "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8";

pub struct App {
    db_path: Option<PathBuf>,
    db: Option<Db>,
    counts: Option<Counts>,
    last_synced: Option<String>,
    sync_rx: Option<Receiver<SyncMsg>>,
    sync_status: String,
    egui_ctx: egui::Context,
    browser: ChannelBrowser,
    player: Option<Player>,
    init_error: Option<String>,
    url: String,
    referrer: String,
    user_agent: String,
    volume: f32,
    muted: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (player, init_error) = match (cc.gl.clone(), cc.get_proc_address.clone()) {
            (Some(gl), Some(loader)) => match Player::new(gl, loader, cc.egui_ctx.clone()) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            },
            _ => (None, Some("OpenGL context not available".to_string())),
        };
        let url = std::env::args()
            .nth(1)
            .unwrap_or_else(|| TEST_URL.to_string());
        let (db_path, db, db_err) = match db::default_path() {
            Ok(path) => match Db::open(&path) {
                Ok(db) => (Some(path), Some(db), None),
                Err(e) => (Some(path), None, Some(format!("{e:#}"))),
            },
            Err(e) => (None, None, Some(format!("{e:#}"))),
        };
        let mut app = Self {
            db_path,
            db,
            counts: None,
            last_synced: None,
            sync_rx: None,
            sync_status: db_err
                .map(|e| format!("Database error: {e}"))
                .unwrap_or_default(),
            egui_ctx: cc.egui_ctx.clone(),
            browser: ChannelBrowser::default(),
            player,
            init_error,
            url,
            referrer: std::env::args().nth(2).unwrap_or_default(),
            user_agent: std::env::args().nth(3).unwrap_or_default(),
            volume: 70.0,
            muted: false,
        };
        egui_extras::install_image_loaders(&cc.egui_ctx);
        app.refresh_counts();
        app.load_browser();
        app.apply_volume();
        app.play_current();
        // Debug hook for headless verification of the sync pipeline.
        if std::env::var_os("IPTV_SYNC_ON_START").is_some() {
            app.start_sync();
        }
        app
    }

    /// Reload countries/channels from the database into the browser list.
    fn load_browser(&mut self) {
        let Some(db) = &self.db else { return };
        let show_nsfw = db.get_setting("show_nsfw").ok().flatten().as_deref() == Some("1");
        self.browser.set_show_nsfw(show_nsfw);
        match db.browse_data() {
            Ok((countries, channels)) => self.browser.set_data(countries, channels),
            Err(e) => self.sync_status = format!("Could not load channels: {e:#}"),
        }
    }

    fn refresh_counts(&mut self) {
        if let Some(db) = &self.db {
            self.counts = db.counts().ok();
            self.last_synced = db.last_synced_at().ok().flatten();
        }
    }

    fn start_sync(&mut self) {
        let Some(path) = self.db_path.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let ctx = self.egui_ctx.clone();
        sync::spawn(path, tx, move || ctx.request_repaint());
        self.sync_rx = Some(rx);
        self.sync_status = "Starting sync...".into();
    }

    fn poll_sync(&mut self) {
        let Some(rx) = &self.sync_rx else { return };
        let mut finished = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                SyncMsg::Progress(s) => self.sync_status = s,
                SyncMsg::Done(s) => {
                    self.sync_status =
                        format!("Synced {} channels, {} streams", s.channels, s.streams);
                    tracing::info!(?s, "sync done");
                    finished = true;
                }
                SyncMsg::Failed(e) => {
                    self.sync_status = format!("Sync failed: {e}");
                    tracing::error!("sync failed: {e}");
                    finished = true;
                }
            }
        }
        if finished {
            self.sync_rx = None;
            self.refresh_counts();
            self.load_browser();
        }
    }

    fn apply_volume(&self) {
        if let Some(p) = &self.player {
            p.set_volume(self.volume as f64);
            p.set_muted(self.muted);
        }
    }

    fn play_current(&mut self) {
        let spec = StreamSpec {
            url: self.url.trim().to_string(),
            referrer: Some(self.referrer.trim().to_string()).filter(|s| !s.is_empty()),
            user_agent: Some(self.user_agent.trim().to_string()).filter(|s| !s.is_empty()),
        };
        if let Some(p) = self.player.as_mut()
            && let Err(e) = p.play(&spec)
        {
            tracing::error!("play failed: {e:#}");
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(p) = self.player.as_mut() {
            p.poll_events();
        }
        self.poll_sync();

        egui::Panel::top("sync_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let syncing = self.sync_rx.is_some();
                if ui
                    .add_enabled(
                        !syncing && self.db_path.is_some(),
                        egui::Button::new("Sync"),
                    )
                    .clicked()
                {
                    self.start_sync();
                }
                if syncing {
                    ui.spinner();
                }
                ui.label(&self.sync_status);
                if let Some(c) = self.counts {
                    ui.separator();
                    ui.label(format!(
                        "{} countries · {} channels · {} streams",
                        c.countries, c.channels, c.streams
                    ));
                }
                if let Some(t) = &self.last_synced {
                    ui.label(format!("(last sync {t} UTC)"));
                }
            });
        });

        egui::Panel::left("channels")
            .default_size(340.0)
            .size_range(260.0..=520.0)
            .show(ui, |ui| {
                let ev = self.browser.ui(ui);
                if let (Some(on), Some(db)) = (ev.nsfw_changed, &self.db) {
                    let _ = db.set_setting("show_nsfw", if on { "1" } else { "0" });
                }
            });

        egui::Panel::top("spike_controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("URL");
                ui.add(egui::TextEdit::singleline(&mut self.url).desired_width(480.0));
                if ui.button("Play").clicked() {
                    self.play_current();
                }
                if ui.button("Stop").clicked()
                    && let Some(p) = self.player.as_mut()
                {
                    p.stop();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Referer");
                ui.add(egui::TextEdit::singleline(&mut self.referrer).desired_width(240.0));
                ui.label("User-Agent");
                ui.add(egui::TextEdit::singleline(&mut self.user_agent).desired_width(240.0));
            });
            ui.horizontal(|ui| {
                let changed = ui
                    .add(egui::Slider::new(&mut self.volume, 0.0..=100.0).text("Volume"))
                    .changed();
                let muted = ui.checkbox(&mut self.muted, "Mute").changed();
                if changed || muted {
                    self.apply_volume();
                }
                if let Some(p) = &self.player {
                    ui.label(format!("{:?}", p.status()));
                }
            });
            if let Some(ch) = self.browser.selected_channel() {
                ui.label(format!(
                    "Selected: {} ({} stream(s)){}",
                    ch.name,
                    ch.stream_count,
                    if ch.categories.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", ch.categories.join(", "))
                    }
                ));
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(err) = &self.init_error {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("Player unavailable: {err}"),
                );
                return;
            }
            let rect = ui.available_rect_before_wrap();
            let ppp = ui.ctx().pixels_per_point();
            let size = [
                (rect.width() * ppp).round() as i32,
                (rect.height() * ppp).round() as i32,
            ];
            if let Some(p) = self.player.as_mut() {
                match p.render_frame(size) {
                    Ok(tex) => {
                        let id = frame.register_native_glow_texture(tex);
                        ui.painter().image(
                            id,
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        if let PlayerStatus::Error(msg) = p.status() {
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                msg,
                                egui::FontId::proportional(20.0),
                                egui::Color32::LIGHT_RED,
                            );
                        }
                    }
                    Err(e) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("{e:#}"));
                    }
                }
            }
        });
    }
}
