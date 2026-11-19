//! The reading pointer: a dot or ring that takes the mouse cursor's place below
//! the header, with an optional trail, a soft edge, shrinking with speed, and a
//! lens that makes the colours of the words under it more vivid.

use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke, Vec2, vec2};
use quran_core::settings::palette::parse_hex;
use quran_core::settings::{Pointer, PointerShape, trail_lag};

use crate::theme::Colors;

/// Frames a second the pointer's motion constants are written for.
const RATE: f32 = 60.0;
/// A ring's line, as a share of its diameter.
const RING: f32 = 0.12;

/// The lens of the glow: inside it the tajweed colours are drawn more vivid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    pub centre: Pos2,
    pub radius: f32,
    /// Share of the radius that is fully lit; from there the effect fades to nothing at the rim.
    pub solid: f32,
}

impl Lens {
    /// How strongly the lens acts at `at`, 0..=1.
    pub fn strength(&self, at: Pos2) -> f32 {
        let share = at.distance(self.centre) / self.radius.max(0.01);
        if share >= 1.0 {
            0.0
        } else if share <= self.solid {
            1.0
        } else {
            (1.0 - share) / (1.0 - self.solid).max(0.001)
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::from_center_size(self.centre, Vec2::splat(2.0 * self.radius))
    }
}

/// How much more vivid the glow makes colours: `(saturation, brightness)` factors.
pub fn boost(glow: u8) -> (f32, f32) {
    let glow = f32::from(glow.min(10));
    (1.0 + glow * 0.4, 1.0 + glow * 0.04)
}

/// One premultiplied RGBA pixel made more vivid (CSS `saturate()` then `brightness()`).
pub fn boosted(px: [u8; 4], (saturation, brightness): (f32, f32)) -> [u8; 4] {
    let a = f32::from(px[3]);
    if px[3] == 0 {
        return px;
    }
    let [r, g, b] = [px[0], px[1], px[2]].map(|c| f32::from(c) / a);
    let s = saturation;
    let mixed = [
        (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b,
    ];
    let [r, g, b] = mixed.map(|c| ((c * brightness).clamp(0.0, 1.0) * a).round() as u8);
    [r, g, b, px[3]]
}

#[derive(Default)]
pub struct ReadingPointer {
    /// The head and the copies trailing it.
    dots: Vec<Pos2>,
    /// Mouse speed in points per millisecond, smoothed.
    speed: f32,
    /// Size drawn, as a share of the full size (less while moving fast).
    size: f32,
    last: Option<(Pos2, f64)>,
    shown: bool,
}

impl ReadingPointer {
    /// Diameter of the pointer for a line this tall.
    fn diameter(pointer: &Pointer, line_height: f32) -> f32 {
        line_height * f32::from(pointer.size.clamp(1, 10)) / 10.0
    }

    /// Whether the pointer stands in for the cursor at `at`: below the header,
    /// with nothing (the settings panel, a dropdown, a tooltip's owner) over the page there.
    fn over_page(ctx: &egui::Context, area: Rect, at: Pos2) -> bool {
        area.contains(at) && ctx.layer_id_at(at).is_none_or(|layer| layer.order == egui::Order::Background)
    }

    /// The glow's lens for this frame, if the pointer glows and is over the page.
    pub fn lens(&self, ctx: &egui::Context, pointer: &Pointer, area: Rect, line_height: f32) -> Option<Lens> {
        if !pointer.enabled || pointer.glow == 0 {
            return None;
        }
        let at = ctx.input(|i| i.pointer.hover_pos()).filter(|at| Self::over_page(ctx, area, *at))?;
        let blur = f32::from(pointer.blur.min(10)) * 0.03;
        let size = if self.size > 0.0 { self.size } else { 1.0 };
        let radius = Self::diameter(pointer, line_height) * (1.0 + 2.0 * blur) / 2.0 * size;
        Some(Lens { centre: at, radius, solid: (1.0 - 3.0 * blur).max(0.0) })
    }

    /// Move and draw the pointer. Call last in a frame: it hides the mouse
    /// cursor where it draws itself.
    pub fn show(&mut self, ctx: &egui::Context, pointer: &Pointer, colors: &Colors, area: Rect, line_height: f32) {
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let Some(at) = hover.filter(|at| pointer.enabled && Self::over_page(ctx, area, *at)) else {
            self.shown = false;
            self.last = None;
            return;
        };
        let (now, dt) = ctx.input(|i| (i.time, i.stable_dt.min(0.1)));
        let frames = dt * RATE;
        let trail = usize::from(pointer.trail.min(10));
        if !self.shown || self.dots.len() != trail + 1 {
            // Appearing: everything starts at the mouse.
            self.dots = vec![at; trail + 1];
            self.size = 1.0;
            self.speed = 0.0;
            self.shown = true;
        }
        // Speed from the distance the mouse covered since it last moved.
        if let Some((from, when)) = self.last
            && from != at
        {
            let ms = ((now - when) * 1000.0) as f32;
            if ms > 0.0 && ms < 100.0 {
                self.speed = self.speed * 0.7 + from.distance(at) / ms * 0.3;
            }
        }
        if self.last.is_none_or(|(from, _)| from != at) {
            self.last = Some((at, now));
        }
        self.speed *= 0.85f32.powf(frames);
        let shrink = f32::from(pointer.shrink.min(10)) * 0.08;
        let wanted = 1.0 - shrink * (self.speed / 2.0).min(1.0);
        self.size += (wanted - self.size) * (1.0 - 0.7f32.powf(frames));

        // Each copy closes a share of the gap to the one before it.
        let chase = 1.0 - (1.0 - trail_lag(pointer.trail_distance)).powf(frames);
        let mut settling = self.speed > 0.01 || (wanted - self.size).abs() > 0.01;
        self.dots[0] = at;
        for i in 1..self.dots.len() {
            let ahead = self.dots[i - 1];
            let dot = self.dots[i];
            self.dots[i] = dot + (ahead - dot) * chase;
            settling |= self.dots[i].distance(ahead) > 0.5;
        }
        if settling {
            // The trail and the size are still on their way: keep animating.
            ctx.request_repaint();
        }

        let [r, g, b] = parse_hex(&pointer.color).unwrap_or([255; 3]);
        let opacity = f32::from(pointer.opacity.clamp(5, 100)) / 100.0;
        let diameter = Self::diameter(pointer, line_height);
        let blur = diameter * f32::from(pointer.blur.min(10)) * 0.03;
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("reading-pointer")));
        let painter = painter.with_clip_rect(area);
        // Farthest copy first, the head last.
        for (i, centre) in self.dots.iter().enumerate().rev() {
            let fade = 1.0 - i as f32 / (trail + 1) as f32;
            let radius = diameter / 2.0 * (0.4 + 0.6 * fade) * self.size;
            let alpha = opacity * fade;
            let color = |a: f32| Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8);
            let band = match pointer.shape {
                PointerShape::Circle => radius,
                PointerShape::Ring => diameter * RING * (0.4 + 0.6 * fade) * self.size,
            };
            if blur < 0.25 {
                match pointer.shape {
                    PointerShape::Circle => painter.circle_filled(*centre, radius, color(alpha)),
                    PointerShape::Ring => painter.circle_stroke(*centre, radius - band / 2.0, Stroke::new(band, color(alpha))),
                };
            } else {
                painter.add(Shape::mesh(soft_ring(*centre, radius, band, blur, |a| color(alpha * a))));
            }
            // A hairline keeps a light pointer visible on a light page.
            if i == 0 && !colors.dark && blur < 0.25 {
                painter.circle_stroke(*centre, radius, Stroke::new(1.0, Color32::from_black_alpha(51)));
            }
        }
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }
}

