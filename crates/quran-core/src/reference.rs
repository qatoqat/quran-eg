//! Reference data the loaded verses are checked against, so a response that
//! lacks a verse, a word or part of a line is noticed even when it is consistent
//! in itself.
//!
//! Small tables are compiled in (built by `tools/reference/build.py` from a
//! full read of the API): the number of verses in each surah, of words in each
//! verse and on each line of each mushaf page, the verses on each page, and the
//! verse each of the API's page listings begins with. Looking something up is
//! an array index or a short search.
//!
//! "Page" means two things in the API. The layout the fonts are drawn for puts
//! each word on a page and a line; the listing `/verses/by_page/N` follows an
//! older layout, and at some page turns a verse is listed on one side but
//! printed on the other (83:34 is listed under 588 and printed on line 1 of
//! 589). The mushaf pages shown are always the printed ones.

use std::sync::OnceLock;

use crate::parse::{CHAPTER_COUNT, VerseSpan};

/// Lines per page in the 15-line Madani mushaf.
const LINES: usize = 15;

/// One byte per verse in mushaf order: its words, not counting the ayah marker.
const WORDS_PER_VERSE: &[u8] = include_bytes!("reference/words_per_verse.bin");
/// 15 bytes per page: the words (markers included) on each line; 0 = no text there.
const WORDS_PER_LINE: &[u8] = include_bytes!("reference/words_per_line.bin");
/// One line per surah: its number of verses.
const VERSES_PER_CHAPTER: &str = include_str!("reference/verses_per_chapter.txt");
/// One line per page: `c:v c:v`, the first and last verse with a word on it.
const PAGE_VERSES: &str = include_str!("reference/page_verses.txt");
/// One line per page: `c:v`, the first verse `/verses/by_page/N` returns.
const LISTING_STARTS: &str = include_str!("reference/listing_starts.txt");

type Key = (u16, u16);

struct Tables {
    /// Verses per surah (index 0 = surah 1).
    verses: Vec<u16>,
    /// Index into `WORDS_PER_VERSE` of each surah's first verse.
    offsets: Vec<usize>,
    /// First and last verse printed on each page (index 0 = page 1).
    page_verses: Vec<(Key, Key)>,
    /// First verse of each page listing (index 0 = page 1).
    listing_starts: Vec<Key>,
}

fn key(s: &str) -> Option<Key> {
    let (c, v) = s.trim().split_once(':')?;
    Some((c.parse().ok()?, v.parse().ok()?))
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let verses: Vec<u16> = VERSES_PER_CHAPTER.lines().filter_map(|l| l.trim().parse().ok()).collect();
        let offsets = verses
            .iter()
            .scan(0usize, |next, n| {
                let start = *next;
                *next += usize::from(*n);
                Some(start)
            })
            .collect();
        let page_verses = PAGE_VERSES
            .lines()
            .filter_map(|l| {
                let (first, last) = l.trim().split_once(' ')?;
                Some((key(first)?, key(last)?))
            })
            .collect();
        let listing_starts = LISTING_STARTS.lines().filter_map(key).collect();
        Tables { verses, offsets, page_verses, listing_starts }
    })
}

/// Number of verses in a surah.
pub fn verses_in(chapter: u16) -> Option<u16> {
    tables().verses.get(usize::from(chapter.checked_sub(1)?)).copied()
}

/// Number of words in a verse (the ayah marker not counted).
pub fn words_in(chapter: u16, verse: u16) -> Option<usize> {
    let t = tables();
    let c = usize::from(chapter.checked_sub(1)?);
    if verse == 0 || verse > *t.verses.get(c)? {
        return None;
    }
    WORDS_PER_VERSE.get(t.offsets[c] + usize::from(verse) - 1).map(|n| usize::from(*n))
}

/// Words (ayah markers included) on a line of a mushaf page; 0 for a line
/// without text (surah banner, Bismillah, or one the page doesn't have).
pub fn words_on_line(page: u16, line: u16) -> Option<usize> {
    let (p, l) = (usize::from(page.checked_sub(1)?), usize::from(line.checked_sub(1)?));
    if l >= LINES {
        return None;
    }
    WORDS_PER_LINE.get(p * LINES + l).map(|n| usize::from(*n))
}

/// First and last verse printed on a mushaf page.
fn page_range(page: u16) -> Option<(Key, Key)> {
    tables().page_verses.get(usize::from(page.checked_sub(1)?)).copied()
}

/// The verses with a word on a mushaf page, as `(chapter, span)` per surah. A
/// verse that runs over a page turn is on both pages.
pub fn verses_on_page(page: u16) -> Vec<(u16, VerseSpan)> {
    let Some(((first_chapter, first_verse), (last_chapter, last_verse))) = page_range(page) else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    for chapter in first_chapter..=last_chapter.min(CHAPTER_COUNT) {
        let from = if chapter == first_chapter { first_verse } else { 1 };
        let to = if chapter == last_chapter { last_verse } else { verses_in(chapter).unwrap_or(0) };
        if from <= to {
            spans.push((chapter, VerseSpan { from, to }));
        }
    }
    spans
}

