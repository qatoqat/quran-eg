//! The settings panel: floating over the right edge, so the page underneath
//! stays put and is the live preview. Every change applies at once.

use egui::{Align, Layout, Rect, RichText, Stroke, Ui, pos2, vec2};
use quran_core::api::{Client, TranslationResource};
use quran_core::settings::palette::parse_hex;
use quran_core::settings::storage::sanitize_font_base_url;
use quran_core::settings::translations::{
    display_name, english_translations, fallback_translations, matches_filter, selected_label, toggle_id,
};
use quran_core::settings::{self, LANGUAGE, MushafFont, PointerShape, Settings, Theme, WbwDisplay};

use crate::kit::{self, Icon, Pill};
use crate::remote::Remote;
use crate::theme::Colors;

pub const WIDTH: f32 = 352.0;

/// What the panel keeps while it is open or between openings.
pub struct Panel {
    /// The translations the API offers.
    translations: Remote<TranslationResource>,
    filter: String,
    /// The font source as typed (applied only while it is a usable address).
    font_url: Option<String>,
}

impl Panel {
    pub fn new() -> Self {
        Self { translations: Remote::new(), filter: String::new(), font_url: None }
    }
}

fn legend(ui: &mut Ui, text: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(2.0);
}

fn help(ui: &mut Ui, colors: &Colors, text: &str) {
    ui.label(RichText::new(text).size(13.6).color(colors.muted));
}

fn small_caps(ui: &mut Ui, colors: &Colors, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(12.0).color(colors.muted));
}

/// A row of exclusive choices.
fn segmented<T: PartialEq + Copy>(ui: &mut Ui, colors: &Colors, value: &mut T, options: &[(T, &str)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (option, label) in options {
            if kit::pill(ui, colors, label, Pill::Segment { on: *value == *option }).clicked() {
                *value = *option;
            }
        }
    });
}

/// A checkbox whose label wraps inside the panel (and toggles it too).
fn check(ui: &mut Ui, on: &mut bool, text: &str) -> bool {
    let before = *on;
    ui.horizontal_top(|ui| {
        ui.checkbox(on, "");
        ui.spacing_mut().item_spacing.x = 0.0;
        let label = egui::Label::new(text).wrap().sense(egui::Sense::click());
        if ui.add(label).clicked() {
            *on = !*on;
        }
    });
    *on != before
}

/// A slider under its label, with the value at the right.
fn range(ui: &mut Ui, label: &str, value: &mut u8, range: std::ops::RangeInclusive<u8>, step: u8, unit: &str) {
    ui.add_space(4.0);
    ui.label(label);
    ui.horizontal(|ui| {
        // The rail takes the row but for the value at its right.
        let rail = (ui.available_width() - 52.0).max(60.0);
        ui.spacing_mut().slider_width = rail;
        let slider = egui::Slider::new(value, range).step_by(f64::from(step)).show_value(false);
        ui.add_sized(vec2(rail, 18.0), slider);
        ui.label(format!("{value}{unit}"));
    });
}

/// A colour swatch that opens a picker; `color` stays `#rrggbb`.
fn color(ui: &mut Ui, color: &mut String, label: &str) {
    let mut rgb = parse_hex(color).unwrap_or([255; 3]);
    ui.horizontal(|ui| {
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *color = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        }
        ui.label(label);
    });
}

