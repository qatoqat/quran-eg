//! User settings: the model, its value mappings (sizes, spacing) and
//! persistence. Ported from quran-dx; the JSON is the same shape, so a settings
//! file written by either app parses in the other. What was CSS there (custom
//! properties, `color-mix` palettes) is plain numbers here; colours are built by
//! the app's theme code.

use serde::{Deserialize, Serialize};

use crate::api;

pub mod palette;
pub mod storage;
pub mod translations;

/// Colours of the app. `Auto` follows the system's light or dark setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
    Sepia,
    /// The reader's own background and accent ([`CustomTheme`]); every other
    /// colour is a shade worked out from those two.
    Custom,
}

/// The two colours a custom theme is built from, as `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomTheme {
    pub background: String,
    pub accent: String,
}

pub const DEFAULT_CUSTOM_BACKGROUND: &str = "#101a2b";
pub const DEFAULT_ACCENT: &str = "#2ca4ab";

impl Default for CustomTheme {
    fn default() -> Self {
        Self { background: DEFAULT_CUSTOM_BACKGROUND.into(), accent: DEFAULT_ACCENT.into() }
    }
}

/// Relative luminance (WCAG) of a `#rrggbb` colour: 0 = black, 1 = white.
pub fn luminance(color: &str) -> Option<f64> {
    let hex = sanitize_color(color)?;
    let channel = |i: usize| {
        let c = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    Some(0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5))
}

impl CustomTheme {
    /// The background as `#rrggbb`, or the default if it isn't a colour.
    pub fn background(&self) -> String {
        sanitize_color(&self.background).unwrap_or_else(|| DEFAULT_CUSTOM_BACKGROUND.into())
    }

    /// The accent as `#rrggbb`, or the default if it isn't a colour.
    pub fn accent(&self) -> String {
        sanitize_color(&self.accent).unwrap_or_else(|| DEFAULT_ACCENT.into())
    }

    /// A background on which white text reads better than black.
    pub fn is_dark(&self) -> bool {
        luminance(&self.background()).unwrap_or(0.0) < 0.179
    }
}

impl Theme {
    pub const ALL: [Theme; 5] = [Theme::Auto, Theme::Light, Theme::Dark, Theme::Sepia, Theme::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Auto => "Auto",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
            Theme::Sepia => "Sepia",
            Theme::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PointerShape {
    /// A filled disc.
    #[default]
    Circle,
    /// An outline.
    Ring,
}

/// A reading pointer that replaces the mouse cursor below the header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pointer {
    pub enabled: bool,
    pub shape: PointerShape,
    /// `#rrggbb`.
    pub color: String,
    /// Diameter in tenths of a mushaf line's height, 1..=10.
    pub size: u8,
    /// Percent, 5..=100.
    pub opacity: u8,
    /// Number of fading copies that follow it, 0..=10.
    pub trail: u8,
    /// How far those copies fall behind, 1..=10 (5 = default look).
    pub trail_distance: u8,
    /// Softness of its edge, 0..=10.
    pub blur: u8,
    /// How much the colours of the text under it are intensified, 0..=10.
    pub glow: u8,
    /// How much it shrinks while the mouse moves fast, 0..=10.
    pub shrink: u8,
}

pub const DEFAULT_POINTER_COLOR: &str = "#ffffff";

impl Default for Pointer {
    fn default() -> Self {
        Self {
            enabled: false,
            shape: PointerShape::Circle,
            color: DEFAULT_POINTER_COLOR.into(),
            size: 5,
            opacity: 50,
            trail: 0,
            trail_distance: 5,
            blur: 0,
            glow: 0,
            shrink: 0,
        }
    }
}

/// The voice pointer: the place being recited, as quran-voice's listener hears it (`qv listen`, which
/// quran-eg starts or joins on ws://127.0.0.1:8765). The same settings as the quran.com extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Voice {
    /// The dot on the letter being read: the reading pointer's look (its size in tenths of the text
    /// size: 8 is from the baseline to the top of the letters); `enabled` shows it.
    pub pointer: Pointer,
    /// A ring's line, points, 1..=20.
    pub ring: u8,
    /// Centre above the baseline in tenths of the text size (2: between the baseline and the head of a
    /// fa), 0..=10.
    pub lift: u8,
    pub range: VoiceRange,
    pub motion: VoiceMotion,
    /// Keep the line being read in view.
    pub auto_scroll: bool,
    /// Stop listening when the reader closes with ṣadaqa-llāhu-l-ʿaẓīm (or ṣadaqa-llāhu-l-ʿaliyyu-l-ʿaẓīm).
    pub stop_on_closing: bool,
    /// Folder of the installed listener (`qv`, `model/`, `ref.json`, from quran-voice's
    /// `scripts/install-listener.sh`); empty = `<data dir>/quran-voice`.
    pub listener: String,
}

