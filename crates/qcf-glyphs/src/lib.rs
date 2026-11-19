//! Rasterises the per-page QCF mushaf fonts (quran.com's `p{N}.woff2` files)
//! into RGBA images, one glyph — one word of the page — at a time.
//!
//! Supported font sets:
//! - **V2**: plain outlines, drawn as coverage in a caller-chosen colour.
//! - **V4 tajweed** (`hafs/v4/colrv1`): `COLR`/`CPAL` colour glyphs, drawn with
//!   the font's light or dark palette. The files quran.com serves are COLR
//!   version 0 (flat coloured layers); COLRv1 paint graphs are drawn too as long
//!   as they use solid fills and source-over compositing. A glyph that needs
//!   gradients or other blend modes is refused (`None`) rather than drawn wrong.
//!
//! The OpenType-SVG variants (`hafs/v4/ot-svg`) are not interpreted: their
//! `SVG ` table is ignored and they draw as single-colour outlines
//! ([`PageFont::is_color`] is `false` for them).
//!
//! Glyphs are drawn unhinted, with the pen on a whole pixel. Place images at
//! whole-pixel positions (round the pen position, then add [`GlyphImage::left`]
//! and subtract [`GlyphImage::top`]) to keep them sharp.

mod raster;
mod scene;
#[cfg(test)]
mod tests;

use std::fmt;

use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};

/// Largest accepted `px_per_em`.
const MAX_PX_PER_EM: f32 = 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The bytes look like WOFF2 but could not be decompressed.
    Woff2,
    /// Not a usable TrueType/OpenType font.
    InvalidFont,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Woff2 => "WOFF2 data could not be decompressed",
            Error::InvalidFont => "not a usable TrueType/OpenType font",
        })
    }
}

impl std::error::Error for Error {}

/// Which of a colour font's palettes to draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Palette {
    /// `CPAL` palette 0: black text, tajweed colours for a light background.
    Light,
    /// `CPAL` palette 1: white text, tajweed colours for a dark background.
    /// Falls back to palette 0 if the font has only one.
    Dark,
    /// A palette by index; glyphs are not drawn if the font lacks it. The QCF
    /// V4 fonts have six: 0 light, 1 dark, 2 sepia, and 3–5 the same three
    /// with the tajweed colours off (only the ayah markers keep colour).
    Index(u16),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlyphImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8, row-major, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Horizontal advance in pixels at the requested size.
    pub advance: f32,
    /// Offset of the image's left edge from the pen position, in pixels.
    pub left: f32,
    /// Offset of the image's top edge above the baseline, in pixels (positive = above).
    pub top: f32,
}

/// One page's font, parsed once. Cheap to keep; holds the decompressed font bytes.
///
/// `Send + Sync`: glyphs of one font can be rasterised from several threads.
pub struct PageFont {
    /// The font as an sfnt (TTF/OTF) file.
    data: Vec<u8>,
    units_per_em: f32,
    ascent: f32,
    descent: f32,
    color: bool,
    palettes: u16,
}

