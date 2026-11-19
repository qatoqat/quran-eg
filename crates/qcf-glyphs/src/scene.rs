//! Turns a glyph into a flat list of drawing operations in font units.
//!
//! Colour glyphs are walked with skrifa's `ColorPainter` callbacks (COLRv0
//! layers and COLRv1 paint graphs alike); plain glyphs become one filled shape.

use std::ops::Range;

use skrifa::GlyphId;
use skrifa::color::{Brush, Color, ColorGlyph, ColorPainter, CompositeMode, Transform};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlinePen};
use skrifa::raw::types::BoundingBox;
use zeno::{Command, Point};

/// Straight (unmultiplied) RGBA.
pub(crate) type Rgba = [u8; 4];

/// Palette index meaning "the text colour".
const FOREGROUND: u16 = 0xFFFF;

/// Axis-aligned box in font units, y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub const EMPTY: Rect = Rect {
        x0: f32::INFINITY,
        y0: f32::INFINITY,
        x1: f32::NEG_INFINITY,
        y1: f32::NEG_INFINITY,
    };

    pub fn is_empty(&self) -> bool {
        !(self.x0 < self.x1 && self.y0 < self.y1)
    }

    fn add(&mut self, x: f32, y: f32) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
    }

    fn union(&mut self, other: &Rect) {
        if !other.is_empty() {
            self.add(other.x0, other.y0);
            self.add(other.x1, other.y1);
        }
    }
}

/// A closed path: a range of [`Scene::commands`] and the box around its points.
#[derive(Debug, Clone)]
pub(crate) struct Shape {
    pub commands: Range<usize>,
    pub bounds: Rect,
}

#[derive(Debug, Clone)]
pub(crate) enum Op {
    /// Intersect the clip with a shape.
    PushClip(Shape),
    PopClip,
    /// Fill the current clip.
    Fill(Rgba),
    /// Fill a shape inside the current clip.
    FillShape(Shape, Rgba),
}

#[derive(Debug, Default)]
pub(crate) struct Scene {
    pub commands: Vec<Command>,
    pub ops: Vec<Op>,
}

impl Scene {
    /// Box around everything that can receive ink.
    pub fn bounds(&self) -> Rect {
        let mut all = Rect::EMPTY;
        for op in &self.ops {
            if let Op::PushClip(shape) | Op::FillShape(shape, _) = op {
                all.union(&shape.bounds);
            }
        }
        all
    }
}

/// One glyph outline filled with one colour.
pub(crate) fn plain(
    outlines: &OutlineGlyphCollection<'_>,
    glyph: GlyphId,
    color: Rgba,
) -> Option<Scene> {
    let mut scene = Scene::default();
    let shape = outline(outlines, glyph, Transform::IDENTITY, &mut scene.commands)?;
    scene.ops.push(Op::FillShape(shape, color));
    Some(scene)
}

/// A colour glyph with `colors` as its palette. `None` if the glyph is broken
/// or uses paints this crate does not draw (gradients, non-source-over
/// compositing).
pub(crate) fn colored(
    outlines: &OutlineGlyphCollection<'_>,
    glyph: &ColorGlyph<'_>,
    colors: &[Color],
    foreground: Rgba,
) -> Option<Scene> {
    let mut recorder = Recorder {
        outlines,
        colors,
        foreground,
        transforms: Vec::new(),
        scene: Scene::default(),
        failed: false,
    };
    glyph.paint(LocationRef::default(), &mut recorder).ok()?;
    (!recorder.failed).then_some(recorder.scene)
}

/// Whether a glyph outline has any segment.
pub(crate) fn outline_has_ink(outlines: &OutlineGlyphCollection<'_>, glyph: GlyphId) -> bool {
    let mut pen = InkPen(false);
    let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
    outlines
        .get(glyph)
        .is_some_and(|g| g.draw(settings, &mut pen).is_ok() && pen.0)
}

/// Whether a colour glyph refers to any outline with ink. A glyph whose paint
/// graph is broken counts as inked: it exists, it just cannot be drawn.
pub(crate) fn color_has_ink(outlines: &OutlineGlyphCollection<'_>, glyph: &ColorGlyph<'_>) -> bool {
    let mut probe = InkProbe {
        outlines,
        ink: false,
    };
    glyph.paint(LocationRef::default(), &mut probe).is_err() || probe.ink
}

struct InkProbe<'a, 'f> {
    outlines: &'a OutlineGlyphCollection<'f>,
    ink: bool,
}

impl ColorPainter for InkProbe<'_, '_> {
    fn push_transform(&mut self, _: Transform) {}

    fn pop_transform(&mut self) {}

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        self.ink = self.ink || outline_has_ink(self.outlines, glyph_id);
    }

    fn push_clip_box(&mut self, _: BoundingBox<f32>) {}

    fn pop_clip(&mut self) {}

    fn fill(&mut self, _: Brush<'_>) {}

    fn push_layer(&mut self, _: CompositeMode) {}
}

struct InkPen(bool);

impl OutlinePen for InkPen {
    fn move_to(&mut self, _: f32, _: f32) {}

    fn line_to(&mut self, _: f32, _: f32) {
        self.0 = true;
    }

    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 = true;
    }

    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 = true;
    }

    fn close(&mut self) {}
}