pub const DEFAULT_VOICE_COLOR: &str = "#ff8c00";

impl Default for Voice {
    fn default() -> Self {
        Self {
            pointer: Pointer { enabled: true, shape: PointerShape::Ring, color: DEFAULT_VOICE_COLOR.into(), size: 8, opacity: 90, ..Pointer::default() },
            ring: 10,
            lift: 2,
            range: VoiceRange::default(),
            motion: VoiceMotion::default(),
            auto_scroll: true,
            stop_on_closing: true,
            listener: String::new(),
        }
    }
}

/// The accuracy range: a rounded rectangle over the words the listener could be on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceRange {
    pub enabled: bool,
    /// `#rrggbb`.
    pub color: String,
    /// Height in tenths of a line's height, 4..=20.
    pub size: u8,
    /// How many words of uncertainty its width covers, in tenths, 5..=30.
    pub spread: u8,
    /// Percent, 0..=100.
    pub opacity: u8,
    /// Corner radius in percent of its height, 0..=50 (50 = pill).
    pub roundness: u8,
    /// Outline width in points, 0..=4.
    pub border: u8,
}

impl Default for VoiceRange {
    fn default() -> Self {
        Self { enabled: true, color: DEFAULT_VOICE_COLOR.into(), size: 8, spread: 10, opacity: 10, roundness: 35, border: 0 }
    }
}

/// How the voice pointer eases toward a new place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VoiceCurve {
    /// Exponential approach: fast at first, settling softly.
    #[default]
    Smooth,
    /// A critically damped spring: starts gently, no overshoot.
    Spring,
    Linear,
    EaseOut,
    EaseInOut,
}

/// How the voice pointer moves between the listener's updates (every ~320 ms, often in bursts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceMotion {
    pub curve: VoiceCurve,
    /// Glide time in ms, 60..=1500.
    pub glide_ms: u16,
    /// Going back more than this (tenths of a word, 3..=50) fades over instead of moving back.
    pub fade_back: u8,
    /// Going forward more than this many words (3..=40) fades over instead of gliding.
    pub far_jump: u8,
    /// How far it may run ahead of what was heard, at the reader's pace (tenths of a word, 0..=30).
    pub max_ahead: u8,
    /// How long it waits for the reader when ahead before easing back, ms 0..=5000.
    pub wait_ms: u16,
    /// Fade out at an ayah's end while the reader pauses there, and in where they go on (experimental).
    pub fade_at_ayah_end: bool,
}

impl Default for VoiceMotion {
    fn default() -> Self {
        Self { curve: VoiceCurve::Smooth, glide_ms: 320, fade_back: 10, far_jump: 12, max_ahead: 5, wait_ms: 1500, fade_at_ayah_end: true }
    }
}

/// Share of the gap a trail copy of the pointer closes per frame (at 60 frames a
/// second), for a 1..=10 distance: 1 → 0.55 (tight), 5 → 0.35 (default), 10 → 0.1 (long).
pub fn trail_lag(distance: u8) -> f32 {
    (60.0 - 5.0 * f32::from(distance.clamp(1, 10))) / 100.0
}

/// `#rrggbb` (lower case) if `color` is one, so it can go into a stylesheet as is.
pub fn sanitize_color(color: &str) -> Option<String> {
    let hex = color.trim().strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", hex.to_ascii_lowercase()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WbwDisplay {
    /// Show word translation/transliteration on hover/tap.
    Tooltip,
    /// Show it under each word (default: like quran.com's word-by-word view).
    #[default]
    Inline,
}

/// Which King Fahd (KFGQPC) page fonts draw the 15-line Madani mushaf
/// (Uthman Taha's calligraphy). Both use the API's `code_v2` glyph codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MushafFont {
    /// QCF V4 tajweed-coloured fonts (COLRv1).
    #[default]
    TajweedV4,
    /// QCF V2 single-colour fonts.
    MadaniV2,
}

impl MushafFont {
    /// `mushaf=` id for the API, so `page_number`/`line_number` match the font's layout.
    pub fn mushaf_id(self) -> u32 {
        match self {
            MushafFont::TajweedV4 => api::query::mushaf::QCF_TAJWEED_V4,
            MushafFont::MadaniV2 => api::query::mushaf::QCF_V2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MushafFont::TajweedV4 => "Tajweed colours (QCF V4)",
            MushafFont::MadaniV2 => "Plain (QCF V2)",
        }
    }
}