/// The `/verses/by_page` listings that together hold every verse printed on a
/// mushaf page: its own, and a neighbour's where a verse is listed across the turn.
pub fn listings_for_page(page: u16) -> Vec<u16> {
    let Some((first, last)) = page_range(page) else { return Vec::new() };
    let starts = &tables().listing_starts;
    // The listing a verse is in: the last one that starts at or before it.
    let listing = |k: Key| starts.partition_point(|s| *s <= k).max(1) as u16;
    (listing(first)..=listing(last)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::MUSHAF_PAGES;

    #[test]
    fn tables_are_whole() {
        let t = tables();
        assert_eq!(t.verses.len(), usize::from(CHAPTER_COUNT));
        assert_eq!(t.verses.iter().map(|n| u32::from(*n)).sum::<u32>(), 6236);
        assert_eq!(WORDS_PER_VERSE.len(), 6236);
        assert!(WORDS_PER_VERSE.iter().all(|n| *n > 0));
        assert_eq!(t.page_verses.len(), usize::from(MUSHAF_PAGES));
        assert_eq!(t.listing_starts.len(), usize::from(MUSHAF_PAGES));
        assert!(t.listing_starts.windows(2).all(|w| w[0] < w[1]), "listings begin in mushaf order");
        assert!(t.page_verses.iter().all(|(first, last)| first <= last));
        // A page begins where the one before ends, or with the verse after it.
        assert!(t.page_verses.windows(2).all(|w| w[0].1 <= w[1].0), "pages in mushaf order");
        // Every word and every marker is on exactly one line.
        assert_eq!(WORDS_PER_LINE.len(), usize::from(MUSHAF_PAGES) * LINES);
        let on_lines: usize = WORDS_PER_LINE.iter().map(|n| usize::from(*n)).sum();
        let in_verses: usize = WORDS_PER_VERSE.iter().map(|n| usize::from(*n) + 1).sum();
        assert_eq!(on_lines, in_verses);
    }

    #[test]
    fn lookups() {
        assert_eq!((verses_in(1), verses_in(2), verses_in(114)), (Some(7), Some(286), Some(6)));
        assert_eq!((verses_in(0), verses_in(115)), (None, None));
        // 1:1 is the Bismillah (4 words); 112:1 "قل هو الله أحد" has 4, 108:1 has 3.
        assert_eq!((words_in(1, 1), words_in(112, 1), words_in(108, 1)), (Some(4), Some(4), Some(3)));
        assert_eq!((words_in(1, 8), words_in(1, 0), words_in(115, 1)), (None, None, None));
        assert!(words_in(114, 6).is_some());
        assert_eq!((words_on_line(1, 0), words_on_line(1, 16), words_on_line(605, 1)), (None, None, None));
        // Page 1 is Al-Fatihah: 29 words and 7 markers.
        assert_eq!((1..=15).filter_map(|l| words_on_line(1, l)).sum::<usize>(), 36);
    }

    #[test]
    fn pages_hold_the_verses_printed_on_them() {
        assert_eq!(verses_on_page(1), [(1, VerseSpan { from: 1, to: 7 })]);
        assert_eq!(verses_on_page(2), [(2, VerseSpan { from: 1, to: 5 })]);
        let last = verses_on_page(604);
        assert_eq!(last.iter().map(|(c, _)| *c).collect::<Vec<_>>(), [112, 113, 114]);
        assert_eq!(last[2], (114, VerseSpan { from: 1, to: 6 }));
        assert!(verses_on_page(0).is_empty() && verses_on_page(605).is_empty());
        // Every verse is on a page, and none is skipped between pages.
        let total: u32 = (1..=MUSHAF_PAGES)
            .flat_map(verses_on_page)
            .map(|(_, s)| u32::from(s.to - s.from) + 1)
            .sum();
        assert!(total >= 6236, "{total}");
    }

    #[test]
    fn a_page_takes_the_neighbouring_listing_when_a_verse_is_listed_across_the_turn() {
        // 83:34 is listed under page 588 but printed on line 1 of 589, before 83:35's first word.
        assert_eq!(verses_on_page(589)[0], (83, VerseSpan { from: 34, to: 36 }));
        assert_eq!(listings_for_page(589), [588, 589]);
        assert_eq!(words_on_line(589, 1), Some(8));
        assert_eq!(listings_for_page(1), [1]);
        assert_eq!(listings_for_page(604), [604]);
        assert!(listings_for_page(0).is_empty());
        // No page needs more than its own listing and its two neighbours.
        for page in 1..=MUSHAF_PAGES {
            let listings = listings_for_page(page);
            assert!(!listings.is_empty() && listings.len() <= 3, "page {page}: {listings:?}");
            assert!(listings.iter().all(|l| l.abs_diff(page) <= 1), "page {page}: {listings:?}");
        }
    }
}
