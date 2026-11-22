//! Settings persistence: a JSON file under the user's config directory, next to
//! the location the app was last at (view and reading position); in a browser,
//! `localStorage` (and the location is the URL).
//!
//! All parsing/serialization lives in plain functions so it can be unit-tested.
//! Missing or corrupt data yields `None`; nothing here panics.

use serde_json::Value;

use super::Settings;

/// Serialize settings to the JSON stored on disk / in localStorage.
pub fn to_json(settings: &Settings) -> String {
    // Serializing plain data structs can't fail; fall back to an empty object just in case.
    serde_json::to_string(settings).unwrap_or_else(|_| "{}".into())
}

/// Parse stored JSON. Unknown fields are ignored and missing ones take defaults;
/// malformed JSON or wrongly-typed values yield `None`, except that non-numeric
/// entries of the `translations` array are dropped. Settings of earlier versions
/// are carried over ([`migrate`]). Values are normalized.
pub fn from_json(raw: &str) -> Option<Settings> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut value: Value = serde_json::from_str(raw).ok()?;
    if let Some(list) = value.get_mut("translations").and_then(|t| t.as_array_mut()) {
        // Keep only resource ids (numbers that fit a u32); their language can't be
        // verified offline, so the settings page lets the user remove stray ones.
        list.retain(|v| v.as_u64().is_some_and(|n| u32::try_from(n).is_ok()));
    }
    migrate(&mut value);
    let mut settings: Settings = serde_json::from_value(value).ok()?;
    normalize(&mut settings);
    Some(settings)
}

/// Settings an earlier version stored, in today's terms. Done on the JSON, not with serde aliases:
/// a file holding an old key and its new one must still parse (the new one wins).
fn migrate(value: &mut Value) {
    if let Some(pointer) = value.get_mut("pointer") {
        trail_to_smudge(pointer);
    }
    let Some(voice) = value.get_mut("voice") else { return };
    if let Some(pointer) = voice.get_mut("pointer") {
        trail_to_smudge(pointer);
    }
    if let Some(motion) = voice.get_mut("motion") {
        one_glide_to_three(motion);
    }
}

/// A pointer's trail of fading copies (`trail`, `trail_distance`) becomes its smudge. Off stays off,
/// and a trail gets 3 more strength: its first copy had half the pointer's opacity or more whatever
/// their number, where the smudge starts at 7 % of it per step, so a short trail would turn into a
/// barely visible smear.
fn trail_to_smudge(pointer: &mut Value) {
    let Some(pointer) = pointer.as_object_mut() else { return };
    if !pointer.contains_key("smudge")
        && let Some(trail) = pointer.get("trail").and_then(Value::as_u64)
    {
        pointer.insert("smudge".into(), (if trail == 0 { 0 } else { trail.min(7) + 3 }).into());
    }
    if !pointer.contains_key("smudge_distance")
        && let Some(distance) = pointer.get("trail_distance").and_then(Value::as_u64)
    {
        pointer.insert("smudge_distance".into(), distance.clamp(1, 10).into());
    }
}

/// The voice pointer's one curve and glide time (`curve`, `glide_ms`) become its pointer's and both
/// of its range's edges' (the range rode on the pointer, so the look stays).
fn one_glide_to_three(motion: &mut Value) {
    let Some(motion) = motion.as_object_mut() else { return };
    if motion.contains_key("pointer") || !(motion.contains_key("curve") || motion.contains_key("glide_ms")) {
        return;
    }
    let curve = motion.get("curve").cloned().unwrap_or_else(|| "Smooth".into());
    let ms = motion.get("glide_ms").cloned().unwrap_or_else(|| 320.into());
    let glide = serde_json::json!({ "curve": curve, "ms": ms });
    for part in ["pointer", "range_start", "range_end"] {
        motion.entry(part).or_insert_with(|| glide.clone());
    }
}

