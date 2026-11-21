//! What a copy for reading offline is made of, for the settings: the lists the app shows, verse 1:1 and every
//! surah's verses, request by request as the reader makes them (so what the reader asks for is what is kept), and
//! the fonts of the mushaf font, file by file as the font store keeps them; and how much of it is kept.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use quran_core::api::{Call, Client, VerseQuery};
use quran_core::feed::VerseSource;
use quran_core::fonts::{FALLBACK_PATH, FontSet, cache_name};
use quran_core::offline::surah_requests;
use quran_core::parse::{CHAPTER_COUNT, MUSHAF_PAGES};
use quran_core::settings::{LANGUAGE, Settings};

use super::store::{self, Store};
use crate::arabic::ArabicFont;
use crate::fonts::{FontStore, MAX_FONT_BYTES, ORNAMENT_BASE, ORNAMENT_FRAME, ORNAMENT_NAMES};
use crate::glyphs::parse_page_font;

/// Whether an answer (its status and body) may be kept.
type Keeps = Box<dyn Fn(u16, &[u8]) -> bool + Send>;

/// A request a copy for reading offline is made of, and whether an answer to it may be kept.
pub struct Need {
    pub url: String,
    /// The surah it is of (the lists are of none).
    pub surah: Option<u16>,
    pub keeps: Keeps,
}

/// `call` as a part of the copy (of `surah`), its answer checked as the reader checks it before keeping it.
pub fn need<T: 'static>(call: Call<T>, surah: Option<u16>) -> Need {
    let url = call.url.clone();
    Need { url, surah, keeps: Box::new(move |status, body| call.read(status, body).is_ok_and(|answer| call.keeps(&answer))) }
}

/// What a copy for reading offline is made of, for `query`: the lists the app shows and verse 1:1 (the Bismillah
/// line), then every surah's verses in order.
pub fn needs(client: &Client, query: &VerseQuery) -> Vec<Need> {
    let mut needs = vec![
        need(client.chapters_call(LANGUAGE), None),
        need(client.juzs_call(), None),
        need(client.translations_call(LANGUAGE), None),
        need(client.verse_by_key_call("1:1", query), None),
    ];
    for chapter in 1..=CHAPTER_COUNT {
        let call = |request| VerseSource::Chapter { chapter, span: None }.call(client, query, request);
        needs.extend((1..=surah_requests(chapter)).map(|request| need(call(request), Some(chapter))));
    }
    needs
}

/// A font the reader draws with, as a copy fetches it: its path below the bases it is looked for in (in order), the
/// page it is the font of, and whether a file is that font (as the font store checks it).
pub struct FontFile {
    pub path: String,
    pub bases: Arc<Vec<String>>,
    pub page: Option<u16>,
    pub good: fn(&[u8]) -> bool,
}

/// The fonts the reader draws with `set` from `bases`: the fallback text font and the banner's ornaments, then every
/// page's font.
pub fn fonts(bases: Vec<String>, set: FontSet) -> Vec<FontFile> {
    let (bases, ornaments) = (Arc::new(bases), Arc::new(vec![ORNAMENT_BASE.to_string()]));
    let text: fn(&[u8]) -> bool = |b| b.len() <= MAX_FONT_BYTES && ArabicFont::from_bytes(b).is_ok();
    let page: fn(&[u8]) -> bool = |b| b.len() <= MAX_FONT_BYTES && parse_page_font(b).is_ok();
    let file = |path: String, bases: &Arc<Vec<String>>, page, good| FontFile { path, bases: bases.clone(), page, good };
    let mut files = vec![
        file(FALLBACK_PATH.into(), &bases, None, text),
        file(ORNAMENT_FRAME.into(), &ornaments, None, text),
        file(ORNAMENT_NAMES.into(), &ornaments, None, text),
    ];
    files.extend((1..=MUSHAF_PAGES).map(|p| file(set.path(p), &bases, Some(p), page)));
    files
}

/// The font store's cache (none in tests, which must leave it alone).
pub fn font_dir() -> Option<PathBuf> {
    FontStore::user_cache().filter(|_| !cfg!(test))
}

/// What is saved for reading offline.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Survey {
    /// Surahs whose every request is kept, for the settings asked about.
    pub surahs: usize,
    /// Page fonts kept, of the mushaf font asked about.
    pub fonts: usize,
    /// Everything a download fetches for them is kept.
    pub whole: bool,
    /// When the oldest of the surahs' answers was fetched.
    pub oldest: Option<SystemTime>,
    /// How much is kept of the API's answers in all (an update asks for all of it again), and of the fonts the mushaf
    /// font draws with, in bytes.
    pub bytes: u64,
    pub font_bytes: u64,
}