struct Recorder<'a, 'f> {
    outlines: &'a OutlineGlyphCollection<'f>,
    colors: &'a [Color],
    foreground: Rgba,
    transforms: Vec<Transform>,
    scene: Scene,
    failed: bool,
}

impl Recorder<'_, '_> {
    fn transform(&self) -> Transform {
        self.transforms
            .last()
            .copied()
            .unwrap_or(Transform::IDENTITY)
    }

    fn glyph_shape(&mut self, glyph: GlyphId) -> Shape {
        let start = self.scene.commands.len();
        outline(
            self.outlines,
            glyph,
            self.transform(),
            &mut self.scene.commands,
        )
        .unwrap_or_else(|| {
            self.failed = true;
            self.scene.commands.truncate(start);
            Shape {
                commands: start..start,
                bounds: Rect::EMPTY,
            }
        })
    }

    fn solid(&mut self, brush: &Brush<'_>) -> Option<Rgba> {
        let Brush::Solid {
            palette_index,
            alpha,
        } = *brush
        else {
            self.failed = true;
            return None;
        };
        let [r, g, b, a] = if palette_index == FOREGROUND {
            self.foreground
        } else if let Some(c) = self.colors.get(usize::from(palette_index)) {
            [c.red, c.green, c.blue, c.alpha]
        } else {
            self.failed = true;
            return None;
        };
        if !alpha.is_finite() {
            self.failed = true;
            return None;
        }
        let a = (f32::from(a) * alpha.clamp(0.0, 1.0)).round() as u8;
        Some([r, g, b, a])
    }
}

impl ColorPainter for Recorder<'_, '_> {
    fn push_transform(&mut self, transform: Transform) {
        self.transforms.push(self.transform() * transform);
    }

    fn pop_transform(&mut self) {
        self.transforms.pop();
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let shape = self.glyph_shape(glyph_id);
        self.scene.ops.push(Op::PushClip(shape));
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let BoundingBox {
            x_min,
            y_min,
            x_max,
            y_max,
        } = clip_box;
        let mut pen = Pen::new(self.transform(), &mut self.scene.commands);
        pen.move_to(x_min, y_min);
        pen.line_to(x_max, y_min);
        pen.line_to(x_max, y_max);
        pen.line_to(x_min, y_max);
        pen.close();
        match pen.finish() {
            Some(shape) => self.scene.ops.push(Op::PushClip(shape)),
            None => self.failed = true,
        }
    }

    fn pop_clip(&mut self) {
        self.scene.ops.push(Op::PopClip);
    }

    fn fill(&mut self, brush: Brush<'_>) {
        if let Some(color) = self.solid(&brush) {
            self.scene.ops.push(Op::Fill(color));
        }
    }

    fn fill_glyph(
        &mut self,
        glyph_id: GlyphId,
        _brush_transform: Option<Transform>,
        brush: Brush<'_>,
    ) {
        if let Some(color) = self.solid(&brush) {
            let shape = self.glyph_shape(glyph_id);
            self.scene.ops.push(Op::FillShape(shape, color));
        }
    }

    // A source-over group is the same as drawing its content straight onto the
    // backdrop, so it needs no buffer of its own.
    fn push_layer(&mut self, composite_mode: CompositeMode) {
        if composite_mode != CompositeMode::SrcOver {
            self.failed = true;
        }
    }
}

/// Appends a glyph outline, transformed, to `commands`. `None` if the glyph is
/// missing, fails to load or has non-finite points.
fn outline(
    outlines: &OutlineGlyphCollection<'_>,
    glyph: GlyphId,
    transform: Transform,
    commands: &mut Vec<Command>,
) -> Option<Shape> {
    let start = commands.len();
    let mut pen = Pen::new(transform, commands);
    let settings = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
    let drawn = outlines
        .get(glyph)
        .and_then(|g| g.draw(settings, &mut pen).ok());
    let shape = pen.finish();
    if drawn.is_none() || shape.is_none() {
        commands.truncate(start);
        return None;
    }
    shape
}

struct Pen<'a> {
    transform: Transform,
    commands: &'a mut Vec<Command>,
    start: usize,
    bounds: Rect,
    finite: bool,
}

impl<'a> Pen<'a> {
    fn new(transform: Transform, commands: &'a mut Vec<Command>) -> Self {
        let start = commands.len();
        Pen {
            transform,
            commands,
            start,
            bounds: Rect::EMPTY,
            finite: true,
        }
    }

    fn point(&mut self, x: f32, y: f32) -> Point {
        let (x, y) = self.transform.transform(x, y);
        self.finite &= x.is_finite() && y.is_finite();
        self.bounds.add(x, y);
        Point::new(x, y)
    }

    fn finish(self) -> Option<Shape> {
        self.finite.then_some(Shape {
            commands: self.start..self.commands.len(),
            bounds: self.bounds,
        })
    }
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.commands.push(Command::MoveTo(p));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.commands.push(Command::LineTo(p));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let c = self.point(cx0, cy0);
        let p = self.point(x, y);
        self.commands.push(Command::QuadTo(c, p));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let c0 = self.point(cx0, cy0);
        let c1 = self.point(cx1, cy1);
        let p = self.point(x, y);
        self.commands.push(Command::CurveTo(c0, c1, p));
    }

    fn close(&mut self) {
        self.commands.push(Command::Close);
    }
}

#[cfg(test)]
mod tests;
