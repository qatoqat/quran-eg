//! Per-page state of a reader view: a page's height, its placement and its
//! sprite sheet. Only pages near the viewport have the last two; they are
//! dropped when the page scrolls away.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{ColorImage, TextureHandle, TextureOptions};
use quran_core::api::{Chapter, Verse};
use quran_core::fonts::FontSet;
use quran_core::glyph::{self, arabic_number};
use quran_core::lists::find_chapter;
use quran_core::mushaf_layout::{Item, PageBlock};
use quran_core::parse::revelation_label;

use super::place::{Placed, label_key, word_key};
use crate::arabic::{ArabicFont, BANNER_FRAME_TEXT, Ornaments, banner_name_text};
use crate::glyphs::{PageGlyphs, Palette};
use crate::sheet::{self, Request, Sprite, Text, TextFont};

/// Unicode Bismillah, the last-resort text of the Bismillah line.
const BISMILLAH: &str = "بِسْمِ ٱللَّهِ ٱلرَّحْمَٰنِ ٱلرَّحِيمِ";

/// Sizes of fallback text relative to the glyph em (as in quran-dx's stylesheet).
const QPC_EM: f32 = 0.82;
const UTHMANI_EM: f32 = 0.8;
const NUMBER_EM: f32 = 0.5;
pub const BANNER_NAME_EM: f32 = 0.6;
pub const BANNER_SIDE_EM: f32 = 0.34;
/// Sizes the ornamental banner's frame and name are drawn at (they are then
/// fitted: the frame to one line's height, the name into its cartouche).
const BANNER_FRAME_EM: f32 = 1.5;
const BANNER_ORNAMENT_NAME_EM: f32 = 1.3;

/// Labels of a surah banner, as parts of its sprite key.
pub mod banner {
    pub const NAME: usize = 0;
    pub const PLACE: usize = 1;
    /// "آياتها" and the number of verses.
    pub const VERSES: usize = 2;
    pub const COUNT: usize = 3;
    /// The ornamental frame (one for the sheet) and calligraphic name.
    pub const FRAME: usize = 4;
    pub const ORNAMENT_NAME: usize = 5;
}

/// A page of the view: its number and which block of that page it is.
pub type BlockKey = (Option<u16>, u32);

/// What a sheet was (or is being) drawn from.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// Device pixels per glyph em.
    pub px_per_em: f32,
    pub palette: Palette,
    pub set: FontSet,
    /// Which fonts were there: the page's, page 1's (Bismillah), the fallback
    /// and the banner ornaments.
    pub fonts: [bool; 4],
    /// Changes when the page's words or labels change.
    pub content: u64,
    /// Strength of the reading pointer's glow (0 = none): with it, the sheet
    /// comes with a second, more vivid copy for the pointer's lens.
    pub glow: u8,
}

impl Spec {
    /// Same sheet apart from its resolution.
    pub fn same_but_size(&self, other: &Spec) -> bool {
        Spec { px_per_em: 0.0, ..self.clone() } == Spec { px_per_em: 0.0, ..other.clone() }
    }
}

pub struct LoadedSheet {
    pub spec: Spec,
    pub texture: TextureHandle,
    /// The sheet with more vivid colours, shown inside the pointer's lens.
    pub vivid: Option<TextureHandle>,
    pub sprites: HashMap<u64, Sprite>,
    /// `(ascent, descent)` of the page font in em.
    pub metrics: Option<(f32, f32)>,
    pub failed_glyphs: Vec<u64>,
    /// Distinguishes this sheet from the one it replaced.
    pub version: u64,
}

/// A finished sheet on its way from the worker thread.
pub struct SheetResult {
    pub key: BlockKey,
    pub spec: Spec,
    pub image: ColorImage,
    pub vivid: Option<ColorImage>,
    pub sprites: HashMap<u64, Sprite>,
    pub metrics: Option<(f32, f32)>,
    pub failed_glyphs: Vec<u64>,
}

pub struct Block {
    pub key: BlockKey,
    pub page: Rc<PageBlock>,
    /// Number of word cells (a page still being loaded grows).
    pub cells: usize,
    /// Measured, or estimated until the page has been laid out. Includes `below`.
    pub height: f32,
    /// Height of the verse translations under the page (0 when they are off).
    pub below: f32,
    pub placed: Option<Placed>,
    pub sheet: Option<LoadedSheet>,
    /// The sheet being drawn on a worker thread, and the flag that calls it off.
    pub job: Option<(Spec, Arc<AtomicBool>)>,
    /// Width its widest line needs at scale 1, once known.
    pub natural: Option<f32>,
}

