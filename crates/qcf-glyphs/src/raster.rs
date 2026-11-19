//! Draws a [`Scene`] into premultiplied RGBA8.
//!
//! Coverage comes from zeno (exact area coverage, 256 levels); colours are
//! composited source-over in the font's own (sRGB-encoded) values, as browsers
//! do for colour fonts.

use zeno::{Mask, Scratch, Transform};

use crate::scene::{Op, Rect, Rgba, Scene, Shape};

/// Largest image side, in pixels. Bigger requests are refused.
const MAX_SIDE: i64 = 8192;

pub(crate) struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Left edge, pixels right of the origin.
    pub left: i32,
    /// Top edge, pixels above the baseline.
    pub top: i32,
}

/// Pixel box, x right and y down, relative to the canvas.
#[derive(Clone, Copy)]
struct Area {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

struct Canvas {
    w: usize,
    h: usize,
    /// Pixel column of the canvas's left edge and pixel row (y up) of its top edge.
    left: i64,
    top: i64,
    scale: f32,
    rgba: Vec<u8>,
    scratch: Scratch,
}

/// `None` if the image would be unreasonably large. An inkless scene gives a
/// 0×0 image.
pub(crate) fn render(scene: &Scene, scale: f32) -> Option<Image> {
    let empty = Image {
        width: 0,
        height: 0,
        rgba: Vec::new(),
        left: 0,
        top: 0,
    };
    let bounds = scene.bounds();
    if bounds.is_empty() {
        return Some(empty);
    }
    let (left, right, top, bottom) = pixel_box(&bounds, scale)?;
    let (w, h) = ((right - left) as usize, (top - bottom) as usize);
    let mut canvas = Canvas {
        w,
        h,
        left,
        top,
        scale,
        rgba: vec![0; w * h * 4],
        scratch: Scratch::new(),
    };
    let whole = Area { x: 0, y: 0, w, h };

    let mut clips: Vec<Vec<u8>> = Vec::new();
    let mut spare: Vec<Vec<u8>> = Vec::new();
    for op in &scene.ops {
        match op {
            Op::PushClip(shape) => {
                let mut mask = spare.pop().unwrap_or_default();
                canvas.coverage(scene, shape, whole, &mut mask);
                if let Some(outer) = clips.last() {
                    intersect(&mut mask, outer);
                }
                clips.push(mask);
            }
            Op::PopClip => spare.extend(clips.pop()),
            Op::Fill(color) => match clips.last() {
                Some(clip) => canvas.blend(whole, clip, *color),
                None => {
                    let mut mask = spare.pop().unwrap_or_default();
                    mask.clear();
                    mask.resize(w * h, 255);
                    canvas.blend(whole, &mask, *color);
                    spare.push(mask);
                }
            },
            Op::FillShape(shape, color) => {
                let mut mask = spare.pop().unwrap_or_default();
                match clips.last() {
                    Some(clip) => {
                        canvas.coverage(scene, shape, whole, &mut mask);
                        intersect(&mut mask, clip);
                        canvas.blend(whole, &mask, *color);
                    }
                    None => {
                        if let Some(area) = canvas.area_of(&shape.bounds) {
                            canvas.coverage(scene, shape, area, &mut mask);
                            canvas.blend(area, &mask, *color);
                        }
                    }
                }
                spare.push(mask);
            }
        }
    }
    Some(canvas.crop().unwrap_or(empty))
}

/// Whole-pixel box (left, right, top, bottom; y up) around `bounds` at `scale`.
fn pixel_box(bounds: &Rect, scale: f32) -> Option<(i64, i64, i64, i64)> {
    let edges = [
        (bounds.x0 * scale).floor(),
        (bounds.x1 * scale).ceil(),
        (bounds.y1 * scale).ceil(),
        (bounds.y0 * scale).floor(),
    ];
    if edges.iter().any(|e| !e.is_finite() || e.abs() > 1.0e6) {
        return None;
    }
    let [left, right, top, bottom] = edges.map(|e| e as i64);
    let (w, h) = (right - left, top - bottom);
    (w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE).then_some((left, right, top, bottom))
}

impl Canvas {
    /// The canvas pixels a font-unit box touches.
    fn area_of(&self, bounds: &Rect) -> Option<Area> {
        if bounds.is_empty() {
            return None;
        }
        let (left, right, top, bottom) = pixel_box(bounds, self.scale)?;
        let x0 = (left - self.left).clamp(0, self.w as i64) as usize;
        let x1 = (right - self.left).clamp(0, self.w as i64) as usize;
        let y0 = (self.top - top).clamp(0, self.h as i64) as usize;
        let y1 = (self.top - bottom).clamp(0, self.h as i64) as usize;
        (x1 > x0 && y1 > y0).then_some(Area {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }

    /// Fills `mask` (resized to `area`) with the coverage of `shape`.
    fn coverage(&mut self, scene: &Scene, shape: &Shape, area: Area, mask: &mut Vec<u8>) {
        mask.clear();
        mask.resize(area.w * area.h, 0);
        let Some(commands) = scene.commands.get(shape.commands.clone()) else {
            return;
        };
        if commands.is_empty() || mask.is_empty() {
            return;
        }
        let s = self.scale;
        let dx = -(self.left as f32) - area.x as f32;
        let dy = self.top as f32 - area.y as f32;
        Mask::with_scratch(commands, &mut self.scratch)
            .transform(Some(Transform::new(s, 0.0, 0.0, -s, dx, dy)))
            .size(area.w as u32, area.h as u32)
            .render_into(mask, None);
    }

    /// Source-over of `color` through `mask` (one byte per pixel of `area`).
    fn blend(&mut self, area: Area, mask: &[u8], [r, g, b, a]: Rgba) {
        if a == 0 || area.w == 0 {
            return;
        }
        let (r, g, b, a) = (u32::from(r), u32::from(g), u32::from(b), u32::from(a));
        for (row, mask_row) in mask.chunks_exact(area.w).take(area.h).enumerate() {
            let start = ((area.y + row) * self.w + area.x) * 4;
            let Some(pixels) = self.rgba.get_mut(start..start + area.w * 4) else {
                return;
            };
            for (px, &cov) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask_row) {
                if cov == 0 {
                    continue;
                }
                let sa = div255(a * u32::from(cov));
                if sa == 255 {
                    *px = [r as u8, g as u8, b as u8, 255];
                    continue;
                }
                let keep = 255 - sa;
                px[0] = (div255(r * sa) + div255(u32::from(px[0]) * keep)).min(255) as u8;
                px[1] = (div255(g * sa) + div255(u32::from(px[1]) * keep)).min(255) as u8;
                px[2] = (div255(b * sa) + div255(u32::from(px[2]) * keep)).min(255) as u8;
                px[3] = (sa + div255(u32::from(px[3]) * keep)).min(255) as u8;
            }
        }
    }

    /// Trims fully transparent edges. `None` if nothing was drawn.
    fn crop(self) -> Option<Image> {
        let (w, h) = (self.w, self.h);
        let alpha = |x: usize, y: usize| self.rgba[(y * w + x) * 4 + 3];
        let row_has_ink = |y: usize| (0..w).any(|x| alpha(x, y) != 0);
        let y0 = (0..h).find(|&y| row_has_ink(y))?;
        let y1 = (y0..h).rev().find(|&y| row_has_ink(y))? + 1;
        let col_has_ink = |x: usize| (y0..y1).any(|y| alpha(x, y) != 0);
        let x0 = (0..w).find(|&x| col_has_ink(x))?;
        let x1 = (x0..w).rev().find(|&x| col_has_ink(x))? + 1;

        let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
        for y in y0..y1 {
            rgba.extend_from_slice(&self.rgba[(y * w + x0) * 4..(y * w + x1) * 4]);
        }
        Some(Image {
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
            rgba,
            left: (self.left + x0 as i64) as i32,
            top: (self.top - y0 as i64) as i32,
        })
    }
}

fn intersect(mask: &mut [u8], other: &[u8]) {
    for (m, &o) in mask.iter_mut().zip(other) {
        *m = div255(u32::from(*m) * u32::from(o)) as u8;
    }
}

/// `x / 255`, rounded, for `x` up to 255 * 255.
fn div255(x: u32) -> u32 {
    let x = x + 128;
    (x + (x >> 8)) >> 8
}
