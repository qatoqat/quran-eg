//! Deserialize recorded-shape fixtures of API v4 responses into the public types.
//!
//! Fixtures live in `tests/fixtures/` and mirror real `api.quran.com/api/v4` payloads
//! (trimmed to a few records). They intentionally include fields our types don't model
//! (`ruku_number`, `manzil_number`, `sajdah_type`, search `words`, ...) to prove that
//! unknown fields are ignored.

use pretty_assertions::assert_eq;
use quran_core::api::*;
use serde::de::DeserializeOwned;

fn fixture<T: DeserializeOwned>(name: &str) -> T {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

/// Serialize then deserialize again; our own output must round-trip losslessly.
fn roundtrip<T>(v: &T)
where
    T: serde::Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(v).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, v);
}

#[test]
fn chapters() {
    let r: ChaptersResponse = fixture("chapters.json");
    assert_eq!(r.chapters.len(), 4);

    let fatihah = &r.chapters[0];
    assert_eq!(fatihah.id, 1);
    assert_eq!(fatihah.name_simple, "Al-Fatihah");
    assert_eq!(fatihah.name_complex, "Al-Fātiĥah");
    assert_eq!(fatihah.name_arabic, "الفاتحة");
    assert_eq!(fatihah.revelation_place, "makkah");
    assert_eq!(fatihah.revelation_order, 5);
    assert_eq!(fatihah.verses_count, 7);
    assert_eq!(fatihah.pages, vec![1, 1]);
    // Al-Fatihah's bismillah *is* verse 1, so no separate bismillah is shown.
    assert!(!fatihah.bismillah_pre);
    assert_eq!(
        fatihah.translated_name,
        TranslatedName {
            language_name: Some("english".into()),
            name: "The Opener".into()
        }
    );

    assert!(
        r.chapters[1].bismillah_pre,
        "Al-Baqarah starts with a bismillah"
    );

    let tawbah = r
        .chapters
        .iter()
        .find(|c| c.id == 9)
        .expect("chapter 9 present");
    assert!(!tawbah.bismillah_pre, "At-Tawbah has no bismillah");
    assert_eq!(tawbah.revelation_place, "madinah");
    assert_eq!(tawbah.verses_count, 129);
    assert_eq!(tawbah.pages, vec![187, 207]);
    assert_eq!(tawbah.translated_name.name, "The Repentance");

    assert_eq!(r.chapters.last().unwrap().id, 114);
    roundtrip(&r);
}

#[test]
fn chapter() {
    let r: ChapterResponse = fixture("chapter.json");
    let c = r.chapter;
    assert_eq!(c.id, 2);
    assert_eq!(c.name_simple, "Al-Baqarah");
    assert_eq!(c.name_arabic, "البقرة");
    assert_eq!(c.verses_count, 286);
    assert_eq!(c.pages, vec![2, 49]);
    assert_eq!(c.translated_name.name, "The Cow");
}