impl Panel {
    /// Show the panel. Returns `true` when it asks to be closed.
    pub fn show(&mut self, ctx: &egui::Context, screen: Rect, settings: &mut Settings, colors: &Colors, client: &Client) -> bool {
        let now = ctx.input(|i| i.time);
        self.translations.poll(now);
        if self.translations.items.is_empty() && self.translations.error.is_none() {
            self.translations.load(ctx, client.translations_call(LANGUAGE), |list| list);
        }
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let rect = Rect::from_min_max(pos2((screen.right() - WIDTH).max(screen.left()), screen.top()), screen.max);
        let frame = egui::Frame::new()
            .fill(colors.card)
            .stroke(Stroke::new(1.0, colors.border))
            .shadow(egui::Shadow { offset: [-6, 0], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(46) });
        let area = egui::Area::new(egui::Id::unique("settings-panel")).order(egui::Order::Foreground).fixed_pos(rect.min);
        area.constrain(false).show(ctx, |ui| {
            // The panel is exactly `rect`: it takes the pointer there, and its content is laid out inside.
            ui.allocate_rect(rect, egui::Sense::click());
            ui.painter().add(frame.paint(rect));
            let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
            ui.set_clip_rect(rect);
            let ui = &mut ui;
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            // The header stays; the sections scroll under it.
            egui::Frame::new().inner_margin(egui::Margin::symmetric(18, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Settings").size(18.4).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close |= kit::icon_button(ui, colors, Icon::Close, false).on_hover_text("Close settings").clicked();
                        if kit::link(ui, colors, "Reset to defaults", 14.0).clicked() {
                            *settings = Settings::default();
                            self.font_url = None;
                        }
                    });
                });
            });
            ui.painter().hline(rect.x_range(), ui.cursor().top(), Stroke::new(1.0, colors.border));
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 22, top: 8, bottom: 32 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    help(ui, colors, "Changes apply immediately and are saved on this device.");
                    theme(ui, colors, settings);
                    mushaf_font(ui, colors, settings);
                    word_by_word(ui, colors, settings);
                    self.translation_picker(ui, colors, settings);
                    sizes(ui, colors, settings);
                    pointer(ui, colors, settings);
                    #[cfg(not(target_arch = "wasm32"))]
                    voice(ui, colors, settings);
                    self.advanced(ui, colors, settings);
                });
            });
        });
        close
    }

    fn translation_picker(&mut self, ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
        legend(ui, "Translation");
        ui.checkbox(&mut settings.show_page_translation, "Show verse translations under each page");
        ui.add_space(4.0);
        let (available, status) = if !self.translations.items.is_empty() {
            let english = english_translations(&self.translations.items);
            if english.is_empty() {
                (Some(fallback_translations()), Some("No English translations were listed; showing common ones."))
            } else {
                (Some(english), None)
            }
        } else if self.translations.error.is_some() {
            (Some(fallback_translations()), Some("Couldn’t load the translation list; showing common English translations."))
        } else {
            (None, None)
        };
        let known = available.clone().unwrap_or_else(fallback_translations);
        let selected = settings.translations.clone();
        ui.horizontal(|ui| {
            small_caps(ui, colors, "English translations");
            help(ui, colors, &format!("{} selected", selected.len()));
            if !selected.is_empty() && kit::link(ui, colors, "Clear all", 13.6).clicked() {
                settings.translations.clear();
            }
        });
        // Chips show the display order, and let unknown (e.g. non-English) ids be removed.
        let unknown = selected.iter().any(|id| !known.iter().any(|t| t.id == *id));
        if selected.len() > 1 || unknown {
            ui.horizontal_wrapped(|ui| {
                for id in &selected {
                    let label = format!("{}  ×", selected_label(*id, &known));
                    if kit::pill(ui, colors, &label, Pill::Tag).on_hover_text("Remove").clicked() {
                        toggle_id(&mut settings.translations, *id, false);
                    }
                }
            });
        }
        if let Some(status) = status {
            help(ui, colors, status);
        }
        let Some(items) = available else {
            help(ui, colors, "Loading…");
            return;
        };
        if items.len() > 8 {
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter by name or author").desired_width(f32::INFINITY));
        }
        let shown: Vec<&TranslationResource> = items.iter().filter(|t| matches_filter(t, &self.filter)).collect();
        if shown.is_empty() {
            help(ui, colors, &format!("No translations match “{}”.", self.filter.trim()));
        }
        egui::ScrollArea::vertical().id_salt("translation-list").max_height(300.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for t in shown {
                let mut on = selected.contains(&t.id);
                if check(ui, &mut on, display_name(t)) {
                    toggle_id(&mut settings.translations, t.id, on);
                }
                if let Some(author) = t.author_name.as_deref().map(str::trim).filter(|a| !a.is_empty() && *a != display_name(t)) {
                    ui.horizontal_top(|ui| {
                        ui.add_space(26.0);
                        ui.add(egui::Label::new(RichText::new(author).size(12.8).color(colors.muted)).wrap());
                    });
                }
            }
        });
    }

    fn advanced(&mut self, ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
        legend(ui, "Advanced");
        ui.label("Font source URL");
        let draft = self.font_url.get_or_insert_with(|| settings.font_base_url.clone());
        let invalid = !draft.trim().is_empty() && sanitize_font_base_url(draft).is_none();
        ui.horizontal(|ui| {
            let reset = kit::pill_size(ui, "Reset", Pill::Small, kit::Arrow::None).x;
            let field = egui::TextEdit::singleline(draft)
                .hint_text("https://…/fonts/quran")
                .desired_width(ui.available_width() - reset - 24.0)
                .text_color_opt(invalid.then_some(colors.danger));
            if ui.add(field).changed() {
                // Applied as soon as (and only while) it is an address that can be used.
                match (draft.trim().is_empty(), sanitize_font_base_url(draft)) {
                    (true, _) => settings.font_base_url.clear(),
                    (false, Some(url)) => settings.font_base_url = url,
                    (false, None) => {}
                }
            }
            if kit::pill(ui, colors, "Reset", Pill::Small).clicked() {
                draft.clear();
                settings.font_base_url.clear();
            }
        });
        help(
            ui,
            colors,
            "Leave empty to use quran.com's font servers. Set this to a copy of quran.com's fonts/quran folder \
             (a web address, or a folder on this computer) if fonts don't load.",
        );
        if invalid {
            ui.label(RichText::new("Enter an http:// or https:// URL, or a path starting with /.").size(13.6).color(colors.danger));
        }
    }
}

