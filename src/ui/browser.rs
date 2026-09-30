//! Channel browser: country-grouped, searchable, virtualized list.
//!
//! The list is flattened into [`Item`]s (country headers + channel rows of
//! expanded countries) so one `ScrollArea::show_rows` can virtualize it, no
//! matter how many of the ~10k channels are visible.

use std::collections::{HashMap, HashSet};

use egui::{Align2, Color32, FontId, Sense, Ui, pos2, vec2};

use crate::model::{BrowseChannel, Country};

const ROW_HEIGHT: f32 = 28.0;
const LOGO_SIZE: f32 = 22.0;
const OTHER_CODE: &str = "";

struct Group {
    code: String,
    title: String,
    /// Indexes into `ChannelBrowser::channels`, sorted by channel name.
    channels: Vec<usize>,
}

#[derive(Clone, Copy)]
enum Item {
    Header { group: usize, shown: usize },
    Channel(usize),
}

/// What the user did in the list this frame.
#[derive(Default)]
pub struct BrowserEvent {
    /// Show-NSFW checkbox changed to this value.
    pub nsfw_changed: Option<bool>,
}

#[derive(Default)]
pub struct ChannelBrowser {
    channels: Vec<BrowseChannel>,
    /// Lowercased channel names, parallel to `channels`.
    lower_names: Vec<String>,
    groups: Vec<Group>,
    query: String,
    show_nsfw: bool,
    open: HashSet<String>,
    visible: Vec<Item>,
    selected: Option<usize>,
}

impl ChannelBrowser {
    pub fn set_data(&mut self, countries: Vec<Country>, channels: Vec<BrowseChannel>) {
        let selected_id = self.selected_channel().map(|c| c.id.clone());

        let mut by_country: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, c) in channels.iter().enumerate() {
            by_country
                .entry(c.country_code.as_str())
                .or_default()
                .push(i);
        }

        let mut groups = Vec::new();
        for c in &countries {
            if let Some(list) = by_country.remove(c.code.as_str()) {
                // Flag emoji are not in egui's default fonts (they show as tofu),
                // so the list shows plain country names.
                let title = c.name.clone();
                groups.push(Group {
                    code: c.code.clone(),
                    title,
                    channels: list,
                });
            }
        }
        // Channels whose country is missing or unknown.
        let rest: Vec<usize> = by_country.into_values().flatten().collect();
        if !rest.is_empty() {
            let mut rest = rest;
            rest.sort_unstable();
            groups.push(Group {
                code: OTHER_CODE.to_string(),
                title: "Other / unknown".to_string(),
                channels: rest,
            });
        }

        self.lower_names = channels.iter().map(|c| c.name.to_lowercase()).collect();
        self.selected = selected_id.and_then(|id| channels.iter().position(|c| c.id == id));
        self.channels = channels;
        self.groups = groups;
        self.rebuild();
    }

    pub fn set_show_nsfw(&mut self, on: bool) {
        self.show_nsfw = on;
        self.rebuild();
    }

    pub fn selected_channel(&self) -> Option<&BrowseChannel> {
        self.selected.and_then(|i| self.channels.get(i))
    }

    /// Recompute the flat list from groups, search text, NSFW flag and open state.
    fn rebuild(&mut self) {
        let q = self.query.trim().to_lowercase();
        let searching = !q.is_empty();
        let mut visible = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            let country_match = searching && g.title.to_lowercase().contains(&q);
            let matching: Vec<usize> = g
                .channels
                .iter()
                .copied()
                .filter(|&i| {
                    (self.show_nsfw || !self.channels[i].is_nsfw)
                        && (!searching || country_match || self.lower_names[i].contains(&q))
                })
                .collect();
            if matching.is_empty() {
                continue;
            }
            visible.push(Item::Header {
                group: gi,
                shown: matching.len(),
            });
            if searching || self.open.contains(&g.code) {
                visible.extend(matching.into_iter().map(Item::Channel));
            }
        }
        self.visible = visible;
    }

    pub fn ui(&mut self, ui: &mut Ui) -> BrowserEvent {
        let mut event = BrowserEvent::default();
        let mut dirty = false;

        ui.add_space(4.0);
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Search channels or countries")
                .desired_width(f32::INFINITY),
        );
        if search.changed() {
            dirty = true;
        }
        ui.horizontal(|ui| {
            if ui.checkbox(&mut self.show_nsfw, "Show NSFW").changed() {
                event.nsfw_changed = Some(self.show_nsfw);
                dirty = true;
            }
            if !self.query.is_empty() && ui.small_button("Clear").clicked() {
                self.query.clear();
                dirty = true;
            }
        });
        ui.separator();

        if self.channels.is_empty() {
            ui.add_space(12.0);
            ui.label("No channels yet. Click Sync to download the list.");
            if dirty {
                self.rebuild();
            }
            return event;
        }
        if self.visible.is_empty() {
            ui.add_space(12.0);
            ui.label("No matches.");
            if dirty {
                self.rebuild();
            }
            return event;
        }

        let searching = !self.query.trim().is_empty();
        let mut toggle: Option<String> = None;
        let mut select: Option<usize> = None;
        let total = self.visible.len();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, total, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in range {
                    match self.visible[row] {
                        Item::Header { group, shown } => {
                            let g = &self.groups[group];
                            let expanded = searching || self.open.contains(&g.code);
                            let (rect, resp) = row_rect(ui);
                            paint_hover(ui, &resp, rect, false);
                            let arrow = if expanded { "▾" } else { "▸" };
                            ui.painter().text(
                                pos2(rect.left() + 6.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                format!("{arrow} {} ({shown})", g.title),
                                FontId::proportional(14.5),
                                ui.visuals().strong_text_color(),
                            );
                            if resp.clicked() && !searching {
                                toggle = Some(g.code.clone());
                            }
                        }
                        Item::Channel(idx) => {
                            let ch = &self.channels[idx];
                            let is_selected = self.selected == Some(idx);
                            let (rect, resp) = row_rect(ui);
                            paint_hover(ui, &resp, rect, is_selected);
                            let logo = egui::Rect::from_min_size(
                                pos2(rect.left() + 22.0, rect.center().y - LOGO_SIZE / 2.0),
                                vec2(LOGO_SIZE, LOGO_SIZE),
                            );
                            if let Some(url) = &ch.logo_url {
                                egui::Image::new(url.as_str())
                                    .fit_to_exact_size(logo.size())
                                    .show_loading_spinner(false)
                                    .paint_at(ui, logo);
                            }
                            let color = if ch.is_nsfw {
                                Color32::from_rgb(220, 120, 120)
                            } else {
                                ui.visuals().text_color()
                            };
                            ui.painter().text(
                                pos2(logo.right() + 8.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                truncate(&ch.name, 34),
                                FontId::proportional(14.0),
                                color,
                            );
                            if resp.clicked() {
                                select = Some(idx);
                            }
                        }
                    }
                }
            });

        if let Some(code) = toggle {
            if !self.open.remove(&code) {
                self.open.insert(code);
            }
            dirty = true;
        }
        if let Some(i) = select {
            self.selected = Some(i);
        }
        if dirty {
            self.rebuild();
        }
        event
    }
}

