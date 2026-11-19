//! Query parameters shared by the `/verses/by_*` endpoints.

/// Parameters for `/verses/by_chapter`, `/by_page`, `/by_juz`, `/by_key`.
#[derive(Debug, Clone, PartialEq)]
pub struct VerseQuery {
    /// Language for word-by-word translation/transliteration (`language=`).
    pub language: String,
    /// Translation resource ids (`translations=131,20`).
    pub translations: Vec<u32>,
    /// Include per-word data (`words=true`).
    pub words: bool,
    /// Extra verse fields, e.g. `text_uthmani`, `text_indopak`.
    pub fields: Vec<String>,
    /// Extra word fields, e.g. `text_uthmani`, `text_indopak`.
    pub word_fields: Vec<String>,
    /// Extra translation fields, e.g. `resource_name`, `language_name`.
    pub translation_fields: Vec<String>,
    /// 1-based page of results.
    pub page: u32,
    /// Results per page (API max 50).
    pub per_page: u32,
    /// Mushaf layout id for `page_number`/`line_number` (`mushaf=`), e.g. 19 = QCF V4
    /// tajweed, 1 = QCF V2. `None` = API default.
    pub mushaf: Option<u32>,
}

/// Mushaf ids used by the Quran.com API.
pub mod mushaf {
    pub const QCF_V2: u32 = 1;
    pub const QCF_V1: u32 = 2;
    pub const QCF_TAJWEED_V4: u32 = 19;
}

impl Default for VerseQuery {
    fn default() -> Self {
        Self {
            language: "en".into(),
            translations: vec![131],
            words: true,
            fields: vec!["text_uthmani".into()],
            word_fields: vec!["text_uthmani".into()],
            translation_fields: vec!["resource_name".into(), "language_name".into()],
            page: 1,
            per_page: 50,
            mushaf: None,
        }
    }
}

impl VerseQuery {
    /// Encode as `(key, value)` query pairs. Empty lists are omitted.
    pub fn to_pairs(&self) -> Vec<(&'static str, String)> {
        let join = |v: &[String]| v.join(",");
        let mut out = vec![
            ("language", self.language.clone()),
            ("words", self.words.to_string()),
            ("page", self.page.to_string()),
            ("per_page", self.per_page.min(50).to_string()),
        ];
        if !self.translations.is_empty() {
            let ids: Vec<String> = self.translations.iter().map(u32::to_string).collect();
            out.push(("translations", ids.join(",")));
        }
        if !self.fields.is_empty() {
            out.push(("fields", join(&self.fields)));
        }
        if !self.word_fields.is_empty() {
            out.push(("word_fields", join(&self.word_fields)));
        }
        if !self.translation_fields.is_empty() {
            out.push(("translation_fields", join(&self.translation_fields)));
        }
        if let Some(m) = self.mushaf {
            out.push(("mushaf", m.to_string()));
        }
        out
    }

    /// Same query for a different results page.
    pub fn with_page(&self, page: u32) -> Self {
        Self { page, ..self.clone() }
    }
}
