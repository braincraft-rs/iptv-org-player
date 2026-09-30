use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Rect, pos2, vec2};

use crate::db::{self, Counts, Db};
use crate::model::Stream;
use crate::player::queue::StreamQueue;
use crate::player::{Player, PlayerStatus, StreamSpec};
use crate::sync::{self, SyncMsg};
use crate::ui::browser::ChannelBrowser;

/// A stream that is still "Loading" after this long counts as failed.
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);

/// The channel currently selected for playback.
struct NowPlaying {
    name: String,
    queue: StreamQueue,
    started: Instant,
    /// Shown when a stream was skipped, e.g. "Stream 1 failed, trying 2/3".
    note: String,
}

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
    now: Option<NowPlaying>,
    paused: bool,
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
        let (db_path, db, db_err) = match db::default_path() {
            Ok(path) => match Db::open(&path) {
                Ok(db) => (Some(path), Some(db), None),
                Err(e) => (Some(path), None, Some(format!("{e:#}"))),
            },
            Err(e) => (None, None, Some(format!("{e:#}"))),
        };
        let setting = |key: &str| db.as_ref().and_then(|d| d.get_setting(key).ok().flatten());
        let volume = setting("volume")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(70.0)
            .clamp(0.0, 100.0);
        let muted = setting("muted").as_deref() == Some("1");

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
            now: None,
            paused: false,
            volume,
            muted,
        };
        egui_extras::install_image_loaders(&cc.egui_ctx);
        app.refresh_counts();
        app.load_browser();
        app.apply_volume();
        // Debug hooks for headless verification.
        if std::env::var_os("IPTV_SYNC_ON_START").is_some() {
            app.start_sync();
        }
        if let Ok(id) = std::env::var("IPTV_AUTOPLAY_ID") {
            app.play_channel(&id, &id);
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

    // ---- sync ----------------------------------------------------------------

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

    // ---- playback ------------------------------------------------------------

    fn apply_volume(&self) {
        if let Some(p) = &self.player {
            p.set_volume(self.volume as f64);
            p.set_muted(self.muted);
        }
    }

    fn save_audio_settings(&self) {
        if let Some(db) = &self.db {
            let _ = db.set_setting("volume", &format!("{:.0}", self.volume));
            let _ = db.set_setting("muted", if self.muted { "1" } else { "0" });
        }
    }

    fn play_channel(&mut self, id: &str, name: &str) {
        let streams = match self.db.as_ref().map(|d| d.streams_for_channel(id)) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                tracing::error!("loading streams failed: {e:#}");
                Vec::new()
            }
            None => Vec::new(),
        };
        self.now = Some(NowPlaying {
            name: name.to_string(),
            queue: StreamQueue::new(streams),
            started: Instant::now(),
            note: String::new(),
        });
        self.start_current();
    }

    /// Start whatever stream the queue currently points at.
    fn start_current(&mut self) {
        let Some(now) = self.now.as_mut() else { return };
        let Some(spec) = now.queue.current().map(to_spec) else {
            return;
        };
        now.started = Instant::now();
        self.paused = false;
        if let Some(p) = self.player.as_mut() {
            p.set_paused(false);
            if let Err(e) = p.play(&spec) {
                tracing::error!("play failed: {e:#}");
            }
        }
    }

    /// The current stream failed or timed out: try the next one.
    fn on_stream_failed(&mut self) {
        let Some(now) = self.now.as_mut() else { return };
        let failed = now.queue.index() + 1;
        let total = now.queue.len();
        tracing::info!(channel = %now.name, failed, total, "stream failed");
        if now.queue.advance().is_some() {
            now.note = format!(
                "Stream {failed}/{total} failed, trying {}/{total}",
                failed + 1
            );
            self.start_current();
        } else {
            now.note = format!("All {total} stream(s) failed");
            if let Some(p) = self.player.as_mut() {
                p.stop();
            }
        }
    }

    fn stop(&mut self) {
        self.now = None;
        self.paused = false;
        if let Some(p) = self.player.as_mut() {
            p.stop();
        }
    }

    fn update_playback(&mut self) {
        let Some(p) = self.player.as_mut() else {
            return;
        };
        p.poll_events();
        let status = p.status().clone();
        let Some(now) = &self.now else { return };
        if now.queue.is_exhausted() {
            return;
        }
        match status {
            PlayerStatus::Error(_) => self.on_stream_failed(),
            PlayerStatus::Loading if now.started.elapsed() > LOAD_TIMEOUT => {
                self.on_stream_failed()
            }
            PlayerStatus::Loading => self.egui_ctx.request_repaint_after(Duration::from_secs(1)),
            PlayerStatus::Playing => {
                if !now.note.is_empty()
                    && now.started.elapsed() > Duration::from_secs(4)
                    && let Some(n) = self.now.as_mut()
                {
                    n.note.clear();
                }
            }
            PlayerStatus::Idle => {}
        }
    }
}

fn to_spec(s: &Stream) -> StreamSpec {
    StreamSpec {
        url: s.url.clone(),
        referrer: s.referrer.clone(),
        user_agent: s.user_agent.clone(),
    }
}

/// "https://host.example/path/x.m3u8" -> "host.example"
fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':', '?'])
        .next()
        .unwrap_or(url)
}