fn theme(ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
    legend(ui, "Theme");
    let options: Vec<(Theme, &str)> = Theme::ALL.iter().map(|t| (*t, t.label())).collect();
    segmented(ui, colors, &mut settings.theme, &options);
    if settings.theme == Theme::Custom {
        ui.horizontal(|ui| {
            color(ui, &mut settings.custom_theme.background, "Background");
            ui.add_space(10.0);
            color(ui, &mut settings.custom_theme.accent, "Accent");
        });
        help(
            ui,
            colors,
            "Text, cards, borders and tooltips are shades worked out from these two colours; a dark background gets \
             light text and the dark tajweed colours.",
        );
    }
}

fn mushaf_font(ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
    legend(ui, "Mushaf font");
    help(ui, colors, "The 15-line Madani mushaf in the King Fahd Complex fonts (Uthman Taha's script).");
    let options = [
        (MushafFont::TajweedV4, "Colour-coded tajweed rules, like quran.com's default mushaf."),
        (MushafFont::MadaniV2, "Single-colour text in the same 15-line Madani layout."),
    ];
    for (font, description) in options {
        let on = settings.mushaf_font == font;
        let stroke = Stroke::new(if on { 2.0 } else { 1.0 }, if on { colors.accent } else { colors.border });
        let card = egui::Frame::new().fill(colors.bg).stroke(stroke).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 10));
        let response = card
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.radio_value(&mut settings.mushaf_font, font, RichText::new(font.label()).strong());
                help(ui, colors, description);
            })
            .response;
        // The whole card chooses, not only its radio button.
        if response.interact(egui::Sense::click()).clicked() {
            settings.mushaf_font = font;
        }
    }
}

fn word_by_word(ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
    legend(ui, "Word by word");
    let wbw = &mut settings.word_by_word;
    ui.checkbox(&mut wbw.show_translation, "Show English meaning");
    ui.checkbox(&mut wbw.show_transliteration, "Show transliteration");
    ui.add_space(4.0);
    small_caps(ui, colors, "Position");
    ui.add_enabled_ui(wbw.show_translation || wbw.show_transliteration, |ui| {
        ui.radio_value(&mut wbw.display, WbwDisplay::Inline, "Under each word");
        ui.radio_value(&mut wbw.display, WbwDisplay::Tooltip, "On hover");
    });
}

fn sizes(ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
    legend(ui, "Text size and layout");
    range(ui, "Arabic text", &mut settings.arabic_font_scale, 1..=10, 1, "");
    range(ui, "Translation text", &mut settings.translation_font_scale, 1..=10, 1, "");
    range(ui, "Space between words", &mut settings.word_spacing, 0..=10, 1, "");
    range(ui, "Space above word meanings", &mut settings.meaning_spacing, 0..=10, 1, "");
    range(ui, "Line spacing", &mut settings.line_spacing, 0..=10, 1, "");
    let width = format!("Content width · {} px", settings::content_width_px(settings.content_width));
    range(ui, &width, &mut settings.content_width, 0..=10, 1, "");
    help(
        ui,
        colors,
        "Spacing applies to the mushaf pages. Line spacing is the space under each line, below the word meanings when \
         they are shown. A page whose lines don't fit the window is scaled down to fit, never wrapped or cut.",
    );
}

fn pointer(ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
    legend(ui, "Pointer");
    let pointer = &mut settings.pointer;
    ui.checkbox(&mut pointer.enabled, "Show a reading pointer");
    help(
        ui,
        colors,
        "A dot or ring in place of the mouse cursor below the top bar. The cursor comes back over the top bar and these settings.",
    );
    if pointer.enabled {
        pointer_look(ui, colors, pointer, "Size (tenths of a line's height)", 1..=10);
    }
}

