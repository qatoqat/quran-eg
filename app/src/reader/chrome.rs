//! What surrounds the pages of a reader view: notices above them (integrity,
//! fonts), previous/next navigation, and the loading state below them.

use egui::{Align, Align2, Layout, Rect, Ui, UiBuilder, pos2, vec2};
use quran_core::fonts::FontSet;
use quran_core::lists::find_chapter;
use quran_core::reference;
use quran_core::util::short_list;
use quran_core::view::{Location, Neighbours, View};

use super::{Env, Reader, Space};
use crate::kit::{self, Arrow, Notice, Pill};
use crate::theme::Colors;

/// A child `Ui` for a column of widgets starting at `top`, clipped to `area`.
fn section(ui: &mut Ui, space: Space, top: f32) -> Ui {
    let Space { area, column } = space;
    let rect = Rect::from_min_size(pos2(column.left(), top), vec2(column.width(), 100_000.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(area);
    child.spacing_mut().item_spacing = vec2(8.0, 8.0);
    child
}

fn text(ui: &mut Ui, text: &str, size: f32, color: egui::Color32) {
    ui.label(egui::RichText::new(text).size(size).color(color));
}

/// Next | middle | previous: the Quran reads right to left, so "next" is on the
/// left and "previous" on the right, each with an arrow pointing the way it leads.
fn nav(ui: &mut Ui, colors: &Colors, neighbours: &Neighbours, open: &mut Option<Location>) {
    let high = kit::pill_size(ui, "Next", Pill::Outline, Arrow::Left).y;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), high), egui::Sense::hover());
    let parts = [
        (&neighbours.next, "Next: ", Arrow::Left, Align::Min),
        (&neighbours.middle, "", Arrow::None, Align::Center),
        (&neighbours.prev, "Previous: ", Arrow::Right, Align::Max),
    ];
    for (link, prefix, arrow, side) in parts {
        let Some((to, name)) = link else { continue };
        let label = format!("{prefix}{name}");
        let size = kit::pill_size(ui, &label, Pill::Outline, arrow);
        let rect = Align2([side, Align::Center]).align_size_within_rect(size, row);
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            if kit::arrow_pill(ui, colors, &label, Pill::Outline, arrow).clicked() {
                *open = Some(to.at(None));
            }
        });
    }
}

impl Reader {
    fn set_chrome_height(ui: &Ui, measured: f32, height: &mut f32) -> f32 {
        let change = measured - *height;
        if change.abs() > 0.5 {
            *height = measured;
            ui.ctx().request_repaint();
            return change;
        }
        0.0
    }