/// Clamp/repair values that may have been hand-edited or written by older versions.
pub fn normalize(s: &mut Settings) {
    s.arabic_font_scale = s.arabic_font_scale.clamp(1, 10);
    s.translation_font_scale = s.translation_font_scale.clamp(1, 10);
    s.word_spacing = s.word_spacing.min(10);
    s.line_spacing = s.line_spacing.min(10);
    s.meaning_spacing = s.meaning_spacing.min(10);
    s.content_width = s.content_width.min(10);
    let custom = &mut s.custom_theme;
    custom.background = super::sanitize_color(&custom.background).unwrap_or_else(|| super::DEFAULT_CUSTOM_BACKGROUND.into());
    custom.accent = super::sanitize_color(&custom.accent).unwrap_or_else(|| super::DEFAULT_ACCENT.into());
    let pointer = &mut s.pointer;
    pointer.size = pointer.size.clamp(1, 10);
    pointer.opacity = pointer.opacity.clamp(5, 100);
    pointer.smudge = pointer.smudge.min(10);
    pointer.smudge_distance = pointer.smudge_distance.clamp(1, 10);
    pointer.blur = pointer.blur.min(10);
    pointer.glow = pointer.glow.min(10);
    pointer.shrink = pointer.shrink.min(10);
    pointer.color = super::sanitize_color(&pointer.color).unwrap_or_else(|| super::DEFAULT_POINTER_COLOR.into());
    let voice = &mut s.voice;
    voice.ring = voice.ring.clamp(1, 40);
    voice.lift = voice.lift.min(10);
    voice.far_wait_ms = voice.far_wait_ms.min(20000);
    voice.model = sanitize_model(&voice.model).unwrap_or_default();
    let dot = &mut voice.pointer;
    dot.size = dot.size.clamp(2, 20);
    dot.opacity = dot.opacity.clamp(5, 100);
    dot.smudge = dot.smudge.min(10);
    dot.smudge_distance = dot.smudge_distance.clamp(1, 10);
    dot.blur = dot.blur.min(10);
    dot.glow = dot.glow.min(10);
    dot.shrink = dot.shrink.min(10);
    dot.color = super::sanitize_color(&dot.color).unwrap_or_else(|| super::DEFAULT_VOICE_COLOR.into());
    let range = &mut voice.range;
    range.color = super::sanitize_color(&range.color).unwrap_or_else(|| super::DEFAULT_VOICE_COLOR.into());
    range.size = range.size.clamp(4, 20);
    range.spread = range.spread.clamp(5, 30);
    range.opacity = range.opacity.min(100);
    range.roundness = range.roundness.min(50);
    range.border = range.border.min(12);
    range.blur = range.blur.min(10);
    let skipped = &mut voice.skipped;
    skipped.color = super::sanitize_color(&skipped.color).unwrap_or_else(|| super::DEFAULT_SKIPPED_COLOR.into());
    skipped.size = skipped.size.clamp(4, 20);
    skipped.opacity = skipped.opacity.min(100);
    skipped.roundness = skipped.roundness.min(50);
    skipped.border = skipped.border.min(12);
    skipped.blur = skipped.blur.min(10);
    skipped.min_gap = skipped.min_gap.clamp(1, 10);
    let similar = &mut voice.similar;
    similar.color = super::sanitize_color(&similar.color).unwrap_or_else(|| super::DEFAULT_SIMILAR_COLOR.into());
    similar.size = similar.size.clamp(4, 20);
    similar.opacity = similar.opacity.min(100);
    similar.roundness = similar.roundness.min(50);
    similar.border = similar.border.min(12);
    similar.blur = similar.blur.min(10);
    let motion = &mut voice.motion;
    for glide in [&mut motion.pointer, &mut motion.range_start, &mut motion.range_end] {
        glide.ms = glide.ms.clamp(60, 1500);
    }
    motion.fade_back = motion.fade_back.clamp(3, 50);
    motion.far_jump = motion.far_jump.clamp(3, 40);
    motion.max_ahead = motion.max_ahead.min(30);
    motion.wait_ms = motion.wait_ms.min(5000);
    motion.ayah_fade_delay_ms = motion.ayah_fade_delay_ms.min(10000);
    motion.quiet_dim = motion.quiet_dim.min(100);
    motion.stale_wait_ms = motion.stale_wait_ms.min(2000);
    let mut seen = std::collections::HashSet::new();
    s.translations.retain(|id| seen.insert(*id));
    s.font_base_url = sanitize_font_base_url(&s.font_base_url).unwrap_or_default();
}