#[test]
fn verses_by_chapter() {
    let r: VersesResponse = fixture("verses_by_chapter.json");
    assert_eq!(
        r.pagination,
        Pagination {
            per_page: 3,
            current_page: 1,
            next_page: Some(2),
            total_pages: 3,
            total_records: 7
        }
    );
    assert_eq!(r.verses.len(), 3);
    let keys: Vec<&str> = r.verses.iter().map(|v| v.verse_key.as_str()).collect();
    assert_eq!(keys, ["1:1", "1:2", "1:3"]);

    let v1 = &r.verses[0];
    assert_eq!(v1.id, 1);
    assert_eq!(v1.verse_number, 1);
    assert_eq!(v1.chapter_number(), Some(1));
    assert_eq!(v1.page_number, Some(1));
    assert_eq!(v1.juz_number, Some(1));
    assert_eq!(v1.hizb_number, Some(1));
    assert_eq!(v1.rub_el_hizb_number, Some(1));
    assert_eq!(v1.sajdah_number, None, "null deserializes to None");
    assert_eq!(v1.text_uthmani.as_deref(), Some("بِسْمِ ٱللَّهِ ٱلرَّحْمَٰنِ ٱلرَّحِيمِ"));
    assert_eq!(v1.text_indopak, None, "field not requested");

    // Words: 4 real words followed by the ayah-number end marker.
    assert_eq!(v1.words.len(), 5);
    let w = &v1.words[0];
    assert_eq!(w.id, 1);
    assert_eq!(w.position, 1);
    assert_eq!(w.char_type_name, "word");
    assert!(!w.is_end_marker());
    assert_eq!(w.text_uthmani.as_deref(), Some("بِسْمِ"));
    assert_eq!(w.text.as_deref(), Some("بِسْمِ"));
    assert_eq!(w.audio_url.as_deref(), Some("wbw/001_001_001.mp3"));
    assert_eq!(w.page_number, Some(1));
    assert_eq!(w.line_number, Some(2));
    assert_eq!(
        w.translation,
        Some(WordText {
            text: Some("In (the) name".into()),
            language_name: Some("english".into())
        })
    );
    assert_eq!(
        w.transliteration.as_ref().unwrap().text.as_deref(),
        Some("bis'mi")
    );

    let end = v1.words.last().unwrap();
    assert!(end.is_end_marker());
    assert_eq!(end.char_type_name, "end");
    assert_eq!(end.text.as_deref(), Some("١"));
    assert_eq!(end.audio_url, None, "end marker has null audio");
    assert_eq!(
        end.translation.as_ref().unwrap().text.as_deref(),
        Some("(1)")
    );
    assert_eq!(
        end.transliteration.as_ref().unwrap().text,
        None,
        "null transliteration text"
    );

    // Every verse ends with exactly one end marker.
    for v in &r.verses {
        assert_eq!(
            v.words.iter().filter(|w| w.is_end_marker()).count(),
            1,
            "{}",
            v.verse_key
        );
        assert!(v.words.last().unwrap().is_end_marker(), "{}", v.verse_key);
    }

    // Translations, including HTML footnote markers left verbatim.
    assert_eq!(v1.translations.len(), 2);
    let t = &v1.translations[0];
    assert_eq!(t.resource_id, 131);
    assert_eq!(t.id, Some(840001));
    assert_eq!(
        t.text,
        "In the Name of Allah—the Most Compassionate, Most Merciful."
    );
    assert_eq!(
        t.resource_name.as_deref(),
        Some("Dr. Mustafa Khattab, The Clear Quran")
    );
    assert_eq!(t.language_name.as_deref(), Some("english"));
    assert_eq!(v1.translations[1].resource_id, 20);

    let kh_1_2 = &r.verses[1].translations[0];
    assert!(
        kh_1_2.text.ends_with("<sup foot_note=77933>1</sup>"),
        "footnote marker preserved: {}",
        kh_1_2.text
    );

    roundtrip(&r);
}

#[test]
fn verses_by_page_without_words_or_translations() {
    // Requested with words=false and no translations: the API omits both keys.
    let r: VersesResponse = fixture("verses_by_page.json");
    assert_eq!(r.verses.len(), 2);
    assert!(r.verses.iter().all(|v| v.page_number == Some(1)));
    assert!(
        r.verses
            .iter()
            .all(|v| v.words.is_empty() && v.translations.is_empty())
    );
    assert_eq!(
        r.verses[1].text_uthmani.as_deref(),
        Some("ٱلْحَمْدُ لِلَّهِ رَبِّ ٱلْعَٰلَمِينَ")
    );
    assert_eq!(r.pagination.next_page, Some(2));
    assert_eq!(r.pagination.total_pages, 4);
    assert_eq!(r.pagination.total_records, 7);
}

#[test]
fn verse_by_key() {
    let r: VerseResponse = fixture("verse_by_key.json");
    let v = r.verse;
    assert_eq!(v.id, 262);
    assert_eq!(v.verse_key, "2:255");
    assert_eq!(v.verse_number, 255);
    assert_eq!(v.chapter_number(), Some(2));
    assert_eq!(v.page_number, Some(42));
    assert_eq!(v.juz_number, Some(3));
    assert_eq!(v.hizb_number, Some(5));
    let text = v.text_uthmani.as_deref().unwrap();
    assert!(text.starts_with("ٱللَّهُ لَآ إِلَٰهَ إِلَّا هُوَ"), "{text}");
    assert!(text.contains("كُرْسِيُّهُ"));
    assert_eq!(v.words.len(), 2);
    assert_eq!(
        v.words[0].translation.as_ref().unwrap().text.as_deref(),
        Some("Allah")
    );
    assert_eq!(v.translations.len(), 2);
    assert!(
        v.translations[0]
            .text
            .contains("His Seat<sup foot_note=78013>1</sup>")
    );
    assert!(
        v.translations[1]
            .text
            .contains("His Kursi extends over the heavens and the earth")
    );
}