/// Where the QCF fonts are fetched from, tried in order. Each must contain the
/// quran.com `fonts/quran/...` layout (`hafs/v4/colrv1/woff2/p1.woff2`, ...).
/// A user-set `Settings::font_base_url` is tried first.
pub const DEFAULT_FONT_BASES: &[&str] =
    &["https://verses.quran.foundation/fonts/quran", "https://quran.com/fonts/quran"];

/// Language of chapter names, word meanings and search (the app is English-only).
pub const LANGUAGE: &str = "en";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WordByWord {
    pub show_translation: bool,
    pub show_transliteration: bool,
    pub display: WbwDisplay,
}

impl Default for WordByWord {
    fn default() -> Self {
        Self { show_translation: true, show_transliteration: false, display: WbwDisplay::Inline }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Selected translation resource ids, in display order.
    pub translations: Vec<u32>,
    /// 1..=10 scale for the Arabic text.
    pub arabic_font_scale: u8,
    /// 1..=10 scale for translations.
    pub translation_font_scale: u8,
    pub word_by_word: WordByWord,
    /// Font set for the 15-line mushaf pages.
    pub mushaf_font: MushafFont,
    /// Show each page's verse translations under the page.
    pub show_page_translation: bool,
    /// Optional custom font base URL (e.g. a self-hosted copy); empty = defaults.
    pub font_base_url: String,
    /// Space between words on mushaf lines, 0..=10 (2 = default look).
    pub word_spacing: u8,
    /// Space under each mushaf line (below its word meanings, when shown), 0..=10 (4 = default look).
    pub line_spacing: u8,
    /// Space between a word and its meaning underneath, 0..=10 (4 = default look).
    pub meaning_spacing: u8,
    /// Extra width of the content column, 0..=10 steps (0 = default width).
    pub content_width: u8,
    pub theme: Theme,
    pub custom_theme: CustomTheme,
    pub pointer: Pointer,
    pub voice: Voice,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Saheeh International (the API no longer serves 131, The Clear Quran).
            translations: vec![20],
            arabic_font_scale: 4,
            translation_font_scale: 3,
            word_by_word: WordByWord::default(),
            mushaf_font: MushafFont::TajweedV4,
            show_page_translation: false,
            font_base_url: String::new(),
            word_spacing: DEFAULT_WORD_SPACING,
            line_spacing: DEFAULT_LINE_SPACING,
            meaning_spacing: DEFAULT_MEANING_SPACING,
            content_width: 0,
            theme: Theme::default(),
            custom_theme: CustomTheme::default(),
            pointer: Pointer::default(),
            voice: Voice::default(),
        }
    }
}

impl Settings {
    /// Query to use for every `/verses/by_*` request.
    pub fn verse_query(&self) -> api::VerseQuery {
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        api::VerseQuery {
            language: LANGUAGE.into(),
            translations: self.translations.clone(),
            words: true,
            fields: strings(&["text_uthmani", "qpc_uthmani_hafs"]),
            // The API has no per-word `qpc_uthmani_hafs`; the verse-level text is split instead.
            word_fields: strings(&["code_v2", "v2_page", "page_number", "line_number", "text_uthmani"]),
            translation_fields: strings(&["resource_name", "language_name"]),
            page: 1,
            per_page: 50,
            mushaf: Some(self.mushaf_font.mushaf_id()),
        }
    }

    /// Font base URLs to try, in order (custom first).
    pub fn font_bases(&self) -> Vec<String> {
        let custom = self.font_base_url.trim().trim_end_matches('/');
        let mut out: Vec<String> = Vec::new();
        if !custom.is_empty() {
            out.push(custom.to_string());
        }
        out.extend(DEFAULT_FONT_BASES.iter().map(|s| s.to_string()));
        out
    }
}

/// CSS px per rem in quran-dx; sizes here are in egui points, which are the same unit.
pub const REM: f32 = 16.0;

/// Arabic size in rem for a 1..=10 scale (clamped), linear between 1 → 1.6,
/// 4 (default) → 2.4 and 10 → 4.4, rounded to 0.01.
pub fn arabic_font_rem(scale: u8) -> f32 {
    let s = u32::from(scale.clamp(1, 10));
    // Hundredths of a rem; `(2x + y) / 2y` rounds x/y to the nearest integer.
    let centi = if s <= 4 { 160 + (2 * 80 * (s - 1) + 3) / 6 } else { 240 + (2 * 200 * (s - 4) + 6) / 12 };
    centi as f32 / 100.0
}

