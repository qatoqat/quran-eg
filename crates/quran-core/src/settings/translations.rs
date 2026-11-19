//! The translation picker's rules: which translations are offered, how they
//! are filtered and named.

use crate::api::TranslationResource;

/// `(id, name)` of English translations, used when `/resources/translations` fails.
pub const FALLBACK_TRANSLATIONS: &[(u32, &str)] = &[
    (20, "Saheeh International"),
    (85, "Abdul Haleem"),
    (84, "Mufti Taqi Usmani"),
    (19, "Pickthall"),
    (22, "Yusuf Ali"),
];

pub fn fallback_translations() -> Vec<TranslationResource> {
    FALLBACK_TRANSLATIONS
        .iter()
        .map(|(id, name)| TranslationResource {
            id: *id,
            name: (*name).into(),
            author_name: None,
            slug: None,
            language_name: "english".into(),
            translated_name: None,
        })
        .collect()
}

pub fn display_name(t: &TranslationResource) -> &str {
    t.translated_name.as_ref().map(|n| n.name.trim()).filter(|n| !n.is_empty()).unwrap_or(t.name.trim())
}

/// English translations only (by `language_name`, case-insensitive), without
/// duplicate ids, sorted by display name.
pub fn english_translations(all: &[TranslationResource]) -> Vec<TranslationResource> {
    let mut out: Vec<TranslationResource> =
        all.iter().filter(|t| t.language_name.trim().eq_ignore_ascii_case("english")).cloned().collect();
    out.sort_by_cached_key(|t| (display_name(t).to_lowercase(), t.id));
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.id));
    out
}

/// Case-insensitive match on display name, name or author.
pub fn matches_filter(t: &TranslationResource, filter: &str) -> bool {
    let needle = filter.trim().to_lowercase();
    needle.is_empty()
        || [Some(t.name.as_str()), Some(display_name(t)), t.author_name.as_deref()]
            .into_iter()
            .flatten()
            .any(|s| s.to_lowercase().contains(&needle))
}

/// Toggle `id` in the ordered selection (appending when added).
pub fn toggle_id(list: &mut Vec<u32>, id: u32, on: bool) {
    if on {
        if !list.contains(&id) {
            list.push(id);
        }
    } else {
        list.retain(|x| *x != id);
    }
}

/// Label for a selected id: its name if known, else `Translation #id` (e.g. a
/// non-English id kept from an older version, which the user can remove).
pub fn selected_label(id: u32, known: &[TranslationResource]) -> String {
    known.iter().find(|t| t.id == id).map(|t| display_name(t).to_string()).unwrap_or_else(|| format!("Translation #{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(id: u32, name: &str, lang: &str, author: Option<&str>) -> TranslationResource {
        TranslationResource {
            id,
            name: name.into(),
            author_name: author.map(Into::into),
            slug: None,
            language_name: lang.into(),
            translated_name: None,
        }
    }

    fn sample() -> Vec<TranslationResource> {
        vec![
            tr(20, "Saheeh International", "english", Some("Saheeh International")),
            tr(131, "Clear Quran", "English", Some("Mustafa Khattab")),
            tr(97, "Maududi", "urdu", Some("Abul Ala Maududi")),
            tr(33, "Indonesian Ministry", "indonesian", None),
            tr(85, "Abdul Haleem", " ENGLISH ", Some("Abdul Haleem")),
            tr(20, "Saheeh International (dup)", "english", None),
        ]
    }

    #[test]
    fn only_english_sorted_and_deduped() {
        let ids: Vec<u32> = english_translations(&sample()).iter().map(|t| t.id).collect();
        assert_eq!(ids, [85, 131, 20]);
        assert!(english_translations(&[]).is_empty());
    }

    #[test]
    fn fallback_list_is_english() {
        let list = fallback_translations();
        assert_eq!(list.iter().map(|t| t.id).collect::<Vec<_>>(), [20, 85, 84, 19, 22]);
        assert!(list.iter().any(|t| crate::settings::Settings::default().translations.contains(&t.id)));
        assert_eq!(english_translations(&list).len(), list.len());
    }

    #[test]
    fn filter_matches_name_and_author() {
        let en = english_translations(&sample());
        let ids = |f: &str| -> Vec<u32> { en.iter().filter(|t| matches_filter(t, f)).map(|t| t.id).collect() };
        assert_eq!(ids("khattab"), [131]);
        assert_eq!(ids("  SAHEEH "), [20]);
        assert_eq!(ids(""), [85, 131, 20]);
        assert!(ids("nothing-matches").is_empty());
    }

    #[test]
    fn selected_labels_fall_back_to_id() {
        let en = english_translations(&sample());
        assert_eq!(selected_label(131, &en), "Clear Quran");
        assert_eq!(selected_label(97, &en), "Translation #97");
    }

    #[test]
    fn toggle_keeps_order_and_allows_empty() {
        let mut v = vec![131];
        toggle_id(&mut v, 20, true);
        toggle_id(&mut v, 20, true);
        assert_eq!(v, [131, 20]);
        toggle_id(&mut v, 131, false);
        toggle_id(&mut v, 20, false);
        assert!(v.is_empty());
    }
}