#[test]
fn juzs() {
    let r: JuzsResponse = fixture("juzs.json");
    assert_eq!(r.juzs.len(), 3);

    let j1 = &r.juzs[0];
    assert_eq!(j1.id, 1);
    assert_eq!(j1.juz_number, 1);
    assert_eq!(j1.verse_mapping.len(), 2);
    assert_eq!(j1.verse_mapping["1"], "1-7");
    assert_eq!(j1.verse_mapping["2"], "1-141");
    assert_eq!(j1.first_verse_id, Some(1));
    assert_eq!(j1.last_verse_id, Some(148));
    assert_eq!(j1.verses_count, Some(148));

    let j30 = &r.juzs[2];
    assert_eq!(j30.juz_number, 30);
    assert_eq!(j30.verse_mapping.len(), 37, "surahs 78..=114");
    assert_eq!(j30.verse_mapping["114"], "1-6");
    assert_eq!(j30.last_verse_id, Some(6236));
    assert_eq!(j30.verses_count, Some(564));
    // BTreeMap keys are strings, so they sort lexicographically ("100" < "78");
    // UI code must parse keys to order chapters numerically.
    let mut nums: Vec<u16> = j30
        .verse_mapping
        .keys()
        .map(|k| k.parse().unwrap())
        .collect();
    nums.sort_unstable();
    assert_eq!(nums.first(), Some(&78));
    assert_eq!(nums.last(), Some(&114));
}

#[test]
fn translations() {
    let r: TranslationsResponse = fixture("translations.json");
    assert_eq!(r.translations.len(), 3);
    let kh = &r.translations[0];
    assert_eq!(kh.id, 131);
    assert_eq!(kh.name, "Dr. Mustafa Khattab, The Clear Quran");
    assert_eq!(kh.author_name.as_deref(), Some("Dr. Mustafa Khattab"));
    assert_eq!(kh.slug.as_deref(), Some("clearquran-with-tafsir"));
    assert_eq!(kh.language_name, "english");
    assert_eq!(
        kh.translated_name.as_ref().map(|t| t.name.as_str()),
        Some("Dr. Mustafa Khattab, The Clear Quran")
    );
    let urdu = &r.translations[2];
    assert_eq!(urdu.language_name, "urdu");
    assert_eq!(urdu.slug, None, "null slug");
}

#[test]
fn languages() {
    let r: LanguagesResponse = fixture("languages.json");
    assert_eq!(r.languages.len(), 3);
    let en = &r.languages[0];
    assert_eq!(
        (en.id, en.name.as_str(), en.iso_code.as_str()),
        (38, "English", "en")
    );
    assert_eq!(en.direction.as_deref(), Some("ltr"));
    assert_eq!(en.translated_name.as_ref().unwrap().name, "English");
    let ar = &r.languages[1];
    assert_eq!(ar.iso_code, "ar");
    assert_eq!(ar.native_name.as_deref(), Some("العربية"));
    assert_eq!(ar.direction.as_deref(), Some("rtl"));
    assert_eq!(r.languages[2].iso_code, "ur");
}

#[test]
fn foot_note() {
    let r: FootNoteResponse = fixture("foot_note.json");
    assert_eq!(r.foot_note.id, 77933);
    assert_eq!(
        r.foot_note.text,
        "i.e., Lord of everything in existence including angels, humans, and animals."
    );
    assert_eq!(r.foot_note.language_name.as_deref(), Some("english"));
}