/// A model's name as the settings keep it: a folder right inside the listener's `models/` (no path), trimmed; `None`
/// for anything else, which is the listener's own `model/`.
pub fn sanitize_model(raw: &str) -> Option<String> {
    let name = raw.trim();
    let bad = |c: char| c.is_control() || c == '/' || c == '\\';
    (!name.is_empty() && name != "." && name != ".." && !name.chars().any(bad)).then(|| name.to_string())
}

/// Validate a custom font base URL: trimmed, and either an `http(s)://host...`
/// URL or an absolute path (`/fonts/quran`; here a local directory holding a
/// copy of quran.com's `fonts/quran` folder, where the web app meant a path on its own site). Characters that could break out of
/// a CSS `url("...")` or JS string (quotes, backslashes, parentheses, whitespace,
/// angle brackets, control characters) are rejected. Trailing `/` are removed.
/// Returns `None` if the value is empty or not acceptable.
pub fn sanitize_font_base_url(raw: &str) -> Option<String> {
    let url = raw.trim().trim_end_matches('/');
    if url.is_empty() || url.len() > 2048 {
        return None;
    }
    let bad = |c: char| c.is_whitespace() || c.is_control() || "\"'\\()<>`".contains(c);
    if url.chars().any(bad) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    let ok = match rest {
        // Needs a host.
        Some(rest) => !rest.is_empty() && !rest.starts_with('/'),
        // Same-origin absolute path; `//host` (protocol-relative) is not a path.
        None => url.starts_with('/') && !url.starts_with("//"),
    };
    ok.then(|| url.to_string())
}

/// Load persisted settings.
pub fn load() -> Option<Settings> {
    backend::load()
}

/// Persist settings. Failures are ignored: settings just won't persist
/// (read-only home, full disk).
pub fn save(settings: &Settings) {
    backend::save(settings);
}

/// Where the app was last (`/2#2:142`), to start there next time.
pub fn load_location() -> Option<String> {
    backend::load_location()
}

pub fn save_location(location: &str) {
    backend::save_location(location);
}

/// Directory name under the config and cache directories.
pub const APP_DIR: &str = "quran-eg";