/// The reading pointer's look (the mouse's, or the voice pointer's dot).
fn pointer_look(ui: &mut Ui, colors: &Colors, pointer: &mut quran_core::settings::Pointer, size: &str, sizes: std::ops::RangeInclusive<u8>) {
    ui.add_space(4.0);
    ui.label("Colour and shape");
    ui.horizontal(|ui| {
        color(ui, &mut pointer.color, "");
        segmented(ui, colors, &mut pointer.shape, &[(PointerShape::Circle, "Circle"), (PointerShape::Ring, "Ring")]);
    });
    range(ui, size, &mut pointer.size, sizes, 1, "");
    range(ui, "Opacity", &mut pointer.opacity, 5..=100, 5, "%");
    range(ui, "Trail", &mut pointer.trail, 0..=10, 1, "");
    if pointer.trail > 0 {
        range(ui, "Trail distance", &mut pointer.trail_distance, 1..=10, 1, "");
    }
    range(ui, "Blur", &mut pointer.blur, 0..=10, 1, "");
    range(ui, "Shrink with speed", &mut pointer.shrink, 0..=10, 1, "");
    range(ui, "Glow (stronger colours under the pointer; blur softens its edge)", &mut pointer.glow, 0..=10, 1, "");
}

/// A slider for a value in milliseconds, shown in seconds.
#[cfg(not(target_arch = "wasm32"))]
fn millis(ui: &mut Ui, label: &str, value: &mut u16, range: std::ops::RangeInclusive<u16>, step: u16) {
    ui.add_space(4.0);
    ui.label(label);
    ui.horizontal(|ui| {
        let rail = (ui.available_width() - 52.0).max(60.0);
        ui.spacing_mut().slider_width = rail;
        let slider = egui::Slider::new(value, range).step_by(f64::from(step)).show_value(false);
        ui.add_sized(vec2(rail, 18.0), slider);
        ui.label(format!("{:.2} s", f32::from(*value) / 1000.0));
    });
}

/// The voice pointer: its dot, the accuracy range around it and how it moves (desktop only).
#[cfg(not(target_arch = "wasm32"))]
fn voice(ui: &mut Ui, colors: &Colors, settings: &mut Settings) {
    legend(ui, "Voice pointer");
    let voice = &mut settings.voice;
    help(
        ui,
        colors,
        "Listen (the microphone in the top bar) follows your recitation with quran-voice's listener and puts a dot on \
         the letter being read, inside a range over the words it could be on.",
    );
    check(ui, &mut voice.auto_scroll, "Keep the line being read in view");
    check(ui, &mut voice.stop_on_closing, "Stop listening at \"sadaqa-llahu-l-'azim\"");
    check(ui, &mut voice.pointer.enabled, "Show the pointer");
    if voice.pointer.enabled {
        pointer_look(ui, colors, &mut voice.pointer, "Size (tenths of the text size)", 2..=20);
        if voice.pointer.shape == PointerShape::Ring {
            range(ui, "Ring line", &mut voice.ring, 1..=20, 1, " pt");
        }
        range(ui, "Centre above the baseline (tenths of the text size)", &mut voice.lift, 0..=10, 1, "");
    }
    ui.add_space(8.0);
    small_caps(ui, colors, "Accuracy range");
    let r = &mut voice.range;
    check(ui, &mut r.enabled, "Show the range (wider when unsure)");
    if r.enabled {
        color(ui, &mut r.color, "Colour");
        range(ui, "Height (tenths of a line's height)", &mut r.size, 4..=20, 1, "");
        range(ui, "Width per unsure word (tenths)", &mut r.spread, 5..=30, 1, "");
        range(ui, "Opacity", &mut r.opacity, 0..=100, 2, "%");
        range(ui, "Roundness", &mut r.roundness, 0..=50, 5, "%");
        range(ui, "Outline", &mut r.border, 0..=4, 1, " pt");
    }
    ui.add_space(8.0);
    small_caps(ui, colors, "Motion");
    let m = &mut voice.motion;
    segmented(ui, colors, &mut m.curve, &crate::voice::CURVES);
    millis(ui, "Glide time", &mut m.glide_ms, 60..=1500, 20);
    range(ui, "Fade over when going back more than (tenths of a word)", &mut m.fade_back, 3..=50, 1, "");
    range(ui, "Fade over when jumping ahead more than (words)", &mut m.far_jump, 3..=40, 1, "");
    range(ui, "Run ahead at your pace at most (tenths of a word)", &mut m.max_ahead, 0..=30, 1, "");
    millis(ui, "Wait before easing back", &mut m.wait_ms, 0..=5000, 100);
    check(ui, &mut m.fade_at_ayah_end, "Fade out at ayah ends (experimental)");
    ui.add_space(8.0);
    small_caps(ui, colors, "Listener");
    ui.add(egui::TextEdit::singleline(&mut voice.listener).hint_text("~/.local/share/quran-voice").desired_width(f32::INFINITY));
    help(ui, colors, "The folder quran-voice's scripts/install-listener.sh installs to. A listener already running is joined instead.");
}