fn stream_label(i: usize, s: &Stream) -> String {
    format!(
        "{}. {} · {}",
        i + 1,
        s.quality.as_deref().unwrap_or("?"),
        host_of(&s.url)
    )
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.poll_sync();
        self.update_playback();

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
                if let Some((id, name)) = ev.activated {
                    self.play_channel(&id, &name);
                }
            });

        self.controls_bar(ui);

        egui::CentralPanel::default().show(ui, |ui| self.video_area(ui, frame));
    }
}

impl App {
    fn controls_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("controls").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let playing = self.now.is_some();
                let label = if self.paused { "▶ Play" } else { "⏸ Pause" };
                if ui.add_enabled(playing, egui::Button::new(label)).clicked() {
                    self.paused = !self.paused;
                    if let Some(p) = &self.player {
                        p.set_paused(self.paused);
                    }
                }
                if ui
                    .add_enabled(playing, egui::Button::new("⏹ Stop"))
                    .clicked()
                {
                    self.stop();
                }
                ui.separator();

                let mut changed = false;
                if ui
                    .selectable_label(self.muted, if self.muted { "🔇" } else { "🔊" })
                    .on_hover_text("Mute")
                    .clicked()
                {
                    self.muted = !self.muted;
                    changed = true;
                }
                let slider = ui.add(
                    egui::Slider::new(&mut self.volume, 0.0..=100.0)
                        .show_value(true)
                        .suffix("%"),
                );
                if slider.changed() {
                    if self.volume > 0.0 {
                        self.muted = false;
                    }
                    changed = true;
                }
                if changed {
                    self.apply_volume();
                }
                if changed && !slider.dragged() || slider.drag_stopped() {
                    self.save_audio_settings();
                }

                // Stream picker for channels with several streams.
                let mut pick = None;
                if let Some(now) = &self.now
                    && now.queue.len() > 1
                {
                    ui.separator();
                    let current = now
                        .queue
                        .current()
                        .map(|s| stream_label(now.queue.index(), s))
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("stream_picker")
                        .selected_text(current)
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            for (i, s) in now.queue.streams().iter().enumerate() {
                                if ui
                                    .selectable_label(i == now.queue.index(), stream_label(i, s))
                                    .clicked()
                                {
                                    pick = Some(i);
                                }
                            }
                        });
                }
                if let Some(i) = pick
                    && let Some(now) = self.now.as_mut()
                {
                    now.queue.select(i);
                    now.note.clear();
                    self.start_current();
                }
            });
            ui.add_space(4.0);
        });
    }

    fn video_area(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(err) = &self.init_error {
            ui.colored_label(Color32::LIGHT_RED, format!("Player unavailable: {err}"));
            return;
        }
        let rect = ui.available_rect_before_wrap();
        let ppp = ui.ctx().pixels_per_point();
        let size = [
            (rect.width() * ppp).round() as i32,
            (rect.height() * ppp).round() as i32,
        ];
        let Some(p) = self.player.as_mut() else {
            return;
        };
        match p.render_frame(size) {
            Ok(tex) => {
                let id = frame.register_native_glow_texture(tex);
                ui.painter().image(
                    id,
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            Err(e) => {
                ui.colored_label(Color32::LIGHT_RED, format!("{e:#}"));
                return;
            }
        }

        // Overlay: what is going on.
        let status = p.status().clone();
        let painter = ui.painter_at(rect);
        let text = |y: f32, s: &str, size: f32, color: Color32| {
            painter.text(
                pos2(rect.center().x, rect.center().y + y),
                Align2::CENTER_CENTER,
                s,
                FontId::proportional(size),
                color,
            );
        };
        let mut retry = false;
        match &self.now {
            None => text(0.0, "Select a channel from the list", 20.0, Color32::GRAY),
            Some(now) => {
                if now.queue.len() == 0 {
                    text(
                        0.0,
                        &format!("{}: no streams available", now.name),
                        20.0,
                        Color32::LIGHT_RED,
                    );
                } else if now.queue.is_exhausted() {
                    text(-16.0, &now.name, 22.0, Color32::WHITE);
                    text(14.0, &now.note, 16.0, Color32::LIGHT_RED);
                    retry = true;
                } else {
                    // Title in the corner once something is on screen.
                    painter.text(
                        rect.left_top() + vec2(12.0, 10.0),
                        Align2::LEFT_TOP,
                        &now.name,
                        FontId::proportional(18.0),
                        Color32::from_white_alpha(200),
                    );
                    if matches!(status, PlayerStatus::Loading | PlayerStatus::Idle) {
                        text(0.0, "Loading…", 20.0, Color32::LIGHT_GRAY);
                    }
                    if !now.note.is_empty() {
                        text(28.0, &now.note, 15.0, Color32::from_rgb(230, 190, 90));
                    }
                }
            }
        }
        if retry {
            let btn = Rect::from_center_size(rect.center() + vec2(0.0, 56.0), vec2(120.0, 30.0));
            if ui.put(btn, egui::Button::new("Retry")).clicked() {
                if let Some(now) = self.now.as_mut() {
                    now.queue.restart();
                    now.note.clear();
                }
                self.start_current();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_strips_scheme_port_and_path() {
        assert_eq!(host_of("https://a.example.com/x/y.m3u8"), "a.example.com");
        assert_eq!(host_of("http://1.2.3.4:1935/live.m3u8"), "1.2.3.4");
        assert_eq!(host_of("weird"), "weird");
    }
}
