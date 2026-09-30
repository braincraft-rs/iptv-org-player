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
    Header {
        group: usize,
        shown: usize,
    },
    Channel(usize),
    /// A favourite whose channel is gone after a sync (index into `missing`).
    Missing(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    All,
    Favourites,
}

/// What the user did in the list this frame.
#[derive(Default)]
pub struct BrowserEvent {
    /// Show-NSFW checkbox changed to this value.
    pub nsfw_changed: Option<bool>,
    /// A channel row was clicked: (channel id, display name).
    pub activated: Option<(String, String)>,
    /// A star was clicked: (channel id, now favourite).
    pub favourite_toggled: Option<(String, bool)>,
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
    tab: Tab,
    favourites: HashSet<String>,
    /// Favourite ids with no matching channel, in the current `visible` list.
    missing: Vec<String>,
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

    pub fn set_favourites(&mut self, ids: impl IntoIterator<Item = String>) {
        self.favourites = ids.into_iter().collect();
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
        if self.tab == Tab::Favourites {
            self.rebuild_favourites(&q);
            return;
        }
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

    /// Flat list of starred channels (by name), then ones no longer available.
    fn rebuild_favourites(&mut self, q: &str) {
        let mut visible = Vec::new();
        let mut known = HashSet::new();
        for (i, c) in self.channels.iter().enumerate() {
            if !self.favourites.contains(&c.id) {
                continue;
            }
            known.insert(c.id.as_str());
            if (self.show_nsfw || !c.is_nsfw) && (q.is_empty() || self.lower_names[i].contains(q)) {
                visible.push(Item::Channel(i));
            }
        }
        let mut missing: Vec<String> = self
            .favourites
            .iter()
            .filter(|id| !known.contains(id.as_str()) && id.to_lowercase().contains(q))
            .cloned()
            .collect();
        missing.sort();
        visible.extend((0..missing.len()).map(Item::Missing));
        self.visible = visible;
        self.missing = missing;
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
        ui.horizontal(|ui| {
            let fav_label = format!("★ Favourites ({})", self.favourites.len());
            for (tab, label) in [
                (Tab::All, "All channels".to_string()),
                (Tab::Favourites, fav_label),
            ] {
                if ui.selectable_label(self.tab == tab, label).clicked() && self.tab != tab {
                    self.tab = tab;
                    dirty = true;
                }
            }
        });
        ui.separator();

        if self.channels.is_empty() && self.favourites.is_empty() {
            ui.add_space(12.0);
            ui.label("No channels yet. Click Sync to download the list.");
            if dirty {
                self.rebuild();
            }
            return event;
        }
        if self.visible.is_empty() {
            ui.add_space(12.0);
            if self.tab == Tab::Favourites && self.query.trim().is_empty() {
                ui.label("No favourites yet. Click ☆ next to a channel to add one.");
            } else {
                ui.label("No matches.");
            }
            if dirty {
                self.rebuild();
            }
            return event;
        }

        let searching = !self.query.trim().is_empty();
        let mut toggle: Option<String> = None;
        let mut select: Option<usize> = None;
        let mut fav_toggle: Option<(String, bool)> = None;
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
                            let is_fav = self.favourites.contains(&ch.id);
                            let star = star_rect(rect);
                            let star_resp =
                                ui.interact(star, ui.id().with(("star", idx)), Sense::click());
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
                                truncate(&ch.name, 30),
                                FontId::proportional(14.0),
                                color,
                            );
                            paint_star(ui, star, is_fav, star_resp.hovered());
                            if star_resp.clicked() {
                                fav_toggle = Some((ch.id.clone(), !is_fav));
                            } else if resp.clicked() {
                                select = Some(idx);
                            }
                        }
                        Item::Missing(mi) => {
                            let id = &self.missing[mi];
                            let (rect, resp) = row_rect(ui);
                            paint_hover(ui, &resp, rect, false);
                            let star = star_rect(rect);
                            let star_resp = ui.interact(
                                star,
                                ui.id().with(("star-missing", mi)),
                                Sense::click(),
                            );
                            ui.painter().text(
                                pos2(rect.left() + 22.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                truncate(&format!("{id} (no longer available)"), 34),
                                FontId::proportional(13.0),
                                Color32::GRAY,
                            );
                            paint_star(ui, star, true, star_resp.hovered());
                            if star_resp.clicked() {
                                fav_toggle = Some((id.clone(), false));
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
        if let Some((id, on)) = fav_toggle {
            if on {
                self.favourites.insert(id.clone());
            } else {
                self.favourites.remove(&id);
            }
            event.favourite_toggled = Some((id, on));
            dirty = true;
        }
        if let Some(i) = select {
            self.selected = Some(i);
            let ch = &self.channels[i];
            event.activated = Some((ch.id.clone(), ch.name.clone()));
        }
        if dirty {
            self.rebuild();
        }
        event
    }
}

fn star_rect(row: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(
        pos2(row.right() - 16.0, row.center().y),
        vec2(26.0, ROW_HEIGHT),
    )
}

fn paint_star(ui: &Ui, rect: egui::Rect, on: bool, hovered: bool) {
    let (glyph, color) = if on {
        ("★", Color32::from_rgb(245, 200, 60))
    } else if hovered {
        ("☆", Color32::from_rgb(245, 200, 60))
    } else {
        ("☆", Color32::from_gray(110))
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(17.0),
        color,
    );
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
    fn favourites_tab_lists_only_starred_and_flags_missing() {
        let mut b = browser();
        b.set_favourites(["h2".to_string(), "gone".to_string()]);
        b.tab = Tab::Favourites;
        b.rebuild();
        assert_eq!(b.visible.len(), 2);
        assert!(matches!(b.visible[0], Item::Channel(i) if b.channels[i].id == "h2"));
        assert!(matches!(b.visible[1], Item::Missing(0)));
        assert_eq!(b.missing, ["gone"]);
    }

    #[test]
    fn favourites_tab_respects_search_and_nsfw() {
        let mut b = browser();
        b.set_favourites(["a".to_string(), "b".to_string()]);
        b.tab = Tab::Favourites;
        b.rebuild();
        assert_eq!(b.visible.len(), 1); // "b" is NSFW, hidden
        b.set_show_nsfw(true);
        assert_eq!(b.visible.len(), 2);
        b.query = "alpha".into();
        b.rebuild();
        assert_eq!(b.visible.len(), 1);
    }

    #[test]
    fn opening_a_group_lists_its_channels() {
        let mut b = browser();
        b.open.insert("HR".into());
        b.rebuild();
        assert_eq!(b.visible.len(), 5);
    }
}