impl Block {
    pub fn new(key: BlockKey, page: Rc<PageBlock>, height: f32) -> Self {
        let cells = cell_count(&page);
        Self { key, page, cells, height, below: 0.0, placed: None, sheet: None, job: None, natural: None }
    }

    pub fn has_bismillah(&self) -> bool {
        self.page.items.iter().any(|i| matches!(i, Item::Bismillah { .. }))
    }

    /// Free what only a page near the viewport needs.
    pub fn release(&mut self) {
        self.placed = None;
        self.sheet = None;
        self.cancel_job();
    }

    /// Call off the sheet being drawn (if it hasn't started, it never will).
    pub fn cancel_job(&mut self) {
        if let Some((_, cancelled)) = self.job.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub fn drawing(&self, spec: &Spec) -> bool {
        self.job.as_ref().is_some_and(|(s, _)| s == spec)
    }

    pub fn accept(&mut self, ctx: &egui::Context, result: SheetResult, version: u64) {
        if !self.drawing(&result.spec) {
            return; // released or superseded meanwhile
        }
        self.job = None;
        let name = format!("page-{:?}-{}", self.key.0, self.key.1);
        let texture = ctx.load_texture(name.clone(), result.image, TextureOptions::LINEAR);
        let vivid = result.vivid.map(|image| ctx.load_texture(name + "-vivid", image, TextureOptions::LINEAR));
        self.sheet = Some(LoadedSheet {
            spec: result.spec,
            texture,
            vivid,
            sprites: result.sprites,
            metrics: result.metrics,
            failed_glyphs: result.failed_glyphs,
            version,
        });
    }
}

impl Drop for Block {
    /// A view that is closed or reloaded calls off what was being drawn for it.
    fn drop(&mut self) {
        self.cancel_job();
    }
}

pub fn cell_count(page: &PageBlock) -> usize {
    page.items
        .iter()
        .map(|i| match i {
            Item::Line { cells, .. } | Item::Flow { cells, .. } => cells.len(),
            _ => 0,
        })
        .sum()
}

/// Stable key: page number plus how many earlier blocks had the same page.
pub fn block_key(pages: &[Rc<PageBlock>], i: usize) -> BlockKey {
    let page = pages[i].page;
    (page, pages[..i].iter().filter(|b| b.page == page).count() as u32)
}

/// "مكية" / "مدنية" for the API's `revelation_place`.
fn place_arabic(place: &str) -> Option<&'static str> {
    match revelation_label(place) {
        "Meccan" => Some("مكية"),
        "Medinan" => Some("مدنية"),
        _ => None,
    }
}