#[test]
fn search() {
    let r: SearchResponse = fixture("search.json");
    let s = r.search;
    assert_eq!(s.query, "mercy");
    assert_eq!((s.total_results, s.current_page, s.total_pages), (3, 1, 1));
    assert_eq!(s.results.len(), 3);

    let first = &s.results[0];
    assert_eq!(first.verse_key, "21:107");
    assert_eq!(first.verse_id, Some(2590));
    assert_eq!(first.text, "وَمَآ أَرْسَلْنَٰكَ إِلَّا رَحْمَةً لِّلْعَٰلَمِينَ");
    assert_eq!(first.highlighted, None);
    let t = &first.translations[0];
    assert_eq!(
        t.text,
        "And We have not sent you, [O Muhammad], except as a <em>mercy</em> to the worlds."
    );
    assert_eq!(t.resource_id, Some(20));
    assert_eq!(t.name.as_deref(), Some("Saheeh International"));
    assert_eq!(t.language_name.as_deref(), Some("english"));

    assert!(s.results.iter().all(|r| {
        r.translations
            .iter()
            .any(|t| t.text.contains("<em>mercy</em>"))
    }));
    roundtrip(&s);
}

// ---------------------------------------------------------------------------
// Tolerance: unknown fields ignored, optional fields may be missing or null.
// ---------------------------------------------------------------------------

#[test]
fn unknown_fields_are_ignored() {
    let json = r#"{
        "chapter": {
            "id": 1, "revelation_place": "makkah", "revelation_order": 5,
            "bismillah_pre": false, "name_simple": "Al-Fatihah",
            "name_complex": "Al-Fātiĥah", "name_arabic": "الفاتحة", "verses_count": 7,
            "pages": [1, 1],
            "translated_name": {"language_name": "english", "name": "The Opener", "language_id": 38},
            "slug": "al-fatihah", "default_slug": {"slug": "al-fatihah", "locale": "en"},
            "some_future_field": {"nested": [1, 2, 3]}
        },
        "meta": {"took_ms": 4}
    }"#;
    let r: ChapterResponse = serde_json::from_str(json).unwrap();
    assert_eq!(r.chapter.name_simple, "Al-Fatihah");

    let word = r#"{"id": 1, "position": 1, "char_type_name": "word", "code_v1": "ﭑ",
                   "code_v2": "ﱁ", "location": "1:1:1", "qpc_uthmani_hafs": "بِسۡمِ",
                   "translation": {"text": "In (the) name", "language_name": "english", "language_id": 38}}"#;
    let w: Word = serde_json::from_str(word).unwrap();
    assert_eq!(
        w.translation.unwrap().text.as_deref(),
        Some("In (the) name")
    );
}