impl PageFont {
    /// Accepts WOFF2 or plain TTF/OTF bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let data = if bytes.starts_with(b"wOF2") {
            wuff::decompress_woff2(bytes).map_err(|_| Error::Woff2)?
        } else {
            bytes.to_vec()
        };
        let font = FontRef::new(&data).map_err(|_| Error::InvalidFont)?;
        let units_per_em = font.head().map_err(|_| Error::InvalidFont)?.units_per_em();
        if units_per_em == 0 || font.maxp().is_err() || font.cmap().is_err() {
            return Err(Error::InvalidFont);
        }
        let units_per_em = f32::from(units_per_em);
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let (ascent, descent) = (
            metrics.ascent / units_per_em,
            -metrics.descent / units_per_em,
        );
        if !ascent.is_finite() || !descent.is_finite() {
            return Err(Error::InvalidFont);
        }
        let color = font.colr().is_ok();
        let palettes = font.color_palettes().len();
        Ok(PageFont {
            data,
            units_per_em,
            ascent,
            descent,
            color,
            palettes,
        })
    }

    /// Whether the font has a real (non-.notdef, non-empty) glyph for `ch`.
    pub fn has_glyph(&self, ch: char) -> bool {
        let Some((font, glyph)) = self.lookup(ch) else {
            return false;
        };
        let outlines = font.outline_glyphs();
        match font.color_glyphs().get(glyph) {
            Some(color_glyph) => scene::color_has_ink(&outlines, &color_glyph),
            None => scene::outline_has_ink(&outlines, glyph),
        }
    }

    /// Advance of `ch` in em units (so layout can be done before rasterising).
    /// `None` if the font does not map `ch`.
    pub fn advance_em(&self, ch: char) -> Option<f32> {
        let (font, glyph) = self.lookup(ch)?;
        self.advance_units(&font, glyph)
            .map(|a| a / self.units_per_em)
    }

    /// Font-wide ascent and descent in em units, both as positive distances
    /// from the baseline (up and down).
    pub fn metrics_em(&self) -> (f32, f32) {
        (self.ascent, self.descent)
    }

    /// Whether the font has colour glyphs (`COLR`), i.e. is a V4 tajweed font
    /// this crate draws in colour.
    pub fn is_color(&self) -> bool {
        self.color
    }

    /// Number of colour palettes (0 for single-colour fonts).
    pub fn palette_count(&self) -> u16 {
        self.palettes
    }

    /// Every character the font maps to a glyph, in codepoint order. Includes
    /// blank ones such as the space (see [`Self::has_glyph`]).
    pub fn chars(&self) -> Vec<char> {
        let Ok(font) = FontRef::new(&self.data) else {
            return Vec::new();
        };
        let mut chars: Vec<char> = font
            .charmap()
            .mappings()
            .filter(|(_, glyph)| *glyph != GlyphId::NOTDEF)
            .filter_map(|(code, _)| char::from_u32(code))
            .collect();
        chars.sort_unstable();
        chars.dedup();
        chars
    }

    /// Rasterise one glyph at `px_per_em`. For colour fonts `palette` picks the
    /// colours; for single-colour fonts the result is coverage in `mono` colour.
    ///
    /// `mono` is straight (not premultiplied) sRGB RGBA. Colour fonts use it
    /// only where a glyph asks for "the text colour", which the QCF fonts never
    /// do: their text colour is the palette's (black or white).
    ///
    /// A blank glyph (the space) gives a 0×0 image with its advance. `None` if
    /// `ch` is not in the font, the size is not a positive number up to 1024,
    /// or the glyph cannot be drawn correctly (broken data, a missing palette,
    /// unsupported paints, an image over 8192 px a side).
    pub fn rasterize(
        &self,
        ch: char,
        px_per_em: f32,
        palette: Palette,
        mono: [u8; 4],
    ) -> Option<GlyphImage> {
        if !(px_per_em > 0.0 && px_per_em <= MAX_PX_PER_EM) {
            return None;
        }
        let (font, glyph) = self.lookup(ch)?;
        let scale = px_per_em / self.units_per_em;
        let advance = self.advance_units(&font, glyph)? / self.units_per_em * px_per_em;

        let outlines = font.outline_glyphs();
        let scene = match font.color_glyphs().get(glyph) {
            Some(color_glyph) => {
                let palettes = font.color_palettes();
                let palette = palettes.get(self.palette_index(palette)?);
                let colors = palette.as_ref().map_or(&[][..], |p| p.colors());
                scene::colored(&outlines, &color_glyph, colors, mono)?
            }
            None => scene::plain(&outlines, glyph, mono)?,
        };
        let image = raster::render(&scene, scale)?;
        Some(GlyphImage {
            width: image.width,
            height: image.height,
            rgba: image.rgba,
            advance,
            left: image.left as f32,
            top: image.top as f32,
        })
    }

    fn lookup(&self, ch: char) -> Option<(FontRef<'_>, GlyphId)> {
        let font = FontRef::new(&self.data).ok()?;
        let glyph = font.charmap().map(ch).filter(|g| *g != GlyphId::NOTDEF)?;
        Some((font, glyph))
    }

    fn advance_units(&self, font: &FontRef<'_>, glyph: GlyphId) -> Option<f32> {
        let advance = font
            .glyph_metrics(Size::unscaled(), LocationRef::default())
            .advance_width(glyph)?;
        advance.is_finite().then_some(advance)
    }

    fn palette_index(&self, palette: Palette) -> Option<u16> {
        match palette {
            Palette::Light => Some(0),
            Palette::Dark => Some(if self.palettes > 1 { 1 } else { 0 }),
            Palette::Index(index) => (index < self.palettes).then_some(index),
        }
    }
}
