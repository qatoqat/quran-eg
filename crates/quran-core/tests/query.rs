//! `VerseQuery::to_pairs` and `Client::url` encoding.

use pretty_assertions::assert_eq;
use quran_core::api::{Client, DEFAULT_BASE_URL, VerseQuery};

fn get<'a>(pairs: &'a [(&'static str, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.as_str())
}

fn keys(pairs: &[(&'static str, String)]) -> Vec<&'static str> {
    pairs.iter().map(|(k, _)| *k).collect()
}

#[test]
fn default_query_pairs() {
    let pairs = VerseQuery::default().to_pairs();
    assert_eq!(
        pairs,
        vec![
            ("language", "en".to_string()),
            ("words", "true".to_string()),
            ("page", "1".to_string()),
            ("per_page", "50".to_string()),
            ("translations", "131".to_string()),
            ("fields", "text_uthmani".to_string()),
            ("word_fields", "text_uthmani".to_string()),
            (
                "translation_fields",
                "resource_name,language_name".to_string()
            ),
        ]
    );
}

#[test]
fn per_page_is_capped_at_50() {
    for (asked, sent) in [
        (1, "1"),
        (10, "10"),
        (50, "50"),
        (51, "50"),
        (500, "50"),
        (u32::MAX, "50"),
    ] {
        let q = VerseQuery {
            per_page: asked,
            ..Default::default()
        };
        assert_eq!(
            get(&q.to_pairs(), "per_page"),
            Some(sent),
            "per_page={asked}"
        );
    }
}

#[test]
fn empty_lists_are_omitted() {
    let q = VerseQuery {
        translations: vec![],
        fields: vec![],
        word_fields: vec![],
        translation_fields: vec![],
        words: false,
        ..Default::default()
    };
    let pairs = q.to_pairs();
    assert_eq!(keys(&pairs), ["language", "words", "page", "per_page"]);
    assert_eq!(get(&pairs, "words"), Some("false"));
}

#[test]
fn lists_are_comma_joined() {
    let q = VerseQuery {
        language: "ur".into(),
        translations: vec![131, 20, 97],
        fields: vec!["text_uthmani".into(), "text_indopak".into()],
        word_fields: vec![
            "text_uthmani".into(),
            "text_indopak".into(),
            "location".into(),
        ],
        translation_fields: vec!["resource_name".into()],
        page: 3,
        per_page: 10,
        words: true,
        mushaf: Some(19),
    };
    let pairs = q.to_pairs();
    assert_eq!(get(&pairs, "language"), Some("ur"));
    assert_eq!(get(&pairs, "translations"), Some("131,20,97"));
    assert_eq!(get(&pairs, "fields"), Some("text_uthmani,text_indopak"));
    assert_eq!(
        get(&pairs, "word_fields"),
        Some("text_uthmani,text_indopak,location")
    );
    assert_eq!(get(&pairs, "translation_fields"), Some("resource_name"));
    assert_eq!(get(&pairs, "page"), Some("3"));
    assert_eq!(get(&pairs, "per_page"), Some("10"));
    assert_eq!(get(&pairs, "mushaf"), Some("19"));
    // Each key appears exactly once.
    let mut ks = keys(&pairs);
    ks.sort_unstable();
    ks.dedup();
    assert_eq!(ks.len(), pairs.len());
}

#[test]
fn with_page_changes_only_page() {
    let q = VerseQuery {
        translations: vec![20],
        per_page: 7,
        ..Default::default()
    };
    let q2 = q.with_page(4);
    assert_eq!(q2.page, 4);
    assert_eq!(
        VerseQuery {
            page: q.page,
            ..q2.clone()
        },
        q
    );
}

#[test]
fn default_client_uses_public_endpoint() {
    let c = Client::default();
    assert_eq!(c.base_url(), DEFAULT_BASE_URL);
    assert_eq!(
        c.url("/chapters", &[]),
        "https://api.quran.com/api/v4/chapters"
    );
}

#[test]
fn trailing_slashes_are_trimmed() {
    let c = Client::new("http://localhost:3000/api/v4///");
    assert_eq!(c.base_url(), "http://localhost:3000/api/v4");
    assert_eq!(c.url("/juzs", &[]), "http://localhost:3000/api/v4/juzs");
    assert_eq!(
        Client::new("http://localhost:3000/api/v4/"),
        Client::new("http://localhost:3000/api/v4")
    );
}

#[test]
fn url_with_verse_query() {
    let c = Client::default();
    let q = VerseQuery {
        translations: vec![131, 20],
        translation_fields: vec![],
        per_page: 100,
        ..Default::default()
    };
    assert_eq!(
        c.url("/verses/by_chapter/1", &q.to_pairs()),
        "https://api.quran.com/api/v4/verses/by_chapter/1?language=en&words=true&page=1&per_page=50\
         &translations=131,20&fields=text_uthmani&word_fields=text_uthmani"
    );
}

#[test]
fn url_encodes_reserved_and_unicode() {
    let c = Client::new("https://example.com/api/v4");
    let url = c.url(
        "/search",
        &[
            ("q", "light & dark = 100%?".into()),
            ("language", "en".into()),
            ("x", "a+b/c#d".into()),
        ],
    );
    assert_eq!(
        url,
        "https://example.com/api/v4/search?q=light%20%26%20dark%20%3D%20100%25%3F&language=en&x=a%2Bb%2Fc%23d"
    );

    // Arabic is percent-encoded as UTF-8; unreserved chars and ',' are kept.
    assert_eq!(
        c.url("/search", &[("q", "رحمة".into())]),
        "https://example.com/api/v4/search?q=%D8%B1%D8%AD%D9%85%D8%A9"
    );
    assert_eq!(
        c.url("/s", &[("q", "Az09-_.~,".into())]),
        "https://example.com/api/v4/s?q=Az09-_.~,"
    );
    // Empty value still emits the key.
    assert_eq!(
        c.url("/s", &[("q", String::new())]),
        "https://example.com/api/v4/s?q="
    );
}