/// `<config dir>/quran-eg/settings.json` and `.../location`.
#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::Settings;

    fn file(name: &str) -> Option<PathBuf> {
        // Tests must not read or overwrite the user's real files.
        if cfg!(test) {
            return None;
        }
        Some(dirs::config_dir()?.join(super::APP_DIR).join(name))
    }

    pub fn load() -> Option<Settings> {
        load_from_path(&file("settings.json")?)
    }

    pub fn save(settings: &Settings) {
        if let Some(path) = file("settings.json") {
            let _ = save_to_path(&path, settings);
        }
    }

    pub fn load_location() -> Option<String> {
        let location = std::fs::read_to_string(file("location")?).ok()?;
        let location = location.trim();
        location.starts_with('/').then(|| location.to_string())
    }

    pub fn save_location(location: &str) {
        if let Some(path) = file("location") {
            let _ = write_atomic(&path, location.as_bytes());
        }
    }

    /// Read settings from a JSON file. Missing/unreadable/corrupt file → `None`.
    pub fn load_from_path(path: &Path) -> Option<Settings> {
        super::from_json(&std::fs::read_to_string(path).ok()?)
    }

    pub fn save_to_path(path: &Path, settings: &Settings) -> std::io::Result<()> {
        write_atomic(path, super::to_json(settings).as_bytes())
    }

    /// Atomically write `bytes` to `path`: create the parent directory, write a
    /// sibling temp file of this write's own (two threads or processes writing
    /// the same file never share one), then rename it over the target.
    pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tmp_name.push(format!(".{}-{n}.tmp", std::process::id()));
        let tmp = path.with_file_name(tmp_name);
        let result = (|| {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

/// Browser: `localStorage` for the settings. The location lives in the URL.
#[cfg(target_arch = "wasm32")]
mod backend {
    use super::Settings;

    const STORAGE_KEY: &str = "quran-eg-settings";

    fn storage() -> Option<web_sys::Storage> {
        // `local_storage()` errors e.g. in sandboxed iframes / privacy modes.
        web_sys::window()?.local_storage().ok().flatten()
    }

    pub fn load() -> Option<Settings> {
        super::from_json(&storage()?.get_item(STORAGE_KEY).ok().flatten()?)
    }

    pub fn save(settings: &Settings) {
        if let Some(s) = storage() {
            let _ = s.set_item(STORAGE_KEY, &super::to_json(settings));
        }
    }

    pub fn load_location() -> Option<String> {
        None
    }

    pub fn save_location(_location: &str) {}
}

#[cfg(not(target_arch = "wasm32"))]
pub use backend::write_atomic;

#[cfg(test)]
mod tests {
    use super::backend::{load_from_path, save_to_path};
    use super::*;
    use crate::settings::{MushafFont, VoiceCurve, VoiceGlide, WbwDisplay};

    #[test]
    fn real_files_are_out_of_reach_in_tests() {
        // `load`/`save` and the location go nowhere under `cfg(test)`.
        save(&Settings { arabic_font_scale: 9, ..Default::default() });
        save_location("/2#2:142");
        assert_eq!((load(), load_location()), (None, None));
    }

    #[test]
    fn roundtrip() {
        let s = Settings {
            translations: vec![20, 131],
            arabic_font_scale: 9,
            word_by_word: crate::settings::WordByWord {
                display: WbwDisplay::Tooltip,
                show_transliteration: true,
                show_translation: false,
            },
            mushaf_font: MushafFont::MadaniV2,
            show_page_translation: true,
            font_base_url: "https://fonts.example.org/quran".into(),
            ..Default::default()
        };
        assert_eq!(from_json(&to_json(&s)), Some(s));
    }

    #[test]
    fn earlier_pointer_and_motion_settings_carry_over() {
        let old = r#"{"pointer":{"trail":3,"trail_distance":7},"voice":{"pointer":{"trail":0,"trail_distance":2},"motion":{"curve":"Linear","glide_ms":500,"fade_at_ayah_end":true}}}"#;
        let s = from_json(old).unwrap();
        assert_eq!((s.pointer.smudge, s.pointer.smudge_distance), (6, 7));
        assert_eq!((s.voice.pointer.smudge, s.voice.pointer.smudge_distance), (0, 2));
        let glide = VoiceGlide { curve: VoiceCurve::Linear, ms: 500 };
        let m = &s.voice.motion;
        assert_eq!((m.pointer, m.range_start, m.range_end), (glide, glide, glide));
        assert!(m.fade_at_ayah_end);
        assert_eq!((m.ayah_fade_delay_ms, s.voice.far_wait_ms, s.voice.range.blur), (1000, 5000, 0));
        assert!(s.voice.tuned, "settings of before get tuned movement, their own motion kept for when it is off");
        // The voice pointer's longest trail and thickest ring of before.
        let s = from_json(r#"{"voice":{"pointer":{"trail":4,"trail_distance":7},"ring":20}}"#).unwrap();
        assert_eq!((s.voice.pointer.smudge, s.voice.pointer.smudge_distance, s.voice.ring), (7, 7, 20));
        let s = from_json(r#"{"pointer":{"trail":10,"trail_distance":0}}"#).unwrap();
        assert_eq!((s.pointer.smudge, s.pointer.smudge_distance), (10, 1));
        // Absurd hand-edited values neither overflow nor wrap.
        let s = from_json(r#"{"pointer":{"trail":18446744073709551615,"trail_distance":18446744073709551615}}"#).unwrap();
        assert_eq!((s.pointer.smudge, s.pointer.smudge_distance), (10, 10));
        // Only the glide time stored: the default curve with that time.
        let s = from_json(r#"{"voice":{"motion":{"glide_ms":700}}}"#).unwrap();
        assert_eq!(s.voice.motion.range_end, VoiceGlide { curve: VoiceCurve::Smooth, ms: 700 });
        assert_eq!(s.voice.motion.pointer, s.voice.motion.range_start);
    }

    #[test]
    fn new_keys_win_over_old_ones_beside_them() {
        let both = r#"{"pointer":{"trail":3,"smudge":1,"trail_distance":9},"voice":{"motion":{"curve":"Spring","glide_ms":900,"pointer":{"curve":"Linear","ms":200}}}}"#;
        let s = from_json(both).unwrap();
        assert_eq!((s.pointer.smudge, s.pointer.smudge_distance), (1, 9));
        assert_eq!(s.voice.motion.pointer, VoiceGlide { curve: VoiceCurve::Linear, ms: 200 });
        assert_eq!(s.voice.motion.range_start, VoiceGlide::default(), "the new shape is taken as it is");
    }

    #[test]
    fn a_removed_curve_loads_as_smooth() {
        let old = r#"{"voice":{"motion":{"pointer":{"curve":"EaseOut","ms":200},"range_start":{"curve":"EaseInOut","ms":900},"range_end":{"curve":"Spring"}}}}"#;
        let m = from_json(old).unwrap().voice.motion;
        assert_eq!((m.pointer, m.range_start), (VoiceGlide { curve: VoiceCurve::Smooth, ms: 200 }, VoiceGlide { curve: VoiceCurve::Smooth, ms: 900 }));
        assert_eq!(m.range_end.curve, VoiceCurve::Spring);
        // The one curve of before, too, and it is written back under its new name.
        let s = from_json(r#"{"voice":{"motion":{"curve":"EaseInOut","glide_ms":500}}}"#).unwrap();
        assert_eq!(s.voice.motion.range_end, VoiceGlide { curve: VoiceCurve::Smooth, ms: 500 });
        assert!(to_json(&s).contains(r#""curve":"Smooth""#) && !to_json(&s).contains("Ease"));
    }

    #[test]
    fn pointer_and_voice_values_are_clamped() {
        let s = from_json(
            r#"{"pointer":{"smudge":99,"smudge_distance":0},"voice":{"ring":200,"far_wait_ms":60000,
            "pointer":{"smudge":99,"smudge_distance":99},"range":{"border":99,"blur":99},
            "motion":{"pointer":{"ms":5},"range_start":{"ms":9999},"range_end":{"ms":5},"ayah_fade_delay_ms":60000}}}"#,
        )
        .unwrap();
        assert_eq!((s.pointer.smudge, s.pointer.smudge_distance), (10, 1));
        assert_eq!((s.voice.pointer.smudge, s.voice.pointer.smudge_distance), (10, 10));
        assert_eq!((s.voice.ring, s.voice.far_wait_ms, s.voice.range.border, s.voice.range.blur), (40, 20000, 12, 10));
        let m = &s.voice.motion;
        assert_eq!((m.pointer.ms, m.range_start.ms, m.range_end.ms, m.ayah_fade_delay_ms), (60, 1500, 60, 10000));
    }

    #[test]
    fn every_pointer_and_voice_setting_roundtrips() {
        let mut s = Settings::default();
        s.pointer.smudge = 4;
        s.pointer.smudge_distance = 9;
        let voice = &mut s.voice;
        voice.pointer.smudge = 10;
        voice.pointer.smudge_distance = 1;
        voice.ring = 33;
        voice.far_wait_ms = 0;
        voice.range.border = 12;
        voice.range.blur = 7;
        voice.motion.pointer = VoiceGlide { curve: VoiceCurve::Spring, ms: 900 };
        voice.motion.range_start = VoiceGlide { curve: VoiceCurve::Linear, ms: 1500 };
        voice.motion.range_end = VoiceGlide { curve: VoiceCurve::Smooth, ms: 60 };
        voice.motion.fade_at_ayah_end = false;
        voice.tuned = false;
        voice.motion.continuous_scroll = true;
        voice.motion.ayah_fade_delay_ms = 2500;
        assert_eq!(from_json(&to_json(&s)), Some(s));
        assert_eq!(from_json("{}"), Some(Settings::default()));
    }

    #[test]
    fn corrupt_or_empty_is_none() {
        for raw in ["", "   ", "not json", "{", "[1,2]", "null", "42", r#"{"translations": "x"}"#,
            r#"{"arabic_font_scale": 300}"#, "\u{0}\u{1}",
            r#"{"mushaf_font": "V9"}"#, r#"{"font_base_url": 7}"#, r#"{"show_page_translation": "yes"}"#]
        {
            assert_eq!(from_json(raw), None, "input: {raw:?}");
        }
    }

    #[test]
    fn partial_uses_defaults_and_ignores_unknown() {
        // Unknown fields include ones older versions stored (`locale`, `script`, `reciter`).
        let s = from_json(r#"{"locale":"ur","script":"IndoPak","reciter":7,"word_by_word":{"display":"Inline"}}"#)
            .unwrap();
        assert_eq!(s.translations, Settings::default().translations);
        assert_eq!(s.word_by_word.display, WbwDisplay::Inline);
        assert!(s.word_by_word.show_translation);
        assert_eq!(from_json("{}"), Some(Settings::default()));
    }

    #[test]
    fn values_are_normalized() {
        let s = from_json(
            r#"{"arabic_font_scale":0,"translation_font_scale":99,"translations":[1,2,1,3,2]}"#,
        )
        .unwrap();
        assert_eq!(s.arabic_font_scale, 1);
        assert_eq!(s.translation_font_scale, 10);
        assert_eq!(s.translations, vec![1, 2, 3]);
    }

    #[test]
    fn the_model_and_memorizing_carry_over_and_default_to_what_was_before() {
        // Settings of before: the listener's own model/, the words shown.
        let s = from_json(r#"{"theme":"Dark","voice":{"listener":"/opt/qv"}}"#).unwrap();
        assert_eq!((s.voice.model.as_str(), s.hifz), ("", false));
        assert_eq!(crate::settings::model_folder(&s.voice.model), "model");
        let mut s = Settings { hifz: true, ..Default::default() };
        s.voice.model = "g-b".into();
        assert_eq!(from_json(&to_json(&s)), Some(s.clone()));
        assert_eq!(crate::settings::model_folder(&s.voice.model), "models/g-b");
        // Only a folder right inside models/.
        for (raw, kept) in [(" h-a ", "h-a"), ("../model", ""), ("a/b", ""), ("..", ""), (".", ""), ("x\\y", ""), ("p\u{0}", "")] {
            let json = serde_json::json!({ "voice": { "model": raw } }).to_string();
            assert_eq!(from_json(&json).unwrap().voice.model, kept, "{raw:?}");
        }
        // Memorizing hides the verse translations under the pages where they are: they keep their place.
        let s = Settings { show_page_translation: true, hifz: true, ..Default::default() };
        assert!(s.page_translations() && Settings { hifz: false, ..s }.page_translations());
    }

    #[test]
    fn skipped_words_are_marked_by_default_and_their_look_carries_over_clamped() {
        // Settings of before: marked, in a soft red under the pointer, gaps of two words or more.
        let s = from_json(r#"{"theme":"Dark","voice":{"range":{"opacity":32,"blur":3}}}"#).unwrap();
        assert_eq!(s.voice.skipped, crate::settings::VoiceSkipped::default());
        let k = &s.voice.skipped;
        assert!(k.enabled && k.color == "#d9534f" && (k.opacity, k.min_gap) == (30, 2));
        let mut s = Settings::default();
        let k = &mut s.voice.skipped;
        (k.enabled, k.color, k.size, k.opacity, k.roundness, k.border, k.blur, k.min_gap) = (false, "#123abc".into(), 20, 0, 50, 12, 10, 10);
        assert_eq!(from_json(&to_json(&s)), Some(s));
        let s = from_json(r#"{"voice":{"skipped":{"color":"red","size":1,"opacity":200,"roundness":99,"border":99,"blur":99,"min_gap":0}}}"#).unwrap();
        let k = &s.voice.skipped;
        assert_eq!((k.color.as_str(), k.size, k.opacity, k.roundness, k.border, k.blur, k.min_gap), ("#d9534f", 4, 100, 50, 12, 10, 1));
        assert_eq!(from_json(r#"{"voice":{"skipped":{"min_gap":99}}}"#).unwrap().voice.skipped.min_gap, 10);
    }

    #[test]
    fn a_slide_into_a_similar_ayah_is_marked_by_default_in_its_own_colour_its_look_clamped() {
        // Settings of before: marked, as the skipped words are, in violet.
        let s = from_json(r##"{"voice":{"skipped":{"color":"#123abc","opacity":50}}}"##).unwrap();
        assert_eq!(s.voice.similar, crate::settings::VoiceSimilar::default());
        assert!(s.voice.similar.enabled && s.voice.similar.color == "#8a63d2" && s.voice.similar.opacity == 30);
        let mut s = Settings::default();
        let k = &mut s.voice.similar;
        (k.enabled, k.color, k.size, k.opacity, k.roundness, k.border, k.blur) = (false, "#00aa55".into(), 12, 45, 50, 2, 4);
        assert_eq!(from_json(&to_json(&s)), Some(s));
        let s = from_json(r#"{"voice":{"similar":{"color":"blue","size":30,"opacity":200,"roundness":99,"border":99,"blur":99}}}"#).unwrap();
        let k = &s.voice.similar;
        assert_eq!((k.color.as_str(), k.size, k.opacity, k.roundness, k.border, k.blur), ("#8a63d2", 20, 100, 50, 12, 10));
    }

    #[test]
    fn the_offline_offer_is_made_once() {
        // Settings of before have not seen it; once answered, it stays answered.
        assert!(!from_json(r#"{"theme":"Dark","voice":{"tuned":false}}"#).unwrap().offline_offered);
        let s = Settings { offline_offered: true, ..Default::default() };
        assert_eq!(from_json(&to_json(&s)), Some(s));
    }

    #[test]
    fn empty_translation_list_is_kept() {
        let s = from_json(r#"{"translations":[]}"#).unwrap();
        assert!(s.translations.is_empty());
    }

    #[test]
    fn file_roundtrip_atomic_and_corrupt() {
        let dir = std::env::temp_dir().join(format!("quran-eg-test-{}", std::process::id()));
        let path = dir.join("nested").join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(load_from_path(&path), None, "missing file");

        let mut s = Settings { translation_font_scale: 7, ..Default::default() };
        save_to_path(&path, &s).unwrap();
        assert_eq!(load_from_path(&path), Some(s.clone()));
        let names = || std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>();
        assert_eq!(names(), ["settings.json"], "temp file renamed away");

        // Overwrite existing file.
        s.mushaf_font = MushafFont::MadaniV2;
        save_to_path(&path, &s).unwrap();
        assert_eq!(load_from_path(&path).unwrap().mushaf_font, MushafFont::MadaniV2);

        std::fs::write(&path, b"{\"locale\": ").unwrap();
        assert_eq!(load_from_path(&path), None, "corrupt file");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(load_from_path(&path), None, "non-utf8 file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepts_word_check_tool_settings() {
        // Exactly what tools/word-check/check-words.mjs writes to localStorage.
        let json = r#"{"mushaf_font":"MadaniV2","word_by_word":{"show_translation":true,"show_transliteration":true,"display":"Tooltip"}}"#;
        let s = from_json(json).expect("tool settings must parse");
        assert_eq!(s.mushaf_font, MushafFont::MadaniV2);
        assert_eq!(s.word_by_word.display, WbwDisplay::Tooltip);
    }

    #[test]
    fn non_numeric_translation_ids_are_dropped() {
        let s = from_json(r#"{"translations":[131,"20",null,85,-1,1.5,{"id":3},4294967296,20,131]}"#).unwrap();
        assert_eq!(s.translations, vec![131, 85, 20]);
        // Not an array at all is still a type error.
        assert_eq!(from_json(r#"{"translations":{"a":1}}"#), None);
    }

    #[test]
    fn font_base_url_sanitized_on_load() {
        let url = |raw: &str| {
            let json = serde_json::json!({ "font_base_url": raw }).to_string();
            from_json(&json).unwrap().font_base_url
        };
        assert_eq!(url("  https://cdn.example.org/fonts/quran/  "), "https://cdn.example.org/fonts/quran");
        assert_eq!(url("http://localhost:8080/quran"), "http://localhost:8080/quran");
        assert_eq!(url("HTTPS://Example.org"), "HTTPS://Example.org");
        assert_eq!(url("/fonts/quran"), "/fonts/quran");
        for bad in ["", "   ", "ftp://x.org", "javascript:alert(1)", "fonts/quran", "//evil.org/x", "https://",
            "https:///path", "https://x.org/a b", "https://x.org/\")", "/a'b", "data:font/woff2;base64,AA", "/", "///"]
        {
            assert_eq!(url(bad), "", "input: {bad:?}");
        }
    }

    #[test]
    fn sanitize_font_base_url_direct() {
        assert_eq!(sanitize_font_base_url(" /quran// ").as_deref(), Some("/quran"));
        assert_eq!(sanitize_font_base_url("https://a.b"), Some("https://a.b".into()));
        assert_eq!(sanitize_font_base_url("https://a.b/<x>"), None);
        assert_eq!(sanitize_font_base_url(&format!("https://a.b/{}", "x".repeat(3000))), None);
    }
}