/// What is saved for reading offline with `settings`; `None` without a cache folder. (It reads the disk: off the
/// frame.)
pub fn survey(client: &Client, settings: &Settings) -> Option<Survey> {
    let fonts = fonts(settings.font_bases(), FontSet::of(settings.mushaf_font));
    Some(survey_of(store::user()?, &needs(client, &settings.verse_query()), &font_dir()?, &fonts))
}

pub fn survey_of(store: &Store, needs: &[Need], dir: &Path, fonts: &[FontFile]) -> Survey {
    let fetched: Vec<Option<SystemTime>> = needs.iter().map(|need| store.fetched(&need.url)).collect();
    let kept = |c: u16| {
        let mut of = needs.iter().zip(&fetched).filter(|(need, _)| need.surah == Some(c)).peekable();
        of.peek().is_some() && of.all(|(_, at)| at.is_some())
    };
    let sizes: Vec<Option<u64>> = fonts.iter().map(|f| std::fs::metadata(dir.join(cache_name(&f.path))).ok().map(|m| m.len())).collect();
    Survey {
        surahs: (1..=CHAPTER_COUNT).filter(|c| kept(*c)).count(),
        fonts: fonts.iter().zip(&sizes).filter(|(f, size)| f.page.is_some() && size.is_some()).count(),
        whole: fetched.iter().all(Option::is_some) && sizes.iter().all(Option::is_some),
        oldest: needs.iter().zip(&fetched).filter(|(need, _)| need.surah.is_some()).filter_map(|(_, at)| *at).min(),
        bytes: store.list().iter().map(|(_, size)| size).sum(),
        font_bytes: sizes.iter().flatten().sum(),
    }
}

#[cfg(test)]
mod tests {
    use quran_core::settings::Settings;

    use super::*;
    use crate::offline::store::tests::{remove, scratch, scratch_dir};

    #[test]
    fn a_copy_is_the_lists_and_every_surah() {
        let (client, query) = (Client::default(), Settings::default().verse_query());
        let needs = needs(&client, &query);
        assert_eq!(needs.len(), 4 + 190);
        assert!(needs[0].url.ends_with("/chapters?language=en") && needs[3].url.contains("/verses/by_key/1:1?"));
        assert_eq!(needs.iter().filter(|n| n.surah == Some(2)).count(), 6);
        // The reader's own requests: what it asks for a surah is what is kept.
        assert_eq!(needs[4].url, VerseSource::Chapter { chapter: 1, span: None }.call(&client, &query, 1).url);
    }

    #[test]
    fn the_fonts_are_the_text_and_banner_fonts_then_every_page() {
        let bases = vec!["https://fonts.example/quran".to_string()];
        let files = fonts(bases.clone(), FontSet::V4Colr);
        assert_eq!(files.len(), 3 + 604);
        assert_eq!((files[0].path.as_str(), files[0].bases.as_slice(), files[0].page), (FALLBACK_PATH, &bases[..], None));
        assert_eq!(files[1].bases.as_slice(), [ORNAMENT_BASE]);
        assert_eq!((files[3].path.as_str(), files[3].page), ("hafs/v4/colrv1/woff2/p1.woff2", Some(1)));
        assert_eq!(fonts(bases, FontSet::V2)[606].path, "hafs/v2/woff2/p604.woff2");
        assert!(!(files[3].good)(b"not a font") && !(files[0].good)(b"nor this"));
    }

    #[test]
    fn a_surah_is_saved_once_all_its_requests_are_and_a_font_once_its_file_is() {
        let (store, client, query) = (scratch("survey"), Client::default(), Settings::default().verse_query());
        let (needs, fonts) = (needs(&client, &query), fonts(Vec::new(), FontSet::V4Colr));
        let dir = scratch_dir("survey-fonts");
        assert_eq!(survey_of(&store, &needs, &dir, &fonts), Survey::default());
        let baqarah: Vec<_> = needs.iter().filter(|n| n.surah == Some(2)).collect();
        for need in &baqarah[..5] {
            store.write(&need.url, b"{}").unwrap();
        }
        assert_eq!(survey_of(&store, &needs, &dir, &fonts).surahs, 0, "one of its six requests is not kept");
        store.write(&baqarah[5].url, b"{}").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        for file in &fonts[..5] {
            std::fs::write(dir.join(cache_name(&file.path)), b"font").unwrap();
        }
        let survey = survey_of(&store, &needs, &dir, &fonts);
        let bytes = 6 * 2 + baqarah.iter().map(|n| n.url.len() as u64 + 1).sum::<u64>();
        assert_eq!((survey.surahs, survey.fonts, survey.whole, survey.bytes, survey.font_bytes), (1, 2, false, bytes, 20));
        assert!(survey.oldest.is_some());
        remove(&store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
