//! The Recordings window: active recordings, scheduling, history, settings.

use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, TimeZone, Timelike};
use egui::{Color32, RichText, Ui};
use egui_extras::DatePickerButton;

use crate::db::RecordingRow;
use crate::record::schedule::{Scheduled, Status};
use crate::record::{self, Active, Preset};

/// Everything the window shows; owned by the app, borrowed for one frame.
pub struct View<'a> {
    pub ffmpeg: Option<&'a Path>,
    pub active: &'a [Active],
    pub schedules: &'a [Scheduled],
    pub history: &'a [RecordingRow],
    /// Channel currently selected in the list: (id, name).
    pub selected: Option<(&'a str, &'a str)>,
    pub out_dir: &'a Path,
    pub ffmpeg_setting: &'a str,
    pub default_preset: Preset,
}

pub enum Cmd {
    Schedule {
        channel_id: String,
        channel_name: String,
        start_ts: i64,
        duration_secs: i64,
        preset: Preset,
    },
    CancelSchedule(i64),
    DeleteSchedule(i64),
    Stop(u64),
    Reveal(PathBuf),
    DeleteHistory(i64),
    SetOutDir(String),
    SetFfmpegPath(String),
    SetPreset(Preset),
}

pub struct RecordingsPanel {
    pub open: bool,
    date: jiff::civil::Date,
    hour: u32,
    minute: u32,
    duration_min: u32,
    preset: Preset,
    out_dir_edit: String,
    ffmpeg_edit: String,
    edits_loaded: bool,
    message: String,
}

impl Default for RecordingsPanel {
    fn default() -> Self {
        // Default start: the next full hour.
        let t = Local::now() + ChronoDuration::hours(1);
        Self {
            open: false,
            date: jiff::civil::date(t.year() as i16, t.month() as i8, t.day() as i8),
            hour: t.hour(),
            minute: 0,
            duration_min: 60,
            preset: Preset::Small,
            out_dir_edit: String::new(),
            ffmpeg_edit: String::new(),
            edits_loaded: false,
            message: String::new(),
        }
    }
}

pub fn preset_combo(ui: &mut Ui, id: &str, preset: &mut Preset, width: f32) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(preset.label())
        .width(width)
        .show_ui(ui, |ui| {
            for p in Preset::ALL {
                let text = format!("{}  ({})", p.label(), p.size_hint());
                if ui.selectable_value(preset, p, text).changed() {
                    changed = true;
                }
            }
        });
    changed
}

impl RecordingsPanel {
    pub fn set_default_preset(&mut self, p: Preset) {
        self.preset = p;
    }