#[test]
fn missing_optional_fields_are_fine() {
    // Bare minimum verse: no words, translations, page/juz numbers or texts.
    let v: Verse =
        serde_json::from_str(r#"{"id": 7, "verse_number": 7, "verse_key": "1:7"}"#).unwrap();
    assert_eq!(v.chapter_number(), Some(1));
    assert!(v.words.is_empty() && v.translations.is_empty());
    assert_eq!(
        (v.page_number, v.juz_number, v.text_uthmani),
        (None, None, None)
    );

    // Explicit nulls behave like absent keys.
    let v: Verse = serde_json::from_str(
        r#"{"id": 1, "verse_number": 1, "verse_key": "1:1", "page_number": null,
            "text_uthmani": null, "sajdah_number": null}"#,
    )
    .unwrap();
    assert_eq!(v.page_number, None);

    // Word with only its required fields.
    let w: Word =
        serde_json::from_str(r#"{"id": 5, "position": 5, "char_type_name": "end"}"#).unwrap();
    assert!(w.is_end_marker());
    assert_eq!(
        (w.text, w.translation, w.transliteration),
        (None, None, None)
    );

    // Translation without id / resource_name / language_name.
    let t: Translation =
        serde_json::from_str(r#"{"resource_id": 131, "text": "Praise be to Allah"}"#).unwrap();
    assert_eq!((t.id, t.resource_name, t.language_name), (None, None, None));

    // Chapter without pages; translated_name without language_name.
    let c: Chapter = serde_json::from_str(
        r#"{"id": 9, "revelation_place": "madinah", "revelation_order": 113,
            "bismillah_pre": false, "name_simple": "At-Tawbah", "name_complex": "At-Tawbah",
            "name_arabic": "التوبة", "verses_count": 129, "translated_name": {"name": "The Repentance"}}"#,
    )
    .unwrap();
    assert!(c.pages.is_empty());
    assert_eq!(c.translated_name.language_name, None);

    // A verses response without pagination is an error on purpose: treating it as
    // "no more pages" could silently cut a juz or page short.
    assert!(serde_json::from_str::<VersesResponse>(r#"{"verses": []}"#).is_err());

    // Pagination on the last page has next_page: null.
    let p: Pagination = serde_json::from_str(
        r#"{"per_page": 50, "current_page": 1, "next_page": null, "total_pages": 1, "total_records": 7}"#,
    )
    .unwrap();
    assert_eq!(p.next_page, None);

    // Juz without the id-range extras.
    let j: Juz =
        serde_json::from_str(r#"{"id": 2, "juz_number": 2, "verse_mapping": {"2": "142-252"}}"#)
            .unwrap();
    assert_eq!(
        (j.first_verse_id, j.last_verse_id, j.verses_count),
        (None, None, None)
    );

    // Resources with only required keys.
    let tr: TranslationResource = serde_json::from_str(
        r#"{"id": 20, "name": "Saheeh International", "language_name": "english"}"#,
    )
    .unwrap();
    assert_eq!(
        (tr.author_name, tr.slug, tr.translated_name),
        (None, None, None)
    );
    let l: Language =
        serde_json::from_str(r#"{"id": 38, "name": "English", "iso_code": "en"}"#).unwrap();
    assert_eq!(
        (l.native_name, l.direction, l.translations_count),
        (None, None, None)
    );
    let f: FootNote = serde_json::from_str(r#"{"id": 1, "text": "note"}"#).unwrap();
    assert_eq!(f.language_name, None);

    // Search with no results key and a bare result.
    let s: Search = serde_json::from_str(
        r#"{"query": "zzz", "total_results": 0, "current_page": 1, "total_pages": 0}"#,
    )
    .unwrap();
    assert!(s.results.is_empty());
    let sr: SearchResult = serde_json::from_str(r#"{"verse_key": "1:1", "text": "بِسْمِ"}"#).unwrap();
    assert_eq!((sr.verse_id, sr.highlighted), (None, None));
    assert!(sr.translations.is_empty());
    let st: SearchTranslation = serde_json::from_str(r#"{"text": "x"}"#).unwrap();
    assert_eq!(
        (st.resource_id, st.name, st.language_name),
        (None, None, None)
    );
}

#[test]
fn missing_required_field_is_an_error() {
    // Sanity check that tolerance isn't accidental: verse_key is required.
    let err = serde_json::from_str::<Verse>(r#"{"id": 1, "verse_number": 1}"#).unwrap_err();
    assert!(err.to_string().contains("verse_key"), "{err}");
}

/// Regression: the API repeats `line_number`/`page_number` in a word when they are
/// also requested via `word_fields`; that must not fail the whole response.
#[test]
fn duplicate_keys_from_the_api_are_tolerated() {
    let body = r#"{"verses":[{"id":2,"verse_number":1,"verse_key":"107:1","page_number":602,"page_number":602,
        "words":[{"id":1,"position":1,"char_type_name":"word","line_number":3,"page_number":602,
                  "code_v2":"\uFB51","line_number":3,"page_number":602,"v2_page":602},
                 {"id":2,"position":2,"char_type_name":"end","line_number":3,"line_number":3}]}],
        "pagination":{"per_page":50,"current_page":1,"next_page":null,"total_pages":1,"total_records":7}}"#;
    // Strict derive would fail…
    assert!(serde_json::from_str::<VersesResponse>(body).is_err());
    // …the client's parser keeps the (identical) last value.
    let r: VersesResponse = quran_core::api::parse_json(body).expect("duplicate keys must be tolerated");
    let w = &r.verses[0].words[0];
    assert_eq!((w.page_number, w.line_number, w.v2_page), (Some(602), Some(3), Some(602)));
    assert_eq!(r.verses[0].words[1].line_number, Some(3));
}
