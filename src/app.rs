use crate::player::{Player, PlayerStatus, StreamSpec};

/// Public HLS test stream used by the M1 playback spike.
const TEST_URL: &str = "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8";

pub struct App {
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
        let mut app = Self {
            player,
            init_error,
            url,
            referrer: std::env::args().nth(2).unwrap_or_default(),
            user_agent: std::env::args().nth(3).unwrap_or_default(),
            volume: 70.0,
            muted: false,
        };
        app.apply_volume();
        app.play_current();
        app
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