/// Everything a page's sheet holds: its words, and the labels of its surah
/// banners and Bismillah lines. Font 0 is the page's, font 1 page 1's.
pub fn requests(page: &PageBlock, chapters: &[Chapter], bismillah: Option<&Verse>) -> Vec<Request> {
    let mut out = Vec::new();
    let label = |key, text: String, em| Request { key, glyph: None, text: Some(Text::plain(text, em)) };
    let ornament = |key, text: String, em, font| Request { key, glyph: None, text: Some(Text { text, em, font }) };
    for item in &page.items {
        match item {
            Item::Line { cells, .. } | Item::Flow { cells, .. } => out.extend(cells.iter().map(|c| Request {
                key: word_key(&c.location),
                glyph: c.glyph.clone().filter(|_| c.page.is_some() && c.page == page.page).map(|g| (0, g)),
                text: c.text.clone().filter(|t| !t.trim().is_empty()).map(|text| {
                    let em = match (c.qpc, c.is_end) {
                        (true, _) => QPC_EM,
                        (false, true) => NUMBER_EM,
                        (false, false) => UTHMANI_EM,
                    };
                    // An ayah number that is not QPC text is drawn plain, in a ring.
                    let font = if c.is_end && !c.qpc { TextFont::Number } else { TextFont::Fallback };
                    Text { text, em, font }
                }),
            })),
            Item::SurahHeader { chapter } => {
                // The printed-mushaf banner: a frame and the calligraphic name (drawn
                // only when the ornament fonts are there)...
                let key = |part| label_key("h", *chapter, part);
                out.push(ornament(label_key("h", 0, banner::FRAME), BANNER_FRAME_TEXT.into(), BANNER_FRAME_EM, TextFont::Frame));
                out.push(ornament(key(banner::ORNAMENT_NAME), banner_name_text(*chapter), BANNER_ORNAMENT_NAME_EM, TextFont::Names));
                // ...and its labels, which also make the plain banner.
                let Some(info) = find_chapter(chapters, *chapter) else { continue };
                out.push(label(key(banner::NAME), format!("سورة {}", info.name_arabic), BANNER_NAME_EM));
                if let Some(place) = place_arabic(&info.revelation_place) {
                    out.push(label(key(banner::PLACE), place.to_string(), BANNER_SIDE_EM));
                }
                out.push(label(key(banner::VERSES), "آياتها".into(), BANNER_SIDE_EM));
                let count = arabic_number(info.verses_count);
                out.push(ornament(key(banner::COUNT), count, BANNER_SIDE_EM, TextFont::Number));
            }
            Item::Bismillah { chapter } => {
                // Verse 1:1's four words in the page-1 font, all or none...
                let words = || bismillah.iter().flat_map(|v| &v.words).filter(|w| !w.is_end_marker());
                let glyphs: Vec<String> = words().filter_map(glyph::glyph_code).collect();
                if glyphs.len() == 4 && words().count() == 4 {
                    out.extend(glyphs.into_iter().enumerate().map(|(i, g)| Request {
                        key: label_key("b", *chapter, i),
                        glyph: Some((1, g)),
                        text: None,
                    }));
                }
                // ...else its QPC text without the trailing ayah number, else plain Unicode.
                let qpc = bismillah.and_then(|v| v.qpc_uthmani_hafs.as_deref()).and_then(|t| {
                    let t = t.trim_end().trim_end_matches(|c: char| c.is_numeric()).trim_end();
                    (!t.is_empty()).then(|| t.to_string())
                });
                let (text, em) = qpc.map_or((BISMILLAH.to_string(), UTHMANI_EM), |t| (t, QPC_EM));
                out.push(label(label_key("bt", *chapter, 0), text, em));
            }
        }
    }
    out
}

/// A hash of what [`requests`] depends on besides the words themselves.
pub fn content_stamp(block: &Block, chapters: &[Chapter], bismillah: Option<&Verse>) -> u64 {
    let headers = block.page.items.iter().any(|i| matches!(i, Item::SurahHeader { .. }));
    let mut stamp = block.cells as u64;
    if headers && !chapters.is_empty() {
        stamp |= 1 << 40;
    }
    if block.has_bismillah() && bismillah.is_some() {
        stamp |= 1 << 41;
    }
    stamp
}

pub struct JobInput {
    /// Set when the sheet is no longer wanted (its page scrolled away).
    pub cancelled: Arc<AtomicBool>,
    pub key: BlockKey,
    pub spec: Spec,
    pub requests: Vec<Request>,
    pub fonts: Vec<Option<Arc<dyn PageGlyphs>>>,
    pub fallback: Option<Arc<ArabicFont>>,
    pub ornaments: Option<Arc<Ornaments>>,
}

