//! The seam to the glyph rasteriser: what the reader needs from one page's QCF
//! font. Implemented by the `qcf-glyphs` crate's `PageFont`.

pub use qcf_glyphs::{GlyphImage, Palette};

pub trait PageGlyphs: Send + Sync {
    /// `(ascent, descent)` in em.
    fn metrics_em(&self) -> (f32, f32);
    /// The font carries its own colours (tajweed); otherwise glyphs are coverage in `mono`.
    fn is_color(&self) -> bool;
    /// The glyph of `ch`. `None` if the font has no such character or cannot
    /// draw it; a blank glyph is an empty image (with its advance).
    fn rasterize(&self, ch: char, px_per_em: f32, palette: Palette, mono: [u8; 4]) -> Option<GlyphImage>;
}

impl PageGlyphs for qcf_glyphs::PageFont {
    fn metrics_em(&self) -> (f32, f32) {
        qcf_glyphs::PageFont::metrics_em(self)
    }
    fn is_color(&self) -> bool {
        qcf_glyphs::PageFont::is_color(self)
    }
    fn rasterize(&self, ch: char, px_per_em: f32, palette: Palette, mono: [u8; 4]) -> Option<GlyphImage> {
        qcf_glyphs::PageFont::rasterize(self, ch, px_per_em, palette, mono)
    }
}

/// Parse a page font file (WOFF2 or TTF/OTF).
pub fn parse_page_font(bytes: &[u8]) -> Result<std::sync::Arc<dyn PageGlyphs>, String> {
    qcf_glyphs::PageFont::from_bytes(bytes)
        .map(|font| std::sync::Arc::new(font) as std::sync::Arc<dyn PageGlyphs>)
        .map_err(|e| e.to_string())
}
