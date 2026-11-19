//! A page's sprite sheet: every word of a mushaf page (QCF glyphs, or shaped
//! fallback text) rasterised into one image, so a page is one texture and one
//! mesh. Built off the UI thread; the reader uploads it and frees it when the
//! page scrolls away.

use std::collections::HashMap;
use std::sync::Arc;

use crate::arabic::{ArabicFont, Ornaments};
use crate::glyphs::{PageGlyphs, Palette};

/// Gap between sprites in the sheet, so linear filtering never bleeds a neighbour in.
const PAD: u32 = 2;
/// Sheets are packed in rows no wider than this.
const MAX_WIDTH: u32 = 2048;

/// What to draw for one word (or label): its glyph in one of the job's fonts,
/// else its text in the fallback font, else nothing (the reader then draws a placeholder).
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub key: u64,
    /// Index into [`Job::fonts`] and the glyph character(s).
    pub glyph: Option<(usize, String)>,
    pub text: Option<Text>,
}

/// Text drawn by shaping instead of from a page font.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub text: String,
    /// Font size relative to the glyph em.
    pub em: f32,
    pub font: TextFont,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFont {
    /// The fallback text face.
    Fallback,
    /// The same face, as a plain number.
    Number,
    /// The surah banner's frame and name ornaments (ligatures).
    Frame,
    Names,
}

impl Text {
    pub fn plain(text: impl Into<String>, em: f32) -> Self {
        Self { text: text.into(), em, font: TextFont::Fallback }
    }
}

pub struct Job {
    pub requests: Vec<Request>,
    pub fonts: Vec<Option<Arc<dyn PageGlyphs>>>,
    pub fallback: Option<Arc<ArabicFont>>,
    pub ornaments: Option<Arc<Ornaments>>,
    /// Device pixels per glyph em.
    pub px_per_em: f32,
    pub palette: Palette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A glyph in a single-colour font or shaped text: coverage, tinted when drawn.
    Mask,
    /// A glyph with its own colours.
    Color,
}

/// One image in the sheet. Lengths are in glyph em.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    /// `[x0, y0, x1, y1]` in the sheet, 0..1.
    pub uv: [f32; 4],
    pub size: [f32; 2],
    /// Left edge from the pen position.
    pub left: f32,
    /// Top edge above the baseline.
    pub top: f32,
    pub advance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sprite {
    pub kind: Kind,
    /// Drawn as a QCF glyph (not fallback text).
    pub glyph: bool,
    /// In reading order: the first part is the rightmost.
    pub parts: Vec<Part>,
    /// Total advance in glyph em.
    pub advance: f32,
}

pub struct Sheet {
    pub width: usize,
    pub height: usize,
    /// Premultiplied RGBA8.
    pub rgba: Vec<u8>,
    pub sprites: HashMap<u64, Sprite>,
    /// Requests whose glyph the font should have drawn but could not: shown as
    /// text (or a placeholder) instead, and reported.
    pub failed_glyphs: Vec<u64>,
    /// `(ascent, descent)` of the first font in em, when it was there.
    pub metrics: Option<(f32, f32)>,
}

struct Bitmap {
    width: u32,
    height: u32,
    /// Premultiplied RGBA8.
    rgba: Vec<u8>,
    left: f32,
    top: f32,
    advance: f32,
}

struct Pending {
    key: u64,
    kind: Kind,
    glyph: bool,
    bitmaps: Vec<Bitmap>,
}

/// Draw one request: its sprite if anything could be drawn, and whether its
/// glyph failed.
fn draw(job: &Job, request: &Request) -> (Option<Pending>, bool) {
    let mut failed = false;
    let glyph = request.glyph.as_ref().and_then(|(font, code)| {
        let font = job.fonts.get(*font)?.as_ref()?;
        let drawn = glyph_bitmaps(font.as_ref(), code, job);
        failed = drawn.is_none();
        let kind = if font.is_color() { Kind::Color } else { Kind::Mask };
        Some(Pending { key: request.key, kind, glyph: true, bitmaps: drawn? })
    });
    let text = || {
        let text = request.text.as_ref()?;
        let px = job.px_per_em * text.em;
        let mask = match text.font {
            TextFont::Fallback => job.fallback.as_ref()?.render(&text.text, px),
            TextFont::Number => job.fallback.as_ref()?.render_number(&text.text, px),
            TextFont::Frame => job.ornaments.as_ref()?.frame.render_ligatures(&text.text, px),
            TextFont::Names => job.ornaments.as_ref()?.names.render_ligatures(&text.text, px),
        }?;
        let rgba = mask.alpha.iter().flat_map(|a| [*a; 4]).collect();
        let bitmap = Bitmap { width: mask.width, height: mask.height, rgba, left: mask.left, top: mask.top, advance: mask.advance };
        Some(Pending { key: request.key, kind: Kind::Mask, glyph: false, bitmaps: vec![bitmap] })
    };
    (glyph.or_else(text), failed)
}

