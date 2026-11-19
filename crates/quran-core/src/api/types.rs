//! Response types for the Quran.com API v4 (snake_case JSON).
//!
//! CONTRACT: other crates depend on these names. Adding fields is fine;
//! renaming/removing public fields or types is not.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TranslatedName {
    #[serde(default)]
    pub language_name: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub id: u16,
    pub revelation_place: String,
    pub revelation_order: u16,
    pub bismillah_pre: bool,
    pub name_simple: String,
    pub name_complex: String,
    pub name_arabic: String,
    pub verses_count: u16,
    /// `[first_page, last_page]` in the Madani mushaf.
    #[serde(default)]
    pub pages: Vec<u16>,
    pub translated_name: TranslatedName,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChaptersResponse {
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterResponse {
    pub chapter: Chapter,
}

/// `text`/`language_name` pair used for word translation & transliteration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WordText {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub language_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Word {
    pub id: u32,
    pub position: u16,
    /// `"word"`, `"end"` (ayah number marker), `"pause"`, `"sajdah"`, `"rub-el-hizb"`.
    pub char_type_name: String,
    #[serde(default)]
    pub text_uthmani: Option<String>,
    #[serde(default)]
    pub text_indopak: Option<String>,
    #[serde(default)]
    pub text_imlaei: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub audio_url: Option<String>,
    #[serde(default)]
    pub page_number: Option<u16>,
    #[serde(default)]
    pub line_number: Option<u16>,
    #[serde(default)]
    pub translation: Option<WordText>,
    #[serde(default)]
    pub transliteration: Option<WordText>,
    /// `"2:255:3"` (chapter:verse:position).
    #[serde(default)]
    pub location: Option<String>,
    /// KFGQPC Hafs Unicode text (the fallback while/if a QCF page font is unavailable).
    #[serde(default)]
    pub qpc_uthmani_hafs: Option<String>,
    /// QCF V1 glyph code(s) for the page font `v1_page`.
    #[serde(default)]
    pub code_v1: Option<String>,
    /// QCF V2 glyph code(s); also used by the V4 tajweed fonts. Font file: `v2_page`.
    #[serde(default)]
    pub code_v2: Option<String>,
    #[serde(default)]
    pub v1_page: Option<u16>,
    #[serde(default)]
    pub v2_page: Option<u16>,
}

impl Word {
    pub fn is_end_marker(&self) -> bool {
        self.char_type_name == "end"
    }
}

/// A verse translation. `text` may contain HTML such as `<sup foot_note=123>1</sup>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    #[serde(default)]
    pub id: Option<u32>,
    pub resource_id: u32,
    pub text: String,
    #[serde(default)]
    pub resource_name: Option<String>,
    #[serde(default)]
    pub language_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verse {
    pub id: u32,
    pub verse_number: u16,
    /// `"2:255"`
    pub verse_key: String,
    #[serde(default)]
    pub page_number: Option<u16>,
    #[serde(default)]
    pub juz_number: Option<u8>,
    #[serde(default)]
    pub hizb_number: Option<u8>,
    #[serde(default)]
    pub rub_el_hizb_number: Option<u16>,
    #[serde(default)]
    pub sajdah_number: Option<u16>,
    #[serde(default)]
    pub text_uthmani: Option<String>,
    #[serde(default)]
    pub text_indopak: Option<String>,
    #[serde(default)]
    pub text_imlaei: Option<String>,
    /// KFGQPC Hafs Unicode text of the whole verse, ending in NBSP + the ayah number
    /// (the API has this per verse only, not per word).
    #[serde(default)]
    pub qpc_uthmani_hafs: Option<String>,
    #[serde(default)]
    pub words: Vec<Word>,
    #[serde(default)]
    pub translations: Vec<Translation>,
}

impl Verse {
    /// Chapter number parsed from `verse_key`.
    pub fn chapter_number(&self) -> Option<u16> {
        self.verse_key.split(':').next()?.parse().ok()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Pagination {
    pub per_page: u32,
    pub current_page: u32,
    #[serde(default)]
    pub next_page: Option<u32>,
    pub total_pages: u32,
    pub total_records: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersesResponse {
    pub verses: Vec<Verse>,
    /// Required on purpose: a response without pagination must fail loudly rather
    /// than look like "everything loaded" and silently cut a juz/page short.
    pub pagination: Pagination,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerseResponse {
    pub verse: Verse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Juz {
    pub id: u32,
    pub juz_number: u8,
    /// chapter number -> `"from-to"` verse range, e.g. `{"1": "1-7", "2": "1-141"}`
    pub verse_mapping: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub first_verse_id: Option<u32>,
    #[serde(default)]
    pub last_verse_id: Option<u32>,
    #[serde(default)]
    pub verses_count: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JuzsResponse {
    pub juzs: Vec<Juz>,
}

/// An available translation resource (`/resources/translations`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationResource {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    pub language_name: String,
    #[serde(default)]
    pub translated_name: Option<TranslatedName>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationsResponse {
    pub translations: Vec<TranslationResource>,
}

/// `/resources/languages`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Language {
    pub id: u32,
    pub name: String,
    pub iso_code: String,
    #[serde(default)]
    pub native_name: Option<String>,
    /// `"ltr"` or `"rtl"`
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub translations_count: Option<u32>,
    #[serde(default)]
    pub translated_name: Option<TranslatedName>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguagesResponse {
    pub languages: Vec<Language>,
}

/// `/foot_notes/{id}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FootNote {
    pub id: u32,
    pub text: String,
    #[serde(default)]
    pub language_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FootNoteResponse {
    pub foot_note: FootNote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchTranslation {
    pub text: String,
    #[serde(default)]
    pub resource_id: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub language_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub verse_key: String,
    #[serde(default)]
    pub verse_id: Option<u32>,
    /// Arabic text of the verse.
    pub text: String,
    /// Arabic text with `<em>` around matches, when the match was in Arabic.
    #[serde(default)]
    pub highlighted: Option<String>,
    #[serde(default)]
    pub translations: Vec<SearchTranslation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Search {
    pub query: String,
    pub total_results: u32,
    pub current_page: u32,
    pub total_pages: u32,
    #[serde(default)]
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub search: Search,
}