/// A ring of outer `radius` and width `band` (a disc when the band is the whole
/// radius) whose edges fade over `blur`, as a mesh with the fade in its vertices.
fn soft_ring(centre: Pos2, radius: f32, band: f32, blur: f32, color: impl Fn(f32) -> Color32) -> Mesh {
    const SEGMENTS: usize = 48;
    const STEPS: usize = 10;
    let inner = (radius - band).max(0.0);
    // How much of the shape covers a point at distance `r`: a smooth step at each edge.
    let step = |x: f32| 1.0 / (1.0 + (-x * 1.7 / blur).exp());
    let cover = |r: f32| step(radius - r) * if inner > 0.0 { step(r - inner) } else { 1.0 };
    let from = (inner - 3.0 * blur).max(0.0);
    let to = radius + 3.0 * blur;
    let mut mesh = Mesh::default();
    for ring in 0..=STEPS {
        let r = from + (to - from) * ring as f32 / STEPS as f32;
        for segment in 0..SEGMENTS {
            let angle = segment as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(centre + r * vec2(angle.cos(), angle.sin()), color(cover(r)));
        }
    }
    for ring in 0..STEPS {
        for segment in 0..SEGMENTS {
            let (a, b) = ((ring * SEGMENTS + segment) as u32, (ring * SEGMENTS + (segment + 1) % SEGMENTS) as u32);
            let (c, d) = (a + SEGMENTS as u32, b + SEGMENTS as u32);
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn the_lens_fades_from_its_solid_part_to_its_rim() {
        let lens = Lens { centre: pos2(100.0, 100.0), radius: 20.0, solid: 0.5 };
        assert_eq!(lens.strength(pos2(100.0, 100.0)), 1.0);
        assert_eq!(lens.strength(pos2(110.0, 100.0)), 1.0);
        assert!((lens.strength(pos2(115.0, 100.0)) - 0.5).abs() < 1e-5);
        assert_eq!(lens.strength(pos2(120.0, 100.0)), 0.0);
        assert_eq!(lens.strength(pos2(100.0, 150.0)), 0.0);
        // No blur: a hard edge.
        let hard = Lens { solid: 1.0, ..lens };
        assert_eq!((hard.strength(pos2(119.0, 100.0)), hard.strength(pos2(121.0, 100.0))), (1.0, 0.0));
        assert_eq!(lens.bounds(), Rect::from_min_max(pos2(80.0, 80.0), pos2(120.0, 120.0)));
    }

    #[test]
    fn boosting_makes_colours_more_vivid_and_leaves_greys_grey() {
        assert_eq!(boost(0), (1.0, 1.0));
        let strong = boost(10);
        assert!((strong.0 - 5.0).abs() < 1e-5 && (strong.1 - 1.4).abs() < 1e-5);
        // Black text stays black, transparent stays transparent, grey stays grey (only brighter).
        assert_eq!(boosted([0, 0, 0, 255], strong), [0, 0, 0, 255]);
        assert_eq!(boosted([0, 0, 0, 0], strong), [0, 0, 0, 0]);
        let grey = boosted([100, 100, 100, 255], boost(5));
        assert!(grey[0] == grey[1] && grey[1] == grey[2] && grey[0] > 100);
        // A muted red becomes a purer red; nothing changes without glow.
        let red = boosted([180, 80, 80, 255], boost(5));
        assert!(red[0] > 180 && red[1] < 80 && red[2] < 80, "{red:?}");
        assert_eq!(boosted([180, 80, 80, 255], boost(0)), [180, 80, 80, 255]);
        // Premultiplied pixels keep their alpha and stay within it.
        let half = boosted([90, 40, 40, 128], boost(5));
        assert!(half[3] == 128 && half.iter().all(|c| *c <= 128));
    }

    #[test]
    fn a_soft_ring_fades_at_both_edges() {
        let alpha = |a: f32| Color32::from_white_alpha((a * 255.0) as u8);
        let disc = soft_ring(pos2(0.0, 0.0), 10.0, 10.0, 2.0, alpha);
        assert!(disc.is_valid());
        // Full in the middle, gone well outside.
        assert!(disc.vertices.first().unwrap().color.a() > 250);
        assert!(disc.vertices.last().unwrap().color.a() < 3);
        let ring = soft_ring(pos2(0.0, 0.0), 10.0, 2.0, 1.0, alpha);
        assert!(ring.vertices.first().unwrap().color.a() < 10, "nothing inside the ring");
        assert!(ring.vertices.iter().any(|v| v.color.a() > 100));
    }
}
