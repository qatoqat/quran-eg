//! The header, kept to the width of the content column: the app's name on the
//! left; in the middle the view's title, or the search box while searching; on
//! the right a search button, the Surah | Juz | Page toggle and the settings button.

use egui::{Align, FontId, Layout, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use quran_core::view::{Location, Mode, View};

use crate::kit::{self, Icon, Mic};
use crate::reader::Env;
use crate::search::{Go, SearchBox};
use crate::theme::Colors;

pub const HEIGHT: f32 = 51.0;

pub enum HeaderAction {
    Open(Location),
    /// Show the search results for a query.
    Results(String),
    ToggleSettings,
    /// Start or stop recording the recitation (for training the voice pointer).
    ToggleRecord,
    /// Start or stop following the recitation with the voice pointer.
    ToggleListen,
}

/// What the header shows.
pub struct HeaderInput {
    pub content_width: f32,
    pub title: Option<String>,
    /// The mode of the view on screen (none for search results).
    pub mode: Option<Mode>,
    /// Where each of the toggle's buttons leads, and its tooltip.
    pub targets: [(Location, String); 3],
    pub settings_open: bool,
    /// Recording: seconds so far and the word it started at.
    pub recording: Option<(f32, String)>,
    pub listening: Option<Listening>,
}

/// Listening: what the listener hears, and how its microphone does.
pub struct Listening {
    pub summary: String,
    pub mic: Mic,
    /// The microphone's level or trouble, and its name, for the tooltip.
    pub mic_line: Option<String>,
}

/// The Listen button's tooltip while listening: room for its usual lines (longer ones wrap), and
/// narrow enough to sit right of the button from the default window width up.
const LISTEN_TIP_WIDTH: f32 = 360.0;

pub fn show(ui: &mut Ui, env: &mut Env<'_>, search: &mut SearchBox, input: HeaderInput) -> Option<HeaderAction> {
    let colors = env.colors;
    let bar = ui.max_rect();
    ui.painter().rect_filled(bar, 0, colors.card);
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, colors.border));
    let width = input.content_width.min(bar.width()) - 2.0 * crate::reader::COLUMN_PAD;
    let inner = Rect::from_center_size(bar.center(), vec2(width.max(100.0), bar.height()));
    let mut action = None;

    // Left: the app's name, leading to the surah list.
    let mut left = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let name = kit::galley(&left, "The Noble Quran", 16.0, colors.fg, f32::INFINITY);
    let (rect, brand) = left.allocate_exact_size(name.size(), Sense::click());
    left.painter().galley(rect.min, name, colors.fg);
    if brand.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        action = Some(HeaderAction::Open(View::SurahList.at(None)));
    }

    // Right: settings, the toggle, search (laid out from the right).
    let mut right = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing.x = 9.6;
    if kit::icon_button(&mut right, colors, Icon::Settings, input.settings_open).clicked() {
        action = Some(HeaderAction::ToggleSettings);
    }
    if let Some(to) = toggle(&mut right, colors, input.mode, &input.targets) {
        action = Some(HeaderAction::Open(to));
    }
    if kit::icon_button(&mut right, colors, Icon::Search, search.open).clicked() {
        search.toggle();
    }
    // The microphone is the desktop app's (the web build has neither button).
    if cfg!(not(target_arch = "wasm32")) {
        let mic = input.listening.as_ref().map_or(Mic::Off, |l| l.mic);
        let button = kit::icon_button(&mut right, colors, Icon::Listen(mic), input.listening.is_some());
        let button = match &input.listening {
            Some(Listening { summary, mic_line, .. }) => {
                let tip = mic_line.as_ref().map_or(format!("Stop listening ({summary})"), |line| format!("Stop listening ({summary})\n{line}"));
                // The same width whatever it says: egui puts a tooltip on the side of the button where
                // its last size fits, so one whose numbers change would jump from side to side.
                button.on_hover_ui(|ui| {
                    ui.set_width(LISTEN_TIP_WIDTH);
                    ui.label(tip);
                })
            }
            None => button.on_hover_text("Follow your recitation: a pointer on the letter being read (quran-voice's listener)"),
        };
        if button.clicked() {
            action = Some(HeaderAction::ToggleListen);
        }
        let tip = match &input.recording {
            Some((secs, from)) => format!("Stop recording ({:.0}:{:02.0}, from {from})", (secs / 60.0).floor(), secs % 60.0),
            None => "Record your recitation from the top line, to train the voice pointer on your voice".into(),
        };
        if kit::icon_button(&mut right, colors, Icon::Record, input.recording.is_some()).on_hover_text(tip).clicked() {
            action = Some(HeaderAction::ToggleRecord);
        }
    }
    let tools_left = right.min_rect().left();
    let brand_right = left.min_rect().right();

    // Middle: what is being read, or the search box while searching.
    if search.open {
        // Centred like the title, as wide as the narrower side allows.
        let centre = inner.center().x;
        let half = (centre - brand_right).min(tools_left - centre) - 16.0;
        let field = Rect::from_center_size(inner.center(), vec2((2.0 * half).clamp(80.0, 352.0), 32.0));
        match search.ui(ui, env, field) {
            Some(Go::To(to)) => action = Some(HeaderAction::Open(to)),
            Some(Go::Results(query)) => action = Some(HeaderAction::Results(query)),
            None => {}
        }
    } else if let Some(title) = &input.title {
        // Centred in the header when there is room between the name and the
        // tools, else in that room, cut short with an ellipsis if need be.
        let room = egui::Rangef::new(brand_right + 16.0, tools_left - 16.0);
        if room.span() > 24.0 {
            let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), FontId::proportional(16.8), colors.fg);
            job.wrap = egui::text::TextWrapping { max_width: room.span(), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let text = ui.painter().layout_job(job);
            let half = text.size().x / 2.0;
            let centre = inner.center().x.clamp(room.min + half, (room.max - half).max(room.min + half));
            ui.painter().galley(pos2(centre - half, inner.center().y - text.size().y / 2.0), text, colors.fg);
        }
    }
    action
}

/// Surah | Juz | Page: the current mode's button is filled; the hovered one
/// says where it leads, in a tooltip under the toggle.
fn toggle(ui: &mut Ui, colors: &Colors, mode: Option<Mode>, targets: &[(Location, String); 3]) -> Option<Location> {
    let on = mode.and_then(|mode| Mode::ALL.iter().position(|m| *m == mode));
    let toggle = kit::segments(ui, colors, &Mode::ALL.map(|m| m.label()), on);
    if let Some(i) = toggle.hovered() {
        kit::tip(ui.ctx(), colors, toggle.rect, &[(&targets[i].1, false)], true);
    }
    toggle.clicked().map(|i| targets[i].0)
}