/// Translation text size in rem for a 1..=10 scale (clamped). 3 (default) → 1.05, 1 → 0.8, 10 → 1.925.
pub fn translation_font_rem(scale: u8) -> f32 {
    let s = u32::from(scale.clamp(1, 10));
    (800 + 125 * (s - 1)) as f32 / 1000.0
}

/// Size of a mushaf glyph's em in points: 1.3 × the Arabic size (scale 4, the
/// default, is about quran.com's default). A line of Arabic is [`MUSHAF_LINE`] of it tall.
pub fn mushaf_em(scale: u8) -> f32 {
    arabic_font_rem(scale) * REM * 1.3
}

/// Height of a line of Arabic, in glyph em.
pub const MUSHAF_LINE: f32 = 1.5;

/// Size of the word-by-word meanings in points (before the shared fit scale).
pub fn meaning_font_px(scale: u8) -> f32 {
    translation_font_rem(scale) * REM * 0.72
}

/// Widest a word's meaning may be before it wraps, in points (before the fit scale).
pub const MEANING_MAX_WIDTH: f32 = 7.5 * REM;

impl Settings {
    /// Whether the theme is dark (`Some`), or follows the system (`None`).
    pub fn dark(&self) -> Option<bool> {
        match self.theme {
            Theme::Auto => None,
            Theme::Dark => Some(true),
            Theme::Light | Theme::Sepia => Some(false),
            Theme::Custom => Some(self.custom_theme.is_dark()),
        }
    }
}

/// Width of the content column (header and pages) in CSS px: 0 → 960 (default),
/// then 160 more per step, up to 2560 at 10. The window still limits it.
pub fn content_width_px(step: u8) -> u32 {
    960 + 160 * u32::from(step.min(10))
}

pub const DEFAULT_WORD_SPACING: u8 = 2;
pub const DEFAULT_LINE_SPACING: u8 = 4;
pub const DEFAULT_MEANING_SPACING: u8 = 4;

/// Minimum gap between words on a mushaf line, in glyph em: 0 → 0 … 2 → 0.12 (default) … 10 → 0.6.
pub fn word_gap(step: u8) -> f32 {
    f32::from(step.min(10)) * 6.0 / 100.0
}

/// Space under a mushaf line, in glyph em: 0 → 0 … 4 → 0.4 (default) … 10 → 1.
pub fn line_gap(step: u8) -> f32 {
    f32::from(step.min(10)) / 10.0
}

/// Space between a word and its meaning, in glyph em: 0 → -0.12 (tucked
/// under the word) … 4 → 0.12 (default) … 10 → 0.48.
pub fn meaning_gap(step: u8) -> f32 {
    (f32::from(step.min(10)) * 6.0 - 12.0) / 100.0
}

