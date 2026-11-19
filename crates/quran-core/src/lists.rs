//! The surah and juz lists: lookups and the surah filter.

use crate::api::{Chapter, Juz};
use crate::parse;

/// Find a chapter by number in a chapter list.
pub fn find_chapter(list: &[Chapter], id: u16) -> Option<&Chapter> {
    list.iter().find(|c| c.id == id)
}

/// The juz that contains a verse.
pub fn juz_of(juzs: &[Juz], chapter: u16, verse: u16) -> Option<u8> {
    juzs.iter()
        .find(|j| parse::juz_mapping(&j.verse_mapping).iter().any(|(c, span)| *c == chapter && span.contains(verse)))
        .map(|j| j.juz_number)
}

/// The API has been known to return each juz more than once; keep the first of each, in order.
pub fn clean_juzs(mut list: Vec<Juz>) -> Vec<Juz> {
    list.sort_by_key(|j| j.juz_number);
    list.dedup_by_key(|j| j.juz_number);
    list
}

/// Lowercase and keep only letters/digits, so "Al-Kahf" matches "al kahf" / "alkahf".
fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Filter predicate for the surah list: number prefix, or name substring
/// (simple, complex, translated or Arabic name).
pub fn chapter_matches(c: &Chapter, filter: &str) -> bool {
    let f = filter.trim();
    if f.is_empty() {
        return true;
    }
    if f.chars().all(|ch| ch.is_ascii_digit()) {
        return c.id.to_string().starts_with(f);
    }
    // "2:255" style input filters by chapter number.
    if let Ok(r) = parse::parse_chapter_segment(f) {
        return c.id == r.chapter;
    }
    let needle = normalize(f);
    if needle.is_empty() {
        return true;
    }
    [&c.name_simple, &c.name_complex, &c.translated_name.name, &c.name_arabic]
        .iter()
        .any(|name| normalize(name).contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(id: u16, simple: &str, translated: &str, arabic: &str) -> Chapter {
        serde_json::from_value(serde_json::json!({
            "id": id, "revelation_place": "makkah", "revelation_order": 1, "bismillah_pre": true,
            "name_simple": simple, "name_complex": simple, "name_arabic": arabic,
            "verses_count": 7, "translated_name": { "name": translated }
        }))
        .unwrap()
    }

    fn juz(n: u8, chapter: u16, range: &str) -> Juz {
        serde_json::from_value(serde_json::json!({
            "id": n, "juz_number": n, "verse_mapping": { chapter.to_string(): range }
        }))
        .unwrap()
    }

    #[test]
    fn filter_by_number_and_name() {
        let kahf = chapter(18, "Al-Kahf", "The Cave", "الكهف");
        assert!(chapter_matches(&kahf, ""));
        assert!(chapter_matches(&kahf, "18"));
        assert!(chapter_matches(&kahf, "1"));
        assert!(!chapter_matches(&kahf, "2"));
        assert!(chapter_matches(&kahf, "kahf"));
        assert!(chapter_matches(&kahf, "Al Kahf"));
        assert!(chapter_matches(&kahf, "alkahf"));
        assert!(chapter_matches(&kahf, "cave"));
        assert!(chapter_matches(&kahf, "الكهف"));
        assert!(chapter_matches(&kahf, "18:10"));
        assert!(!chapter_matches(&kahf, "2:255"));
        assert!(!chapter_matches(&kahf, "baqarah"));
        assert!(chapter_matches(&kahf, " - "));
    }

    #[test]
    fn juz_lookup_and_cleanup() {
        let juzs = clean_juzs(vec![juz(2, 2, "142-252"), juz(1, 2, "1-141"), juz(2, 2, "142-252")]);
        assert_eq!(juzs.iter().map(|j| j.juz_number).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(juz_of(&juzs, 2, 141), Some(1));
        assert_eq!(juz_of(&juzs, 2, 142), Some(2));
        assert_eq!(juz_of(&juzs, 3, 1), None);
        let list = [chapter(18, "Al-Kahf", "The Cave", "الكهف")];
        assert_eq!(find_chapter(&list, 18).map(|c| c.id), Some(18));
        assert!(find_chapter(&list, 19).is_none());
    }
}