/// Draw a sheet (on a worker thread). Should drawing the glyphs ever die, the
/// page is drawn as text instead and every glyph is reported as failed.
/// `None` if the sheet was called off before its turn came.
pub fn draw(input: JobInput) -> Option<SheetResult> {
    if input.cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let mut job = sheet::Job {
        requests: input.requests,
        fonts: input.fonts,
        fallback: input.fallback,
        ornaments: input.ornaments,
        px_per_em: input.spec.px_per_em,
        palette: input.spec.palette,
    };
    let sheet = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sheet::build(&job))) {
        Ok(sheet) => sheet,
        Err(_) => {
            eprintln!("quran-eg: drawing page {:?} with its font failed; showing its text", input.key.0);
            job.fonts.clear();
            let mut sheet = sheet::build(&job);
            sheet.failed_glyphs = job.requests.iter().filter(|r| r.glyph.is_some()).map(|r| r.key).collect();
            sheet
        }
    };
    // The lens's copy, if the pointer glows and the page has colours of its own.
    let colourful = sheet.sprites.values().any(|s| s.kind == sheet::Kind::Color);
    let vivid = (input.spec.glow > 0 && colourful).then(|| {
        let boost = crate::pointer::boost(input.spec.glow);
        let pixels: Vec<u8> = sheet.rgba.as_chunks::<4>().0.iter().flat_map(|px| crate::pointer::boosted(*px, boost)).collect();
        ColorImage::from_rgba_premultiplied([sheet.width, sheet.height], &pixels)
    });
    Some(SheetResult {
        key: input.key,
        spec: input.spec,
        vivid,
        image: ColorImage::from_rgba_premultiplied([sheet.width, sheet.height], &sheet.rgba),
        sprites: sheet.sprites,
        metrics: sheet.metrics,
        failed_glyphs: sheet.failed_glyphs,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::verse;
    use super::*;
    use quran_core::mushaf_layout::{Trim, build_layout};
    use serde_json::json;

    fn chapter(id: u16) -> Chapter {
        serde_json::from_value(json!({
            "id": id, "revelation_place": "madinah", "revelation_order": 1, "bismillah_pre": true,
            "name_simple": "Al-Baqarah", "name_complex": "Al-Baqarah", "name_arabic": "البقرة",
            "verses_count": 286, "translated_name": { "name": "The Cow" }
        }))
        .unwrap()
    }

    #[test]
    fn a_sheet_asks_for_every_word_and_label_of_its_page() {
        // 111:5 ends page 603; 112:1 (4 words) and 112:2 (2 words) are on page 604.
        let layout = build_layout(&[verse("111:5", 603, 15), verse("112:1", 604, 2), verse("112:2", 604, 3)], Trim::default());
        assert_eq!(layout.pages.len(), 2);
        assert_eq!(block_key(&layout.pages, 1), (Some(604), 0));
        let page = &layout.pages[1];
        assert_eq!(cell_count(page), 8);

        let all = requests(page, &[chapter(112)], None);
        // The banner's frame and name ornaments and its four labels; the Bismillah's text; eight cells.
        assert_eq!(all.len(), 15);
        let words: Vec<&Request> = all.iter().filter(|r| r.glyph.is_some()).collect();
        assert_eq!(words.len(), 8);
        assert!(words.iter().all(|r| r.glyph.as_ref().unwrap().0 == 0 && r.text.is_some()));
        assert_eq!(words[0].key, word_key("112:1:1"));
        let texts: Vec<(&str, TextFont)> =
            all.iter().filter(|r| r.glyph.is_none()).filter_map(|r| r.text.as_ref()).map(|t| (t.text.as_str(), t.font)).collect();
        assert_eq!(
            texts,
            [
                ("header", TextFont::Frame),
                ("surah112 surah-icon", TextFont::Names),
                ("سورة البقرة", TextFont::Fallback),
                ("مدنية", TextFont::Fallback),
                ("آياتها", TextFont::Fallback),
                // The count is a plain number, not the text face's ayah medallion.
                ("٢٨٦", TextFont::Number),
                (BISMILLAH, TextFont::Fallback),
            ]
        );
        // Keys are unique: no word can take another's sprite.
        let mut keys: Vec<u64> = all.iter().map(|r| r.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), all.len());

        // Without the chapter list the banner has no labels yet; the stamp changes when it arrives.
        assert_eq!(requests(page, &[], None).len(), 11);
        let block = Block::new((Some(604), 0), page.clone(), 100.0);
        assert_ne!(content_stamp(&block, &[], None), content_stamp(&block, &[chapter(112)], None));
    }

    #[test]
    fn bismillah_uses_verse_one_glyphs_when_it_has_all_four() {
        let layout = build_layout(&[verse("112:1", 604, 2)], Trim::default());
        let mut first = verse("1:1", 1, 2);
        first.qpc_uthmani_hafs = Some("بِسۡمِ ٱللَّهِ ٱلرَّحۡمَٰنِ ٱلرَّحِيمِ ١".into());
        let all = requests(&layout.pages[0], &[], Some(&first));
        let glyphs: Vec<&Request> = all.iter().filter(|r| r.glyph.as_ref().is_some_and(|g| g.0 == 1)).collect();
        assert_eq!(glyphs.len(), 4);
        let text = all.iter().find(|r| r.key == label_key("bt", 112, 0)).unwrap();
        assert_eq!(text.text.as_ref().map(|t| t.text.as_str()), Some("بِسۡمِ ٱللَّهِ ٱلرَّحۡمَٰنِ ٱلرَّحِيمِ"), "the ayah number is dropped");
    }
}
