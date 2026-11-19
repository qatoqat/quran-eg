//! The translations of a page's verses, under the page: each verse with a link
//! to it and a copy action, its translations with their footnotes.
//!
//! API text is never interpreted as markup: it is reduced to plain segments
//! (`quran_core::html`) and drawn as text.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use egui::{Align, Layout, RichText, Stroke, Ui, vec2};
use quran_core::api::{Translation, Verse};
use quran_core::html::{Segment, display_segments};
use quran_core::settings;
use quran_core::verse_text::copy_text;
use quran_core::view::Location;

use super::Env;
use crate::kit;
use crate::task::{self, Poll};
use crate::theme::Colors;

/// Widest the translations run (they are centred under the page).
pub const MAX_WIDTH: f32 = 760.0;
/// How long "Copied" stays on a copy button, in seconds.
const COPIED_FOR: f64 = 1.5;

enum Note {
    Loading(Receiver<Result<String, String>>),
    Text(String),
    Failed(String),
}

/// The state of the translations of one view: which footnotes are open, the
/// footnotes fetched so far, and what was just copied.
#[derive(Default)]
pub struct Notes {
    /// The open footnote of a translation, by `(verse key, resource id)`.
    open: HashMap<(String, u32), u32>,
    fetched: HashMap<u32, Note>,
    copied: Option<(String, f64)>,
}

impl Notes {
    fn poll(&mut self) {
        for note in self.fetched.values_mut() {
            let Note::Loading(rx) = note else { continue };
            *note = match task::poll(rx) {
                Poll::Done(Ok(text)) => Note::Text(text),
                Poll::Done(Err(e)) => Note::Failed(e),
                Poll::Lost => Note::Failed("the request ended unexpectedly".into()),
                Poll::Pending => continue,
            };
        }
    }

    fn fetch(&mut self, id: u32, ui: &Ui, env: &Env<'_>) {
        self.fetched.entry(id).or_insert_with(|| {
            let read = |answer: Result<quran_core::api::FootNote, quran_core::api::Error>| answer.map(|n| n.text).map_err(|e| e.to_string());
            Note::Loading(crate::net::call(ui.ctx(), env.client.footnote_call(id), read))
        });
    }
}

fn segments(ui: &mut Ui, colors: &Colors, parts: &[Segment], size: f32, mut footnote: impl FnMut(u32) -> bool) -> Option<u32> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, size * 0.35);
        for part in parts {
            match part {
                Segment::Text { text, style } => {
                    let mut rich = RichText::new(text).size(size).color(colors.fg);
                    if style.italic {
                        rich = rich.italics();
                    }
                    if style.bold {
                        rich = rich.strong();
                    }
                    ui.label(rich);
                }
                Segment::LineBreak => ui.end_row(),
                Segment::FootNote { id, label } => {
                    let label = if label.is_empty() { "*" } else { label };
                    let on = footnote(*id);
                    let text = RichText::new(label).size(size * 0.75).color(colors.accent);
                    let stroke = Stroke::new(1.0, if on { colors.accent } else { egui::Color32::TRANSPARENT });
                    let button = egui::Button::new(text).small().frame(on).stroke(stroke);
                    if ui.add(button).on_hover_text(format!("Footnote {label}")).clicked() {
                        clicked = Some(*id);
                    }
                }
            }
        }
    });
    clicked
}

fn translation(ui: &mut Ui, env: &Env<'_>, notes: &mut Notes, verse_key: &str, t: &Translation) {
    let colors = env.colors;
    let size = settings::translation_font_rem(env.settings.translation_font_scale) * settings::REM;
    let key = (verse_key.to_string(), t.resource_id);
    let open = notes.open.get(&key).copied();
    ui.add_space(6.0);
    if let Some(id) = segments(ui, colors, &display_segments(&t.text), size, |id| open == Some(id)) {
        if open == Some(id) {
            notes.open.remove(&key);
        } else {
            notes.open.insert(key.clone(), id);
            notes.fetch(id, ui, env);
        }
    }
    if let Some(name) = t.resource_name.as_deref().filter(|n| !n.is_empty()) {
        ui.label(RichText::new(format!("— {name}")).size(12.5).color(colors.muted));
    }
    let Some(id) = notes.open.get(&key).copied() else { return };
    let frame = egui::Frame::new()
        .fill(colors.card)
        .stroke(Stroke::new(1.0, colors.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 10));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if kit::icon_button(ui, colors, kit::Icon::Close, false).on_hover_text("Close footnote").clicked() {
                notes.open.remove(&key);
            }
            ui.with_layout(Layout::top_down(Align::Min), |ui| match notes.fetched.get(&id) {
                Some(Note::Text(text)) => {
                    segments(ui, colors, &display_segments(text), size * 0.9, |_| false);
                }
                Some(Note::Failed(e)) => {
                    kit::notice(ui, colors, kit::Notice::Error, |ui| ui.label(format!("Could not load footnote: {e}")));
                }
                _ => {
                    ui.label(RichText::new("Loading…").color(colors.muted));
                }
            });
        });
    });
}

/// Draw the translations of `verses` into `ui`. Returns a view to open, if a
/// verse's key was clicked.
pub fn show(ui: &mut Ui, env: &Env<'_>, notes: &mut Notes, verses: &[&Verse]) -> Option<Location> {
    let colors = env.colors;
    let now = ui.input(|i| i.time);
    notes.poll();
    let mut open = None;
    for (i, verse) in verses.iter().enumerate() {
        // A rule above each verse: solid under the page, dashed between verses.
        let y = ui.cursor().top();
        let stroke = Stroke::new(1.0, colors.border);
        if i == 0 {
            ui.painter().hline(ui.max_rect().x_range(), y, stroke);
        } else {
            let ends = [egui::pos2(ui.max_rect().left(), y), egui::pos2(ui.max_rect().right(), y)];
            ui.painter().extend(egui::Shape::dashed_line(&ends, stroke, 4.0, 3.0));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if kit::pill(ui, colors, &verse.verse_key, kit::Pill::Tag).on_hover_text("Open this verse").clicked()
                && let Some(view) = quran_core::nav::verse_view(&verse.verse_key)
            {
                open = Some(view.at(None));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let copied = notes.copied.as_ref().is_some_and(|(key, at)| *key == verse.verse_key && now - at < COPIED_FOR);
                let label = if copied { "Copied" } else { "Copy" };
                if kit::pill(ui, colors, label, kit::Pill::Small).on_hover_text("Copy verse text and translations").clicked() {
                    ui.ctx().copy_text(copy_text(verse));
                    notes.copied = Some((verse.verse_key.clone(), now));
                    ui.ctx().request_repaint_after_secs(COPIED_FOR as f32 + 0.1);
                }
            });
        });
        if verse.translations.is_empty() {
            ui.label(RichText::new("No translation selected.").size(14.4).color(colors.muted));
        }
        for t in &verse.translations {
            translation(ui, env, notes, &verse.verse_key, t);
        }
        ui.add_space(12.0);
    }
    open
}
