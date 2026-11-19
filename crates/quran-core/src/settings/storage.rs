//! Settings persistence: a JSON file under the user's config directory, next to
//! the location the app was last at (view and reading position).
//!
//! All parsing/serialization lives in plain functions so it can be unit-tested.
//! Missing or corrupt data yields `None`; nothing here panics.

use super::Settings;

/// Serialize settings to the JSON stored on disk / in localStorage.
pub fn to_json(settings: &Settings) -> String {
    // Serializing plain data structs can't fail; fall back to an empty object just in case.
    serde_json::to_string(settings).unwrap_or_else(|_| "{}".into())
}

/// Parse stored JSON. Unknown fields are ignored and missing ones take defaults;
/// malformed JSON or wrongly-typed values yield `None`, except that non-numeric
/// entries of the `translations` array are dropped. Values are normalized.
pub fn from_json(raw: &str) -> Option<Settings> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut value: serde_json::Value = serde_json::from_str(raw).ok()?;
    if let Some(list) = value.get_mut("translations").and_then(|t| t.as_array_mut()) {
        // Keep only resource ids (numbers that fit a u32); their language can't be
        // verified offline, so the settings page lets the user remove stray ones.
        list.retain(|v| v.as_u64().is_some_and(|n| u32::try_from(n).is_ok()));
    }
    let mut settings: Settings = serde_json::from_value(value).ok()?;
    normalize(&mut settings);
    Some(settings)
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
    pointer.trail = pointer.trail.min(10);
    pointer.trail_distance = pointer.trail_distance.clamp(1, 10);
    pointer.blur = pointer.blur.min(10);
    pointer.glow = pointer.glow.min(10);
    pointer.shrink = pointer.shrink.min(10);
    pointer.color = super::sanitize_color(&pointer.color).unwrap_or_else(|| super::DEFAULT_POINTER_COLOR.into());
    let mut seen = std::collections::HashSet::new();
    s.translations.retain(|id| seen.insert(*id));
    s.font_base_url = sanitize_font_base_url(&s.font_base_url).unwrap_or_default();
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
    /// sibling temp file, then rename it over the target.
    pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".tmp");
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

pub use backend::write_atomic;

#[cfg(test)]
mod tests {
    use super::backend::{load_from_path, save_to_path};
    use super::*;
    use crate::settings::{MushafFont, WbwDisplay};

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
        assert!(!path.with_file_name("settings.json.tmp").exists(), "temp file renamed away");

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
