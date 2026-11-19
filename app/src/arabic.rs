//! Arabic text outside the page fonts — fallback words, surah names — shaped
//! with HarfRust and drawn as coverage masks. egui lays text out left to right
//! only, so such text never goes through it.

use harfrust::{FontRef, ShaperData, UnicodeBuffer};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use zeno::{Command, Mask, Vector};

/// A line of shaped text as an 8-bit coverage mask.
pub struct TextMask {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
    /// Advance of the whole line, in pixels.
    pub advance: f32,
    /// Left edge of the mask from the pen position, in pixels.
    pub left: f32,
    /// Top edge of the mask above the baseline, in pixels.
    pub top: f32,
}

/// The two ornament fonts of the printed-mushaf surah banner (QUL): the frame
/// (`header` draws it) and the calligraphic surah names (`surah001`…).
pub struct Ornaments {
    pub frame: ArabicFont,
    pub names: ArabicFont,
}

/// The ligature text that draws the banner's frame.
pub const BANNER_FRAME_TEXT: &str = "header";

/// The ligature text for "سورة <name>" (`surah-icon` draws "سورة").
pub fn banner_name_text(chapter: u16) -> String {
    format!("surah{chapter:03} surah-icon")
}

impl Ornaments {
    /// The fonts, if they draw what the banner needs: the frame as one wide
    /// drawing, and a name.
    pub fn verified(frame: ArabicFont, names: ArabicFont) -> Result<Self, String> {
        let wide = frame.render_ligatures(BANNER_FRAME_TEXT, 64.0).is_some_and(|m| m.width > 4 * m.height);
        let name = [1, 114].iter().all(|c| names.render_ligatures(&banner_name_text(*c), 64.0).is_some());
        if wide && name { Ok(Self { frame, names }) } else { Err("the banner fonts do not draw the banner".into()) }
    }
}

/// A font for shaped text (the KFGQPC Hafs face), parsed once.
pub struct ArabicFont {
    data: Vec<u8>,
    shaper: ShaperData,
    units_per_em: f32,
}

impl ArabicFont {
    /// Accepts WOFF2 or plain TTF/OTF bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let data = if bytes.starts_with(b"wOF2") {
            wuff::decompress_woff2(bytes).map_err(|e| format!("WOFF2 data could not be decompressed: {e:?}"))?
        } else {
            bytes.to_vec()
        };
        let font = FontRef::new(&data).map_err(|e| format!("not a usable font: {e}"))?;
        let units_per_em = font.metrics(Size::unscaled(), LocationRef::default()).units_per_em;
        if units_per_em == 0 {
            return Err("not a usable font: no units per em".into());
        }
        let shaper = ShaperData::new(&font);
        Ok(Self { data, shaper, units_per_em: f32::from(units_per_em) })
    }

    /// Shape `text` (one line; direction and script are taken from its
    /// characters) and draw it at `px_per_em`. `None` if nothing was drawn.
    pub fn render(&self, text: &str, px_per_em: f32) -> Option<TextMask> {
        self.draw(text, px_per_em, &[])
    }

    /// The same with discretionary ligatures on: how the ornament fonts draw a
    /// whole frame or a surah's name from a word such as `header` or `surah001`.
    pub fn render_ligatures(&self, text: &str, px_per_em: f32) -> Option<TextMask> {
        let on = |tag: &[u8; 4]| harfrust::Feature::new(harfrust::Tag::new(tag), 1, ..);
        self.draw(text, px_per_em, &[on(b"liga"), on(b"dlig")])
    }

    /// A number in plain digits. (The Hafs face turns a run of Arabic-Indic digits
    /// into an ayah medallion through its required ligatures: those are left
    /// out here, and the digits are set left to right, as numbers are.)
    pub fn render_number(&self, digits: &str, px_per_em: f32) -> Option<TextMask> {
        let off = harfrust::Feature::new(harfrust::Tag::new(b"rlig"), 0, ..);
        self.shape_and_draw(digits, px_per_em, &[off], Some(harfrust::Direction::LeftToRight))
    }

    fn draw(&self, text: &str, px_per_em: f32, features: &[harfrust::Feature]) -> Option<TextMask> {
        self.shape_and_draw(text, px_per_em, features, None)
    }

    fn shape_and_draw(
        &self,
        text: &str,
        px_per_em: f32,
        features: &[harfrust::Feature],
        direction: Option<harfrust::Direction>,
    ) -> Option<TextMask> {
        let font = FontRef::new(&self.data).ok()?;
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        if let Some(direction) = direction {
            buffer.set_direction(direction);
        }
        let shaped = self.shaper.shaper(&font).build().shape(buffer, harfrust::ShapeOptions::new().features(features));

        let scale = px_per_em / self.units_per_em;
        let outlines = font.outline_glyphs();
        let settings = || DrawSettings::unhinted(Size::new(px_per_em), LocationRef::default());
        let mut pen = PathPen { commands: Vec::new(), origin: Vector::ZERO };
        let mut x = 0.0f32;
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            pen.origin = Vector::new(x + pos.x_offset as f32 * scale, -(pos.y_offset as f32) * scale);
            if let Some(glyph) = outlines.get(skrifa::GlyphId::new(info.glyph_id)) {
                let _ = glyph.draw(settings(), &mut pen);
            }
            x += pos.x_advance as f32 * scale;
        }
        if pen.commands.is_empty() {
            return None;
        }
        let (alpha, placement) = Mask::new(pen.commands.as_slice()).render();
        if placement.width == 0 || placement.height == 0 {
            return None;
        }
        Some(TextMask {
            width: placement.width,
            height: placement.height,
            alpha,
            advance: x,
            left: placement.left as f32,
            top: -(placement.top as f32),
        })
    }
}

/// Collects glyph outlines into one path, y pointing down.
struct PathPen {
    commands: Vec<Command>,
    origin: Vector,
}

impl PathPen {
    fn point(&self, x: f32, y: f32) -> Vector {
        Vector::new(self.origin.x + x, self.origin.y - y)
    }
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(Command::MoveTo(self.point(x, y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.commands.push(Command::LineTo(self.point(x, y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.commands.push(Command::QuadTo(self.point(cx0, cy0), self.point(x, y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.commands.push(Command::CurveTo(self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y)));
    }
    fn close(&mut self) {
        self.commands.push(Command::Close);
    }
}