fn row_rect(ui: &mut Ui) -> (egui::Rect, egui::Response) {
    ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click())
}

fn paint_hover(ui: &Ui, resp: &egui::Response, rect: egui::Rect, selected: bool) {
    if selected {
        ui.painter()
            .rect_filled(rect, 3.0, ui.visuals().selection.bg_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 3.0, ui.visuals().widgets.hovered.bg_fill);
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars - 1).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn country(code: &str, name: &str) -> Country {
        Country {
            code: code.into(),
            name: name.into(),
            flag: String::new(),
        }
    }

    fn channel(id: &str, name: &str, cc: &str, nsfw: bool) -> BrowseChannel {
        BrowseChannel {
            id: id.into(),
            name: name.into(),
            country_code: cc.into(),
            categories: vec![],
            is_nsfw: nsfw,
            logo_url: None,
            stream_count: 1,
        }
    }

    fn browser() -> ChannelBrowser {
        let mut b = ChannelBrowser::default();
        b.set_data(
            vec![country("AF", "Afghanistan"), country("HR", "Croatia")],
            vec![
                channel("a", "Alpha News", "AF", false),
                channel("b", "Beta Adult", "AF", true),
                channel("h1", "HRT 1", "HR", false),
                channel("h2", "HRT 2", "HR", false),
                channel("x", "Mystery", "ZZ", false),
            ],
        );
        b
    }

    fn headers(b: &ChannelBrowser) -> usize {
        b.visible
            .iter()
            .filter(|i| matches!(i, Item::Header { .. }))
            .count()
    }

    #[test]
    fn groups_start_collapsed_and_unknown_country_goes_last() {
        let b = browser();
        assert_eq!(b.visible.len(), 3); // AF, HR, Other - no channel rows
        assert_eq!(headers(&b), 3);
        assert_eq!(b.groups.last().unwrap().title, "Other / unknown");
    }

    #[test]
    fn nsfw_hidden_by_default_and_counted_when_shown() {
        let mut b = browser();
        let Item::Header { shown, .. } = b.visible[0] else {
            panic!()
        };
        assert_eq!(shown, 1); // Beta Adult hidden
        b.set_show_nsfw(true);
        let Item::Header { shown, .. } = b.visible[0] else {
            panic!()
        };
        assert_eq!(shown, 2);
    }

    #[test]
    fn search_expands_and_filters_by_channel_name() {
        let mut b = browser();
        b.query = "hrt 2".into();
        b.rebuild();
        assert_eq!(b.visible.len(), 2); // HR header + one row
        assert!(matches!(b.visible[1], Item::Channel(_)));
    }

    #[test]
    fn search_matches_country_name() {
        let mut b = browser();
        b.query = "croat".into();
        b.rebuild();
        assert_eq!(b.visible.len(), 3); // header + both HRT channels
    }

    #[test]
    fn opening_a_group_lists_its_channels() {
        let mut b = browser();
        b.open.insert("HR".into());
        b.rebuild();
        assert_eq!(b.visible.len(), 5);
    }
}