/// A sheet being built, a few requests at a time: so that in a browser, where
/// this runs between frames, no frame waits for a whole page.
pub struct Builder {
    job: Job,
    next: usize,
    pending: Vec<Pending>,
    failed_glyphs: Vec<u64>,
}

impl Builder {
    pub fn new(job: Job) -> Self {
        let pending = Vec::with_capacity(job.requests.len());
        Self { job, next: 0, pending, failed_glyphs: Vec::new() }
    }

    /// Draw requests until the deadline; the sheet once all are drawn.
    pub fn step(&mut self, deadline: &crate::task::Deadline) -> Option<Sheet> {
        while let Some(request) = self.job.requests.get(self.next) {
            let (sprite, failed) = draw(&self.job, request);
            self.pending.extend(sprite);
            if failed {
                self.failed_glyphs.push(request.key);
            }
            self.next += 1;
            if deadline.expired() {
                return None;
            }
        }
        let metrics = self.job.fonts.first().and_then(|f| f.as_ref()).map(|f| f.metrics_em());
        let (pending, failed) = (std::mem::take(&mut self.pending), std::mem::take(&mut self.failed_glyphs));
        Some(pack(pending, self.job.px_per_em, failed, metrics))
    }
}

impl Builder {
    /// Give the job back, without its page fonts (to draw it as text after all).
    pub fn into_plain_job(mut self) -> Job {
        self.job.fonts.clear();
        self.job
    }
}

/// Build a whole sheet in one go.
pub fn build(job: Job) -> Sheet {
    let mut builder = Builder::new(job);
    loop {
        if let Some(sheet) = builder.step(&crate::task::Deadline::none()) {
            return sheet;
        }
    }
}

/// Every character of `code` must draw: a word is never shown half.
///
/// A character may draw nothing if it also takes no room: in the tajweed fonts
/// some words have a second glyph that is blank and has no advance (10:1:1 is
/// U+FC41 U+FC42, and U+FC42 is such a glyph). The word as a whole must leave ink.
fn glyph_bitmaps(font: &dyn PageGlyphs, code: &str, job: &Job) -> Option<Vec<Bitmap>> {
    let mut any_ink = false;
    let bitmaps: Option<Vec<Bitmap>> = code
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|ch| {
            // `None`: the font has no such character, or cannot draw it.
            let image = font.rasterize(ch, job.px_per_em, job.palette, [255; 4])?;
            let ink = image.rgba.as_chunks::<4>().0.iter().any(|px| px[3] != 0);
            if !ink && image.advance != 0.0 {
                return None; // room for a glyph, and nothing in it
            }
            any_ink |= ink;
            Some(Bitmap {
                width: image.width,
                height: image.height,
                rgba: image.rgba,
                left: image.left,
                top: image.top,
                advance: image.advance,
            })
        })
        .collect();
    bitmaps.filter(|b| !b.is_empty() && any_ink)
}