    pub(super) fn top_chrome(
        &mut self,
        ui: &mut Ui,
        env: &mut Env<'_>,
        space: Space,
        top: f32,
        open: &mut Option<Location>,
    ) {
        let colors = env.colors;
        let mut reload = false;
        let mut ui = section(ui, space, top);
        ui.add_space(16.0);

        if let Some((shown, total)) = self.feed.short_by().filter(|_| !self.loading() && self.error.is_none()) {
            kit::notice(&mut ui, colors, Notice::Warning, |ui| {
                ui.label(format!("Only {shown} of {total} verses were loaded; some verses may be missing."));
                reload |= kit::link(ui, colors, "Reload", 14.0).clicked();
            });
        }
        if !self.notice.0.is_empty() {
            kit::notice(&mut ui, colors, Notice::Warning, |ui| {
                for line in &self.notice.0 {
                    ui.label(line);
                }
                if self.notice.1 {
                    reload |= kit::link(ui, colors, "Reload", 14.0).clicked();
                }
            });
        }
        let failed = env.fonts.failed_pages(FontSet::of(env.settings.mushaf_font));
        let no_text = env.fonts.fallback_failed();
        if !failed.is_empty() || no_text {
            kit::notice(&mut ui, colors, Notice::Warning, |ui| {
                if !failed.is_empty() {
                    let s = if failed.len() == 1 { "" } else { "s" };
                    let instead = if no_text { "" } else { "; showing the standard Uthmani text there instead" };
                    ui.label(format!("The mushaf font could not be loaded for page{s} {}{instead}.", short_list(&failed, 10)));
                }
                if no_text {
                    ui.label("The text font could not be loaded either: words without their page font are shown as empty boxes.");
                }
                if kit::link(ui, colors, "Retry", 14.0).clicked() {
                    env.fonts.retry_failed();
                    // Line widths measured with the fallback text no longer count.
                    self.blocks.iter_mut().for_each(|b| b.natural = None);
                }
            });
        }

        // The surah's name is on its banner inside the page; a range that starts
        // later has no banner, so it is named here.
        if let View::Surah { chapter, span: Some(span) } = self.view {
            let name = find_chapter(env.chapters, chapter).map_or(format!("Surah {chapter}"), |c| c.name_simple.clone());
            let count = reference::verses_in(chapter).unwrap_or(span.to);
            let verses = if span.is_single() {
                format!("Verse {}", span.from)
            } else {
                format!("Verses {}–{}", span.from, span.to)
            };
            ui.horizontal(|ui| {
                text(ui, &format!("{name} · {verses} of {count} ·"), 14.4, colors.muted);
                if kit::link(ui, colors, "Read the full surah", 14.4).clicked() {
                    *open = Some(View::surah(chapter).at(None));
                }
            });
        }
        if matches!(self.view, View::Page(_) | View::Juz(_)) {
            nav(&mut ui, colors, &Neighbours::of(&self.view, env.chapters), open);
        }
        ui.add_space(4.0);

        let change = Self::set_chrome_height(&ui, ui.min_rect().height(), &mut self.top_height);
        // The pages moved by that much; stay on the line being read.
        if self.scroll > self.top_height - change {
            self.scroll += change;
        }
        if reload {
            self.reload(ui.ctx(), env);
        }
    }

    pub(super) fn bottom_chrome(
        &mut self,
        ui: &mut Ui,
        env: &mut Env<'_>,
        space: Space,
        top: f32,
        open: &mut Option<Location>,
    ) {
        let colors = env.colors;
        let mut ui = section(ui, space, top);
        ui.add_space(8.0);
        let centred = Layout::top_down(Align::Center);
        let mut more = false;
        if let Some(message) = self.error.clone() {
            kit::notice(&mut ui, colors, Notice::Error, |ui| ui.label(format!("Could not load verses: {message}")));
            ui.with_layout(centred, |ui| more = kit::pill(ui, colors, "Try again", Pill::Primary).clicked());
        } else if self.loading() {
            ui.with_layout(centred, |ui| {
                ui.add_space(24.0);
                text(ui, "Loading…", 16.0, colors.muted);
                ui.add_space(24.0);
            });
        } else if !self.feed.complete() {
            ui.with_layout(centred, |ui| {
                if let Some(total) = self.feed.total {
                    text(ui, &format!("{} of {total} verses", self.feed.verses.len()), 14.4, colors.muted);
                }
                more = kit::pill(ui, colors, "Load more", Pill::Primary).clicked();
            });
        } else if self.feed.verses.is_empty() {
            ui.with_layout(centred, |ui| {
                ui.add_space(24.0);
                text(ui, "No verses found.", 14.4, colors.muted);
            });
        }
        ui.add_space(12.0);
        nav(&mut ui, colors, &Neighbours::of(&self.view, env.chapters), open);
        ui.with_layout(centred, |ui| {
            ui.add_space(40.0);
            text(ui, "Text, translations and data from the Quran.com API.", 13.6, colors.muted);
            ui.add_space(28.0);
        });
        Self::set_chrome_height(&ui, ui.min_rect().height(), &mut self.bottom_height);
        if more {
            self.load_more(ui.ctx(), env.client);
        }
    }
}