/// Right-to-left languages, matched by ISO code (`ur`, `ar-SA`) or by English
/// name as used in the API's `language_name` (`urdu`, `Arabic`).
pub fn is_rtl_language(lang: &str) -> bool {
    const CODES: &[&str] = &["ar", "ur", "fa", "he", "iw", "ps", "sd", "ug", "dv", "yi", "ckb", "ks", "bal"];
    const NAMES: &[&str] = &[
        "arabic", "urdu", "persian", "farsi", "hebrew", "pashto", "sindhi", "uyghur", "uighur",
        "divehi", "dhivehi", "yiddish", "kurdish", "sorani", "kashmiri", "balochi",
    ];
    let lang = lang.trim().to_ascii_lowercase();
    let code = lang.split(['-', '_']).next().unwrap_or("");
    CODES.contains(&code) || NAMES.contains(&lang.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn longer_distance_means_slower_catch_up() {
        assert!(close(trail_lag(1), 0.55) && close(trail_lag(5), 0.35) && close(trail_lag(10), 0.1));
        assert!(close(trail_lag(0), 0.55) && close(trail_lag(99), 0.1));
    }

    #[test]
    fn spacing_mapping() {
        assert!(close(word_gap(0), 0.0) && close(word_gap(DEFAULT_WORD_SPACING), 0.12));
        assert!(close(word_gap(10), 0.6) && close(word_gap(99), 0.6));
        assert!(close(line_gap(DEFAULT_LINE_SPACING), 0.4));
        assert!(close(line_gap(0), 0.0) && close(line_gap(10), 1.0) && close(line_gap(99), 1.0));
        assert!(close(meaning_gap(DEFAULT_MEANING_SPACING), 0.12));
        assert!(close(meaning_gap(0), -0.12) && close(meaning_gap(2), 0.0) && close(meaning_gap(10), 0.48));
        assert_eq!((content_width_px(0), content_width_px(3), content_width_px(99)), (960, 1440, 2560));
    }

    #[test]
    fn themes_and_colours() {
        let mut s = Settings { theme: Theme::Custom, ..Settings::default() };
        assert_eq!(s.dark(), Some(true), "the default custom background is dark");
        // A light background flips the text to dark.
        s.custom_theme = CustomTheme { background: "#F3E9D2".into(), accent: "#ffd54a".into() };
        assert_eq!(s.dark(), Some(false));
        assert_eq!((s.custom_theme.background(), s.custom_theme.accent()), ("#f3e9d2".into(), "#ffd54a".into()));
        // Bad colours fall back to the defaults.
        s.custom_theme = CustomTheme { background: "url(x)".into(), accent: "} *{".into() };
        assert_eq!(s.custom_theme.background(), DEFAULT_CUSTOM_BACKGROUND);
        assert_eq!(s.custom_theme.accent(), DEFAULT_ACCENT);
        assert_eq!((luminance("#000000"), luminance("#ffffff")), (Some(0.0), Some(1.0)));
        assert!(luminance("nope").is_none());
        let themes = |t| Settings { theme: t, ..Settings::default() }.dark();
        assert_eq!((themes(Theme::Auto), themes(Theme::Dark), themes(Theme::Sepia)), (None, Some(true), Some(false)));
        assert_eq!(themes(Theme::Light), Some(false));
        assert_eq!(sanitize_color(" #AbCdEf "), Some("#abcdef".into()));
        assert_eq!((sanitize_color("#fff"), sanitize_color("abcdef"), sanitize_color("#12345g")), (None, None, None));
    }

    #[test]
    fn font_scale_mapping() {
        for (scale, rem) in [(1, 1.6), (4, 2.4), (10, 4.4), (0, 1.6), (200, 4.4), (2, 1.87), (7, 3.4)] {
            assert!(close(arabic_font_rem(scale), rem), "{scale}: {}", arabic_font_rem(scale));
        }
        for (scale, rem) in [(3, 1.05), (1, 0.8), (2, 0.925), (10, 1.925)] {
            assert!(close(translation_font_rem(scale), rem), "{scale}");
        }
        for s in 1..10u8 {
            // Strictly increasing, in even-ish steps (no jumps over 0.4rem).
            let (a, b) = (arabic_font_rem(s), arabic_font_rem(s + 1));
            assert!(a < b && b - a <= 0.4 + 1e-5, "{s}: {a} -> {b}");
            assert!(translation_font_rem(s) < translation_font_rem(s + 1));
        }
        // Default sizes in points: a 49.92 pt glyph em, 12.096 pt meanings.
        assert!(close(mushaf_em(4), 49.92) && close(meaning_font_px(3), 12.096));
    }

    #[test]
    fn verse_query_asks_for_the_mushaf_fields() {
        let mut s = Settings::default();
        let q = s.verse_query();
        assert_eq!((q.mushaf, q.per_page, q.words), (Some(19), 50, true));
        assert_eq!(q.word_fields, ["code_v2", "v2_page", "page_number", "line_number", "text_uthmani"]);
        assert_eq!(q.fields, ["text_uthmani", "qpc_uthmani_hafs"]);
        s.mushaf_font = MushafFont::MadaniV2;
        assert_eq!(s.verse_query().mushaf, Some(1));
    }

    #[test]
    fn custom_font_base_is_tried_first() {
        let mut s = Settings::default();
        assert_eq!(s.font_bases(), DEFAULT_FONT_BASES);
        s.font_base_url = " /fonts/quran/ ".into();
        assert_eq!(s.font_bases()[0], "/fonts/quran");
        assert_eq!(s.font_bases().len(), DEFAULT_FONT_BASES.len() + 1);
    }

    #[test]
    fn rtl_detection() {
        for l in ["ar", "ur", "fa", "ar-SA", "Urdu", "arabic", "Persian", " urdu "] {
            assert!(is_rtl_language(l), "{l}");
        }
        for l in ["en", "id", "english", "French", "", "bangla", "turkish"] {
            assert!(!is_rtl_language(l), "{l}");
        }
    }
}