/// Shelf packing: sprites in rows, tallest first.
fn pack(mut pending: Vec<Pending>, px_per_em: f32, failed_glyphs: Vec<u64>, metrics: Option<(f32, f32)>) -> Sheet {
    let mut order: Vec<(usize, usize)> = pending
        .iter()
        .enumerate()
        .flat_map(|(i, p)| (0..p.bitmaps.len()).map(move |j| (i, j)))
        .filter(|&(i, j)| pending[i].bitmaps[j].width > 0 && pending[i].bitmaps[j].height > 0)
        .collect();
    order.sort_by_key(|&(i, j)| std::cmp::Reverse(pending[i].bitmaps[j].height));
    let area: u64 = order
        .iter()
        .map(|&(i, j)| u64::from(pending[i].bitmaps[j].width + PAD) * u64::from(pending[i].bitmaps[j].height + PAD))
        .sum();
    let widest = order.iter().map(|&(i, j)| pending[i].bitmaps[j].width).max().unwrap_or(0) + PAD;
    let width = (((area as f64).sqrt() * 1.2) as u32).clamp(64, MAX_WIDTH).max(widest);

    let mut places: HashMap<(usize, usize), (u32, u32)> = HashMap::with_capacity(order.len());
    let (mut x, mut y, mut row_height) = (PAD, PAD, 0u32);
    for &(i, j) in &order {
        let b = &pending[i].bitmaps[j];
        if x + b.width + PAD > width && x > PAD {
            x = PAD;
            y += row_height + PAD;
            row_height = 0;
        }
        places.insert((i, j), (x, y));
        x += b.width + PAD;
        row_height = row_height.max(b.height);
    }
    let height = y + row_height + PAD;

    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let mut sprites = HashMap::with_capacity(pending.len());
    for (i, p) in pending.iter_mut().enumerate() {
        let mut parts = Vec::with_capacity(p.bitmaps.len());
        let mut advance = 0.0;
        for (j, b) in p.bitmaps.iter().enumerate() {
            let (px, py) = places.get(&(i, j)).copied().unwrap_or((0, 0));
            if places.contains_key(&(i, j)) {
                let row_bytes = b.width as usize * 4;
                for row in 0..b.height as usize {
                    let src = row * row_bytes;
                    let dst = ((py as usize + row) * w + px as usize) * 4;
                    rgba[dst..dst + row_bytes].copy_from_slice(&b.rgba[src..src + row_bytes]);
                }
            }
            parts.push(Part {
                uv: [
                    px as f32 / width as f32,
                    py as f32 / height as f32,
                    (px + b.width) as f32 / width as f32,
                    (py + b.height) as f32 / height as f32,
                ],
                size: [b.width as f32 / px_per_em, b.height as f32 / px_per_em],
                left: b.left / px_per_em,
                top: b.top / px_per_em,
                advance: b.advance / px_per_em,
            });
            advance += b.advance / px_per_em;
        }
        p.bitmaps = Vec::new();
        sprites.insert(p.key, Sprite { kind: p.kind, glyph: p.glyph, parts, advance });
    }
    Sheet { width: w, height: h, rgba, sprites, failed_glyphs, metrics }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(width: u32, height: u32, value: u8) -> Bitmap {
        Bitmap { width, height, rgba: vec![value; (width * height * 4) as usize], left: 1.0, top: 8.0, advance: 10.0 }
    }

    #[test]
    fn packs_without_overlap_and_keeps_every_sprite() {
        let pending: Vec<Pending> = (0..40u64)
            .map(|key| Pending {
                key,
                kind: Kind::Mask,
                glyph: true,
                bitmaps: vec![bitmap(10 + (key as u32 % 7) * 9, 12 + (key as u32 % 5) * 6, key as u8 + 1)],
            })
            .collect();
        let sheet = pack(pending, 20.0, vec![], None);
        assert_eq!(sheet.sprites.len(), 40);
        assert_eq!(sheet.rgba.len(), sheet.width * sheet.height * 4);
        for (key, sprite) in &sheet.sprites {
            let part = sprite.parts[0];
            let (x0, y0) = ((part.uv[0] * sheet.width as f32).round() as usize, (part.uv[1] * sheet.height as f32).round() as usize);
            let (x1, y1) = ((part.uv[2] * sheet.width as f32).round() as usize, (part.uv[3] * sheet.height as f32).round() as usize);
            // Its own pixels are all there (so nothing else was written over them)...
            for y in y0..y1 {
                for x in x0..x1 {
                    assert_eq!(sheet.rgba[(y * sheet.width + x) * 4], *key as u8 + 1, "sprite {key} at {x},{y}");
                }
            }
            // ...with an empty border around them.
            assert_eq!(sheet.rgba[(y0 * sheet.width + x0 - 1) * 4], 0);
            assert_eq!(sheet.rgba[((y0 - 1) * sheet.width + x0) * 4], 0);
            assert!((part.size[0] - (x1 - x0) as f32 / 20.0).abs() < 1e-4);
            assert!((sprite.advance - 0.5).abs() < 1e-6 && (part.top - 0.4).abs() < 1e-6);
        }
    }

    #[test]
    fn blank_and_multi_part_sprites() {
        let pending = vec![Pending {
            key: 7,
            kind: Kind::Color,
            glyph: true,
            bitmaps: vec![bitmap(4, 4, 9), bitmap(0, 0, 0), bitmap(6, 3, 5)],
        }];
        let sheet = pack(pending, 10.0, vec![3], Some((1.0, 0.5)));
        let sprite = &sheet.sprites[&7];
        assert_eq!((sprite.parts.len(), sprite.kind), (3, Kind::Color));
        assert!((sprite.advance - 3.0).abs() < 1e-6);
        assert_eq!(sprite.parts[1].size, [0.0, 0.0]);
        assert_eq!((sheet.failed_glyphs.as_slice(), sheet.metrics), (&[3u64][..], Some((1.0, 0.5))));
    }

    /// Draws 'a' as a 2×2 square and 'm' as a mark with ink but no advance;
    /// cannot draw 'b'; 'c' is blank but takes room; 'z' is blank and takes none.
    struct FakeFont;

    impl PageGlyphs for FakeFont {
        fn metrics_em(&self) -> (f32, f32) {
            (1.0, 0.5)
        }
        fn is_color(&self) -> bool {
            false
        }
        fn rasterize(&self, ch: char, px_per_em: f32, _: Palette, mono: [u8; 4]) -> Option<crate::glyphs::GlyphImage> {
            let image = |ink: u8, advance: f32| {
                Some(crate::glyphs::GlyphImage { width: 2, height: 2, rgba: vec![ink; 16], advance, left: 0.0, top: 2.0 })
            };
            match ch {
                'a' => image(mono[3], px_per_em / 2.0),
                'm' => image(mono[3], 0.0),
                'c' => image(0, px_per_em / 2.0),
                'z' => Some(crate::glyphs::GlyphImage { width: 0, height: 0, rgba: Vec::new(), advance: 0.0, left: 0.0, top: 0.0 }),
                _ => None,
            }
        }
    }

    #[test]
    fn a_word_is_a_glyph_only_if_all_of_it_draws() {
        let request = |key, code: &str| Request { key, glyph: Some((0, code.into())), text: Some(Text::plain("نص", 0.8)) };
        let codes = ["a", "b", "c", "ab", "aa", "x", "az", "z", "m"];
        let job = Job {
            requests: codes.iter().enumerate().map(|(i, code)| request(i as u64 + 1, code)).collect(),
            fonts: vec![Some(Arc::new(FakeFont))],
            fallback: None,
            ornaments: None,
            px_per_em: 40.0,
            palette: Palette::Light,
        };
        let sheet = build(job);
        let mut drawn: Vec<u64> = sheet.sprites.keys().copied().collect();
        drawn.sort_unstable();
        // Drawn: "a"; "aa" (two parts, right to left by their advances); "az", whose second
        // glyph is blank and takes no room (as 10:1:1's in the tajweed fonts); and "m", a
        // mark with ink and no advance (as 40:77:5's pause mark).
        assert_eq!(drawn, [1, 5, 7, 9]);
        assert!(sheet.sprites[&1].glyph && sheet.sprites[&5].parts.len() == 2);
        assert!((sheet.sprites[&5].advance - 1.0).abs() < 1e-6);
        let blank = &sheet.sprites[&7];
        assert_eq!((blank.parts.len(), blank.parts[1].size, blank.parts[1].advance), (2, [0.0, 0.0], 0.0));
        assert!((blank.advance - 0.5).abs() < 1e-6, "the blank glyph adds no width");
        assert_eq!((sheet.sprites[&9].advance, sheet.sprites[&9].parts[0].size), (0.0, [0.05, 0.05]));
        // Not drawn — undrawable, blank where there is room for a glyph, half drawable, not in
        // the font, nothing but a blank — and reported, so the reader shows their text (or a
        // placeholder) with a mark.
        assert_eq!(sheet.failed_glyphs, [2, 3, 4, 6, 8]);
        assert_eq!(sheet.metrics, Some((1.0, 0.5)));
    }

    #[test]
    fn without_fonts_nothing_is_drawn_and_nothing_is_lost() {
        let job = Job {
            requests: vec![Request { key: 1, glyph: Some((0, "x".into())), text: Some(Text::plain("نص", 0.8)) }],
            fonts: vec![None],
            fallback: None,
            ornaments: None,
            px_per_em: 40.0,
            palette: Palette::Light,
        };
        let sheet = build(job);
        // No sprite: the reader draws its placeholder for the word.
        assert!(sheet.sprites.is_empty() && sheet.failed_glyphs.is_empty());
        assert!(sheet.width > 0 && sheet.height > 0);
    }
}