    pub fn ui(&mut self, ctx: &egui::Context, v: &View) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        if !self.edits_loaded {
            self.out_dir_edit = v.out_dir.display().to_string();
            self.ffmpeg_edit = v.ffmpeg_setting.to_string();
            self.preset = v.default_preset;
            self.edits_loaded = true;
        }
        let mut open = self.open;
        egui::Window::new("Recordings")
            .open(&mut open)
            .default_width(620.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        self.settings_section(ui, v, &mut cmds);
                        ui.separator();
                        self.active_section(ui, v, &mut cmds);
                        self.schedule_section(ui, v, &mut cmds);
                        self.scheduled_list(ui, v, &mut cmds);
                        ui.separator();
                        self.history_section(ui, v, &mut cmds);
                    });
            });
        self.open = open;
        cmds
    }

    fn settings_section(&mut self, ui: &mut Ui, v: &View, cmds: &mut Vec<Cmd>) {
        match v.ffmpeg {
            Some(p) => {
                ui.colored_label(
                    Color32::from_rgb(110, 200, 130),
                    format!("ffmpeg: {}", p.display()),
                );
            }
            None => {
                ui.colored_label(Color32::LIGHT_RED, record::install_hint());
            }
        }
        egui::CollapsingHeader::new("Settings")
            .default_open(v.ffmpeg.is_none())
            .show(ui, |ui| {
                egui::Grid::new("rec_settings")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Save recordings to");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.out_dir_edit).desired_width(360.0),
                        );
                        ui.end_row();
                        ui.label("ffmpeg path (optional)");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ffmpeg_edit)
                                .hint_text("leave empty to auto-detect")
                                .desired_width(360.0),
                        );
                        ui.end_row();
                    });
                if ui.button("Apply").clicked() {
                    cmds.push(Cmd::SetOutDir(self.out_dir_edit.clone()));
                    cmds.push(Cmd::SetFfmpegPath(self.ffmpeg_edit.clone()));
                }
            });
    }

    fn active_section(&mut self, ui: &mut Ui, v: &View, cmds: &mut Vec<Cmd>) {
        ui.heading("Recording now");
        if v.active.is_empty() {
            ui.weak(
                "Nothing is being recorded. Use ⏺ Record in the player bar, or schedule one below.",
            );
        }
        for a in v.active {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(Color32::from_rgb(230, 60, 60)));
                ui.strong(&a.channel_name);
                ui.label(format!(
                    "{} · {} · {}",
                    a.preset.label(),
                    record::format_duration(a.started.elapsed()),
                    record::format_size(a.size_bytes())
                ));
                if let Some(d) = a.duration {
                    ui.weak(format!("of {}", record::format_duration(d)));
                }
                let label = if a.is_stopping() {
                    "Stopping…"
                } else {
                    "⏹ Stop"
                };
                if ui
                    .add_enabled(!a.is_stopping(), egui::Button::new(label))
                    .clicked()
                {
                    cmds.push(Cmd::Stop(a.id));
                }
            });
        }
        ui.add_space(6.0);
    }

    fn schedule_section(&mut self, ui: &mut Ui, v: &View, cmds: &mut Vec<Cmd>) {
        ui.heading("Schedule a recording");
        match v.selected {
            Some((_, name)) => {
                ui.horizontal(|ui| {
                    ui.label("Channel:");
                    ui.strong(name);
                    ui.weak("(selected in the list)");
                });
            }
            None => {
                ui.weak("Select a channel in the list first.");
            }
        }
        egui::Grid::new("sched_form")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Start");
                ui.horizontal(|ui| {
                    ui.add(DatePickerButton::new(&mut self.date).id_salt("sched_date"));
                    ui.add(
                        egui::DragValue::new(&mut self.hour)
                            .range(0..=23)
                            .custom_formatter(|n, _| format!("{:02}", n as u32)),
                    );
                    ui.label(":");
                    ui.add(
                        egui::DragValue::new(&mut self.minute)
                            .range(0..=59)
                            .custom_formatter(|n, _| format!("{:02}", n as u32)),
                    );
                });
                ui.end_row();
                ui.label("Duration");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.duration_min)
                            .range(1..=1440)
                            .suffix(" min"),
                    );
                    for (label, m) in [("30m", 30), ("1h", 60), ("2h", 120), ("3h", 180)] {
                        if ui.small_button(label).clicked() {
                            self.duration_min = m;
                        }
                    }
                });
                ui.end_row();
                ui.label("Quality");
                if preset_combo(ui, "sched_preset", &mut self.preset, 210.0) {
                    cmds.push(Cmd::SetPreset(self.preset));
                }
                ui.end_row();
            });
        ui.horizontal(|ui| {
            let can = v.selected.is_some();
            if ui
                .add_enabled(can, egui::Button::new("⏺ Schedule"))
                .clicked()
                && let Some((id, name)) = v.selected
            {
                let start = NaiveDate::from_ymd_opt(
                    i32::from(self.date.year()),
                    self.date.month() as u32,
                    self.date.day() as u32,
                )
                .and_then(|d| d.and_hms_opt(self.hour, self.minute, 0))
                .and_then(|dt| Local.from_local_datetime(&dt).earliest());
                match start {
                    None => {
                        self.message =
                            "That local time does not exist (daylight saving change).".into()
                    }
                    Some(t)
                        if t.timestamp() + i64::from(self.duration_min) * 60
                            <= Local::now().timestamp() =>
                    {
                        self.message = "That time is already in the past.".into();
                    }
                    Some(t) => {
                        self.message = String::new();
                        cmds.push(Cmd::Schedule {
                            channel_id: id.to_string(),
                            channel_name: name.to_string(),
                            start_ts: t.timestamp(),
                            duration_secs: i64::from(self.duration_min) * 60,
                            preset: self.preset,
                        });
                    }
                }
            }
            if !self.message.is_empty() {
                ui.colored_label(Color32::from_rgb(230, 190, 90), &self.message);
            }
        });
        ui.weak(
            "Scheduled recordings only run while this app is open (and the computer is awake).",
        );
        ui.add_space(6.0);
    }

    fn scheduled_list(&mut self, ui: &mut Ui, v: &View, cmds: &mut Vec<Cmd>) {
        if v.schedules.is_empty() {
            return;
        }
        ui.heading("Scheduled");
        for s in v.schedules {
            ui.horizontal(|ui| {
                let when = Local
                    .timestamp_opt(s.start_ts, 0)
                    .single()
                    .map(|t| t.format("%a %d %b %H:%M").to_string())
                    .unwrap_or_default();
                let color = match s.status {
                    Status::Pending => ui.visuals().text_color(),
                    Status::Running => Color32::from_rgb(230, 90, 90),
                    Status::Done => Color32::from_rgb(110, 200, 130),
                    _ => Color32::GRAY,
                };
                ui.strong(&s.channel_name);
                let len = if s.duration_secs < 60 {
                    format!("{} s", s.duration_secs)
                } else {
                    format!("{} min", s.duration_secs / 60)
                };
                ui.label(format!("{when} · {len} · {}", s.preset.label()));
                ui.colored_label(color, s.status.label());
                if !s.note.is_empty() {
                    ui.weak(&s.note);
                }
                match s.status {
                    Status::Pending => {
                        if ui.small_button("Cancel").clicked() {
                            cmds.push(Cmd::CancelSchedule(s.id));
                        }
                    }
                    Status::Running => {}
                    _ => {
                        if ui.small_button("Remove").clicked() {
                            cmds.push(Cmd::DeleteSchedule(s.id));
                        }
                    }
                }
            });
        }
        ui.add_space(6.0);
    }

    fn history_section(&mut self, ui: &mut Ui, v: &View, cmds: &mut Vec<Cmd>) {
        ui.heading("Recordings");
        if v.history.is_empty() {
            ui.weak("No recordings yet.");
        }
        for r in v.history {
            ui.horizontal(|ui| {
                let when = Local
                    .timestamp_opt(r.started_ts, 0)
                    .single()
                    .map(|t| t.format("%d %b %H:%M").to_string())
                    .unwrap_or_default();
                let exists = Path::new(&r.path).exists();
                if r.ok {
                    ui.strong(&r.channel_name);
                } else {
                    ui.strong(RichText::new(&r.channel_name).color(Color32::LIGHT_RED));
                }
                ui.label(format!(
                    "{when} · {} · {}",
                    r.preset.label(),
                    record::format_size(r.size_bytes)
                ));
                if !r.ok {
                    ui.colored_label(Color32::LIGHT_RED, "failed");
                } else if !exists {
                    ui.weak("(file missing)");
                }
                if !r.note.is_empty() {
                    ui.weak(&r.note).on_hover_text(&r.note);
                }
                if r.ok && exists && ui.small_button("Show").on_hover_text(&r.path).clicked() {
                    cmds.push(Cmd::Reveal(PathBuf::from(&r.path)));
                }
                if ui
                    .small_button("Remove")
                    .on_hover_text("Remove from this list (the file is kept)")
                    .clicked()
                {
                    cmds.push(Cmd::DeleteHistory(r.id));
                }
            });
        }
    }
}
