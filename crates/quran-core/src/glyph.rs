//! What to draw for one mushaf word: its QCF glyph (when the page font is
//! loaded) or Unicode text, never nothing. Pure, so the fallback chain is tested.

use crate::api::Word;

use crate::mushaf_layout::WordCell;

/// Placeholder for a word without any glyph or text (never render nothing).
pub const MISSING_WORD: &str = "⟨?⟩";

/// How a word is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordMode {
    /// QCF glyph code(s) in the font of the word's page.
    Glyph { glyph: String },
    /// Unicode text; `qpc`: in the QPC encoding the Hafs fallback font draws.
    Text { text: String, qpc: bool },
    /// Nothing to draw: show [`MISSING_WORD`].
    Missing,
}

impl WordMode {
    pub fn is_glyph(&self) -> bool {
        matches!(self, WordMode::Glyph { .. })
    }

    /// The text the word shows.
    pub fn text(&self) -> &str {
        match self {
            WordMode::Glyph { glyph } => glyph,
            WordMode::Text { text, .. } => text,
            WordMode::Missing => MISSING_WORD,
        }
    }
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|t| !t.is_empty())
}

/// Decode numeric character references (`&#xFB51;`, `&#64337;`) that some API
/// glyph codes use. Anything else stays as it is: plain characters.
pub fn decode_glyph(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(amp) = rest.find("&#") {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 2..];
        let Some(semi) = after.find(';').filter(|i| *i <= 8) else {
            out.push_str(&rest[amp..amp + 2]);
            rest = after;
            continue;
        };
        let num = &after[..semi];
        let parsed = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => num.parse::<u32>().ok(),
        };
        match parsed.and_then(char::from_u32) {
            Some(c) => out.push(c),
            None => out.push_str(&rest[amp..amp + 2 + semi + 1]),
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// The word's own Unicode text and whether it is QPC-encoded: `qpc_uthmani_hafs`
/// → `text_uthmani`. (`text` is not used: with a mushaf selected it holds the
/// glyph code, which is not readable text.)
pub fn unicode_text(word: &Word) -> Option<(String, bool)> {
    let qpc = non_empty(word.qpc_uthmani_hafs.as_deref()).map(|t| (t.to_string(), true));
    qpc.or_else(|| non_empty(word.text_uthmani.as_deref()).map(|t| (t.to_string(), false)))
}

/// The word's glyph code, if it has one that belongs to its layout page
/// (`v2_page` absent or equal to `page_number`).
pub fn glyph_code(word: &Word) -> Option<String> {
    if !crate::integrity::glyph_page_ok(word) {
        return None;
    }
    non_empty(word.code_v2.as_deref()).map(decode_glyph).filter(|g| !g.is_empty())
}

/// Choose how to draw a cell: its glyph if `font_ready` (the QCF font of its
/// page is loaded and has the glyph), else its Unicode text.
pub fn choose_mode(cell: &WordCell, font_ready: bool) -> WordMode {
    match (non_empty(cell.glyph.as_deref()), non_empty(cell.text.as_deref())) {
        (Some(g), _) if font_ready => WordMode::Glyph { glyph: g.to_string() },
        (_, Some(t)) => WordMode::Text { text: t.to_string(), qpc: cell.qpc },
        _ => WordMode::Missing,
    }
}

/// `255` → `٢٥٥`
pub fn arabic_number(n: u16) -> String {
    n.to_string().chars().map(|c| c.to_digit(10).and_then(|d| char::from_u32(0x0660 + d)).unwrap_or(c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word() -> Word {
        Word { id: 1, position: 1, char_type_name: "word".into(), ..Default::default() }
    }

    fn cell(glyph: Option<&str>, text: Option<&str>) -> WordCell {
        WordCell { glyph: glyph.map(Into::into), text: text.map(Into::into), ..WordCell::default() }
    }

    #[test]
    fn glyph_only_when_font_loaded_and_code_present() {
        let text = |t: &str| WordMode::Text { text: t.into(), qpc: false };
        assert_eq!(
            choose_mode(&cell(Some("\u{fb51}"), Some("بِسْمِ")), true),
            WordMode::Glyph { glyph: "\u{fb51}".into() }
        );
        // Font not loaded (yet / failed) → Unicode text.
        assert_eq!(choose_mode(&cell(Some("\u{fb51}"), Some("بِسْمِ")), false), text("بِسْمِ"));
        // No glyph code → text even if the font is loaded.
        assert_eq!(choose_mode(&cell(None, Some("t")), true), text("t"));
        assert_eq!(choose_mode(&cell(Some("  "), Some("t")), true), text("t"));
        // Nothing at all → placeholder.
        assert_eq!(choose_mode(&cell(None, Some(" ")), true), WordMode::Missing);
        assert_eq!(choose_mode(&cell(Some("g"), None), false), WordMode::Missing);
        assert_eq!((WordMode::Missing.is_glyph(), WordMode::Missing.text()), (false, MISSING_WORD));
        assert!(choose_mode(&cell(Some("g"), None), true).is_glyph());
        // QPC-encoded text is drawn with the Hafs fallback font.
        let qpc = WordCell { qpc: true, ..cell(None, Some("بِسۡمِ")) };
        assert_eq!(choose_mode(&qpc, true), WordMode::Text { text: "بِسۡمِ".into(), qpc: true });
    }

    #[test]
    fn unicode_fallback_chain() {
        let mut w = word();
        assert_eq!(unicode_text(&w), None);
        // `text` holds the glyph code when a mushaf is selected: never shown as text.
        w.text = Some("\u{fb51}".into());
        assert_eq!(unicode_text(&w), None);
        w.text_uthmani = Some("u".into());
        assert_eq!(unicode_text(&w), Some(("u".into(), false)));
        w.qpc_uthmani_hafs = Some(" ".into());
        assert_eq!(unicode_text(&w), Some(("u".into(), false)));
        w.qpc_uthmani_hafs = Some("q".into());
        assert_eq!(unicode_text(&w), Some(("q".into(), true)));
    }

    #[test]
    fn glyph_code_requires_matching_page() {
        let mut w = word();
        assert_eq!(glyph_code(&w), None);
        w.code_v2 = Some("\u{fb51}".into());
        assert_eq!(glyph_code(&w).as_deref(), Some("\u{fb51}"));
        w.page_number = Some(5);
        w.v2_page = Some(5);
        assert!(glyph_code(&w).is_some());
        w.v2_page = Some(6);
        assert_eq!(glyph_code(&w), None);
    }

    #[test]
    fn decodes_entities_as_text() {
        assert_eq!(decode_glyph("&#xFB51;"), "\u{fb51}");
        assert_eq!(decode_glyph("&#64337; &#xfb52;"), "\u{fb51} \u{fb52}");
        assert_eq!(decode_glyph("\u{fb51}"), "\u{fb51}");
        assert_eq!(decode_glyph("&#xZZ;"), "&#xZZ;");
        assert_eq!(decode_glyph("a&#b"), "a&#b");
        assert_eq!(decode_glyph("<b>x</b>"), "<b>x</b>"); // stays inert text
    }

    #[test]
    fn arabic_numbers() {
        assert_eq!(arabic_number(255), "٢٥٥");
    }
}
