//! The reading pointer: a dot or ring that takes the mouse cursor's place below
//! the header, with a smudge along the path it has just travelled (a motion
//! blur), a soft edge, shrinking with speed, and a lens that makes the colours of
//! the words under it more vivid.

use std::collections::VecDeque;

use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke, Vec2, vec2};
use quran_core::settings::palette::parse_hex;
use quran_core::settings::{Pointer, PointerShape, smudge_reach};

use crate::theme::Colors;

/// Frames a second the pointer's motion constants are written for.
const RATE: f32 = 60.0;
/// A ring's line, as a share of its diameter.
const RING: f32 = 0.12;
/// The smudge is the pointer's own shape stamped along its path this share of its radius apart, each
/// stamp so faint that together they make one streak; edges softened by half this spacing leave no
/// ribs between the stamps.
const SPACING: f32 = 0.3;
/// How opaque the streak is where it is densest, at strength 10, as a share of the pointer's opacity.
const PEAK: f32 = 0.7;
/// The most stamps a smudge is drawn with (34 at the longest reach).
const STAMPS: usize = 40;
/// Seconds of path the head's speed is measured over, for how long its smudge may be.
const SPEED: f32 = 0.06;
/// Seconds a smudge of the full reach takes to draw back into a head that has stopped.
const RETRACT: f32 = 0.25;

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
    /// Where the head has been lately, and when (newest last): the path its smudge lies along.
    path: VecDeque<(Pos2, f64)>,
    /// How far back along the path the smudge reaches, in points: as far as the head has come, up to
    /// the reach, and less once the head slows (it draws back then).
    tail: f32,
    /// Mouse speed in points per millisecond, smoothed.
    speed: f32,
    /// Size drawn, as a share of the full size (less while moving fast).
    size: f32,
    last: Option<(Pos2, f64)>,
    shown: bool,
    /// When it was last drawn, and whether it was on its way then (moving, or its smudge or size still settling).
    drawn: Option<(f64, bool)>,
}

impl ReadingPointer {
    /// Diameter of the pointer for a line this tall.
    fn diameter(pointer: &Pointer, line_height: f32) -> f32 {
        line_height * f32::from(pointer.size.clamp(1, 20)) / 10.0
    }

    /// Whether the pointer stands in for the cursor at `at`: below the header,
    /// with nothing (the settings panel, a dropdown, a tooltip's owner) over the page there.
    fn over_page(ctx: &egui::Context, area: Rect, at: Pos2) -> bool {
        area.contains(at) && ctx.layer_id_at(at).is_none_or(|layer| layer.order == egui::Order::Background)
    }

    /// The glow's lens for this frame, if the pointer glows and is over the page.
    pub fn lens(&self, ctx: &egui::Context, pointer: &Pointer, area: Rect, line_height: f32) -> Option<Lens> {
        if !pointer.enabled {
            return None;
        }
        let at = ctx.input(|i| i.pointer.hover_pos()).filter(|at| Self::over_page(ctx, area, *at))?;
        self.lens_at(pointer, at, line_height)
    }

    /// The glow's lens with the pointer at `at`, if it glows.
    pub fn lens_at(&self, pointer: &Pointer, at: Pos2, line_height: f32) -> Option<Lens> {
        if pointer.glow == 0 {
            return None;
        }
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
            self.hide();
            return;
        };
        let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("reading-pointer"));
        if self.draw(ctx, layer, pointer, colors, area, line_height, at, 1.0, None) {
            ctx.request_repaint();
        }
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }

    /// The page moved under the pointer by `by` (it scrolled). The smudge lies on the page, so its path moves with
    /// it: a head that stays on its word while the page scrolls leaves no streak (in screen space it smeared along the
    /// scroll). The voice pointer's; the mouse's head stays where the cursor is.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn scrolled(&mut self, by: Vec2) {
        for (p, _) in self.path.iter_mut().chain(self.last.as_mut()) {
            *p += by;
        }
    }

    /// The oldest and the newest point of the smudge's path.
    #[cfg(test)]
    pub fn path_ends(&self) -> Option<(Pos2, Pos2)> {
        Some((self.path.front()?.0, self.path.back()?.0))
    }

    /// Nothing drawn: the next pointer appears with nothing behind it to smear.
    pub fn hide(&mut self) {
        self.shown = false;
        self.last = None;
    }

    /// Move the pointer to `at` (not the mouse: the voice pointer) and draw it at `alpha` of its
    /// opacity, on `layer` (its own over the page; the settings' preview's own).
    /// `size` is what the pointer's size is in tenths of (the voice pointer: the text size); `ring`, a
    /// ring's line in points instead of a share of its size. Whether its smudge or its size is still on its
    /// way with the head where it is (the caller asks for the frames: the head moves by its own).
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    pub fn show_at(&mut self, ctx: &egui::Context, layer: egui::LayerId, pointer: &Pointer, colors: &Colors, area: Rect, size: f32, ring: Option<f32>, at: Pos2, alpha: f32) -> bool {
        self.draw(ctx, layer, pointer, colors, area, size, at, alpha, ring)
    }

    /// Draws the pointer; whether it is still on its way with the head where it is: the smudge drawing back, the size
    /// easing by half a pixel or more, or (shrinking with speed) the speed dying down.
    #[allow(clippy::too_many_arguments)]
    fn draw(&mut self, ctx: &egui::Context, layer: egui::LayerId, pointer: &Pointer, colors: &Colors, area: Rect, line_height: f32, at: Pos2, alpha: f32, ring: Option<f32>) -> bool {
        // The time since the last frame while it was on its way (frames come when they are asked for, however far
        // apart); after a rest, egui's steady measure (a frame, not the whole sleep).
        let (now, steady) = ctx.input(|i| (i.time, i.stable_dt));
        let dt = match self.drawn {
            Some((then, true)) => (now - then) as f32,
            _ => steady,
        }
        .clamp(0.0, 0.1);
        let frames = dt * RATE;
        let moved = self.last.is_some_and(|(from, _)| from != at);
        if !self.shown {
            // Appearing: nothing behind it to smear yet.
            self.path.clear();
            self.tail = 0.0;
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
        let diameter = Self::diameter(pointer, line_height);
        // Within half a pixel of its size it is at its size.
        let px = 0.5 / ctx.pixels_per_point();
        if (wanted - self.size).abs() * diameter / 2.0 < px {
            self.size = wanted;
        }
        let (window, widths) = smudge_reach(pointer.smudge_distance);
        self.trace(at, now, dt, pointer.smudge > 0, window, widths * diameter);

        let [r, g, b] = parse_hex(&pointer.color).unwrap_or([255; 3]);
        let color = |a: f32| Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8);
        let opacity = f32::from(pointer.opacity.clamp(5, 100)) / 100.0 * alpha.clamp(0.0, 1.0);
        let blur = diameter * f32::from(pointer.blur.min(10)) * 0.03;
        let radius = diameter / 2.0 * self.size;
        let band = match pointer.shape {
            PointerShape::Circle => radius,
            // A ring keeps its hole: a set line is at most a quarter of its width.
            PointerShape::Ring => ring.map_or(diameter * RING, |r| r.min(diameter / 4.0)) * self.size,
        };
        let painter = ctx.layer_painter(layer).with_clip_rect(area);
        let ds = (radius * SPACING).max(1.0);
        let stamps = smear(&self.path, at, now, ds, window, self.tail);
        // A smudge drawn draws back into a head that stays, frame by frame; the size eases; a shrunken pointer grows
        // back as its speed dies down (only the mouse pointer shrinks).
        let settling = !stamps.is_empty() || self.size != wanted || (shrink > 0.0 && self.speed > 0.01);
        self.drawn = Some((now, moved || settling));
        if !stamps.is_empty() {
            // One mesh under the head; every stamp is the same colour, so their order does not matter.
            let chord = 2.0 * (band * (2.0 * radius - band)).sqrt();
            let soft = blur.max(0.5 * ds);
            let mut mesh = Mesh::default();
            for (centre, weight) in stamps {
                let a = stamp_alpha(opacity, pointer.smudge, weight, chord, ds);
                soft_ring(&mut mesh, centre, radius, band, soft, 32, |c| color(a * c));
            }
            clear_under_head(&mut mesh, at, radius, band, blur, soft);
            painter.add(Shape::mesh(mesh));
        }
        if blur < 0.25 {
            match pointer.shape {
                PointerShape::Circle => painter.circle_filled(at, radius, color(opacity)),
                PointerShape::Ring => painter.add(hard_ring(at, radius, band, color(opacity))),
            };
            // A hairline keeps a light pointer visible on a light page.
            if !colors.dark {
                painter.circle_stroke(at, radius, Stroke::new(1.0, Color32::from_black_alpha((51.0 * opacity) as u8)));
            }
        } else {
            let mut mesh = Mesh::default();
            soft_ring(&mut mesh, at, radius, band, blur, 48, |c| color(opacity * c));
            painter.add(Shape::mesh(mesh));
        }
        settling
    }

    /// Notes the head at `at`, `dt` after the last frame: the path behind it, kept for `window` seconds
    /// and only as far back as the smudge reaches (none without a smudge). The smudge grows as the head
    /// moves, up to `reach` points, and draws back into it, a full reach in [`RETRACT`] seconds, while
    /// the head is slower than would make it that long (at once when it stops, so a fast move leaves no
    /// streak standing behind it). What it draws back from is gone: a later twitch brings none back.
    fn trace(&mut self, at: Pos2, now: f64, dt: f32, smudge: bool, window: f32, reach: f32) {
        if !smudge {
            self.path.clear();
            self.tail = 0.0;
            return;
        }
        match self.path.back().copied() {
            Some((p, t)) if p.distance(at) > 0.25 => {
                // It rested there until the last frame: it moved off now, not when it arrived.
                let left = now - f64::from(dt);
                if t < left - 1e-4 {
                    self.path.push_back((p, left));
                }
                self.path.push_back((at, now));
                self.tail = (self.tail + p.distance(at)).min(reach);
            }
            Some(_) => {}
            None => self.path.push_back((at, now)),
        }
        while self.path.len() > 1 && self.path.front().is_some_and(|(_, t)| now - t > f64::from(window)) {
            self.path.pop_front();
        }
        let long = reach.min(recent(&self.path, now, SPEED) / SPEED * window);
        if self.tail > long {
            self.tail = (self.tail - reach * dt / RETRACT).max(long);
        }
        cut(&mut self.path, at, self.tail);
    }
}

/// How far the head went along `path` (newest last) in the last `secs` seconds.
fn recent(path: &VecDeque<(Pos2, f64)>, now: f64, secs: f32) -> f32 {
    let since = now - f64::from(secs);
    let mut went = 0.0;
    for (&(a, ta), &(b, tb)) in path.iter().rev().skip(1).zip(path.iter().rev()) {
        if tb <= since {
            break;
        }
        let share = if ta >= since { 1.0 } else { ((tb - since) / (tb - ta)) as f32 };
        went += a.distance(b) * share;
    }
    went
}

/// Drops what lies more than `keep` behind the head at `at` along `path` (newest last), but for the
/// first point past it.
fn cut(path: &mut VecDeque<(Pos2, f64)>, at: Pos2, keep: f32) {
    let (mut from, mut walked) = (at, 0.0);
    for (i, &(p, _)) in path.iter().enumerate().rev() {
        walked += from.distance(p);
        from = p;
        if walked >= keep {
            path.drain(..i);
            return;
        }
    }
}

/// The smudge behind a head at `at` that came along `path` (newest last): the centres of its stamps,
/// `ds` apart along the path from the head (so they slide with it instead of popping in and out), each
/// with its weight (1 - u)², u being how far it lies through the smudge in time (`window` seconds since
/// the head was there) or in length (`tail` points), whichever is further. It draws back into the head
/// as `tail` shortens; a head that has not moved `ds` has none.
fn smear(path: &VecDeque<(Pos2, f64)>, at: Pos2, now: f64, ds: f32, window: f32, tail: f32) -> Vec<(Pos2, f32)> {
    let mut stamps = Vec::new();
    let (mut from, mut from_age, mut walked, mut next) = (at, 0.0, 0.0, ds);
    for &(to, t) in path.iter().rev() {
        let age = (now - t) as f32;
        let seg = from.distance(to);
        while seg > 0.0 && walked + seg >= next {
            let f = (next - walked) / seg;
            let u = ((from_age + (age - from_age) * f) / window).max(next / tail);
            if u >= 1.0 || stamps.len() == STAMPS {
                return stamps;
            }
            stamps.push((from.lerp(to, f), (1.0 - u) * (1.0 - u)));
            next += ds;
        }
        (from, from_age, walked) = (to, age, walked + seg);
    }
    stamps
}

/// The opacity to draw a stamp of weight `weight` at, so that where the streak is densest (a point
/// there lies under `chord / ds` stamps; `chord` is the longest line across the shape's band, the
/// diameter for a disc, the rail for a ring) it reaches its share of the pointer's opacity: at most
/// [`PEAK`] of it at strength 10.
fn stamp_alpha(opacity: f32, smudge: u8, weight: f32, chord: f32, ds: f32) -> f32 {
    let wanted = (opacity * f32::from(smudge.min(10)) / 10.0 * PEAK * weight).clamp(0.0, 1.0);
    1.0 - (1.0 - wanted).powf(ds / chord.max(ds))
}

/// A sharp ring of outer `radius` and width `band`: where [`soft_ring`] puts the line (and quran-dx's
/// border-box ring), so the smudge is as wide as the head and an edge blur keeps its size. epaint strokes
/// a circle outside its radius, so the circle is the line's inner edge.
fn hard_ring(centre: Pos2, radius: f32, band: f32, color: Color32) -> Shape {
    Shape::circle_stroke(centre, radius - band, Stroke::new(band, color))
}

/// Rings of vertices across a [`soft_ring`], centre to rim.
const STEPS: u32 = 10;

/// How much a ring of outer `radius` and width `band` whose edges fade over `blur` covers a point `r`
/// from its centre: a smooth step at each edge.
fn ring_cover(r: f32, radius: f32, band: f32, blur: f32) -> f32 {
    let step = |x: f32| 1.0 / (1.0 + (-x * 1.7 / blur).exp());
    let inner = (radius - band).max(0.0);
    step(radius - r) * if inner > 0.0 { step(r - inner) } else { 1.0 }
}

/// Takes out of a smudge `mesh` (stamps softened by `soft`) whatever lies under the head at `at`
/// (`blur` its own edge), so head and smudge never add up to more than the pointer's opacity: the head
/// shows the same whether it moves or rests, and the smudge only appears behind it. The head's edges
/// grow by half a ring step of the stamps, so their coarser triangles cannot reach in under it.
fn clear_under_head(mesh: &mut Mesh, at: Pos2, radius: f32, band: f32, blur: f32, soft: f32) {
    let inner = (radius - band).max(0.0);
    let half = 0.5 * (radius + 3.0 * soft - (inner - 3.0 * soft).max(0.0)) / STEPS as f32;
    let (radius, band) = (radius + half, if inner > 0.0 { band + 2.0 * half } else { radius + half });
    for v in &mut mesh.vertices {
        let under = ring_cover(v.pos.distance(at), radius, band, blur.max(0.5));
        v.color = v.color.gamma_multiply(1.0 - under);
    }
}

/// Appends a ring of outer `radius` and width `band` (a disc when the band is the whole radius) whose
/// edges fade over `blur` to `mesh`, the fade in its vertices; `segments` around.
fn soft_ring(mesh: &mut Mesh, centre: Pos2, radius: f32, band: f32, blur: f32, segments: u32, color: impl Fn(f32) -> Color32) {
    let inner = (radius - band).max(0.0);
    let from = (inner - 3.0 * blur).max(0.0);
    let to = radius + 3.0 * blur;
    let first = mesh.vertices.len() as u32;
    for ring in 0..=STEPS {
        let r = from + (to - from) * ring as f32 / STEPS as f32;
        let ink = color(ring_cover(r, radius, band, blur));
        for segment in 0..segments {
            let angle = segment as f32 / segments as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(centre + r * vec2(angle.cos(), angle.sin()), ink);
        }
    }
    for ring in 0..STEPS {
        for segment in 0..segments {
            let a = first + ring * segments + segment;
            let b = first + ring * segments + (segment + 1) % segments;
            mesh.add_triangle(a, b, a + segments);
            mesh.add_triangle(b, b + segments, a + segments);
        }
    }
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
        let mut disc = Mesh::default();
        soft_ring(&mut disc, pos2(0.0, 0.0), 10.0, 10.0, 2.0, 48, alpha);
        assert!(disc.is_valid());
        // Full in the middle, gone well outside.
        assert!(disc.vertices.first().unwrap().color.a() > 250);
        assert!(disc.vertices.last().unwrap().color.a() < 3);
        // A second ring in the same mesh: its own vertices, nothing inside it.
        soft_ring(&mut disc, pos2(50.0, 0.0), 10.0, 2.0, 1.0, 32, alpha);
        assert!(disc.is_valid() && disc.vertices.len() == 11 * (48 + 32));
        let ring = &disc.vertices[11 * 48..];
        assert!(ring.first().unwrap().color.a() < 10, "nothing inside the ring");
        assert!(ring.iter().any(|v| v.color.a() > 100));
    }

    const FRAME: f32 = 1.0 / 60.0;

    /// Moves `p` along `points` from `now`, a point a frame at 60 a second, its smudge reaching `window`
    /// seconds and `reach` points; returns where and when it ends.
    fn go(p: &mut ReadingPointer, points: impl IntoIterator<Item = Pos2>, mut now: f64, window: f32, reach: f32) -> (Pos2, f64) {
        let mut at = p.path.back().map_or(pos2(0.0, 0.0), |(q, _)| *q);
        for point in points {
            now += f64::from(FRAME);
            at = point;
            p.trace(at, now, FRAME, true, window, reach);
        }
        (at, now)
    }

    /// A pointer that moved along `points` from rest.
    fn moved(points: impl IntoIterator<Item = Pos2>, window: f32, reach: f32) -> (ReadingPointer, Pos2, f64) {
        let mut p = ReadingPointer::default();
        let (at, now) = go(&mut p, points, 0.0, window, reach);
        (p, at, now)
    }

    /// Points along x at `speed` points a second, for `secs`.
    fn line(speed: f32, secs: f32) -> impl Iterator<Item = Pos2> {
        (1..=(secs * 60.0) as u32).map(move |i| pos2(speed * i as f32 / 60.0, 0.0))
    }

    /// How far behind a head at `at` its smudge's last stamp lies (0: none).
    fn farthest(p: &ReadingPointer, at: Pos2, now: f64, ds: f32, window: f32) -> f32 {
        smear(&p.path, at, now, ds, window, p.tail).iter().map(|(c, _)| at.distance(*c)).fold(0.0, f32::max)
    }

    #[test]
    fn no_smudge_until_the_head_has_moved() {
        let (p, at, now) = moved([pos2(5.0, 5.0); 30], 0.75, 100.0);
        assert!(smear(&p.path, at, now, 6.0, 0.75, p.tail).is_empty(), "still");
        let (p, at, now) = moved((0..5).map(|i| pos2(i as f32, 0.0)), 0.75, 100.0);
        assert!(smear(&p.path, at, now, 6.0, 0.75, p.tail).is_empty(), "moved less than the spacing");
        let mut off = ReadingPointer::default();
        off.trace(at, now, FRAME, false, 0.75, 100.0);
        assert!(off.path.is_empty(), "no path kept without a smudge");
    }

    #[test]
    fn a_steady_move_smears_evenly_spaced_fading_stamps() {
        let (ds, reach) = (6.0, 100.0);
        let (p, at, now) = moved(line(600.0, 1.0), 0.75, reach);
        assert_eq!(p.tail, reach);
        let stamps = smear(&p.path, at, now, ds, 0.75, p.tail);
        assert!(stamps.len() > 10 && stamps.len() <= STAMPS, "{}", stamps.len());
        for (k, (centre, _)) in stamps.iter().enumerate() {
            assert!((at.x - centre.x - ds * (k + 1) as f32).abs() < 1e-3 && centre.y == 0.0, "stamp {k} at {centre:?}");
            assert!(at.distance(*centre) < reach);
        }
        assert!(stamps.windows(2).all(|w| w[1].1 < w[0].1), "weights fall");
        // Slowly (20 points a second), it reaches only as far as the head went in the time.
        let (p, at, now) = moved(line(20.0, 2.0), 0.75, reach);
        let stamps = smear(&p.path, at, now, 2.0, 0.75, p.tail);
        assert!(!stamps.is_empty() && stamps.iter().all(|(c, _)| at.distance(*c) <= 20.0 * 0.75), "{stamps:?}");
    }

    #[test]
    fn a_stopped_smudge_draws_back_into_the_head() {
        let (mut p, at, mut now) = moved(line(300.0, 0.5), 0.75, 100.0);
        let mut stamps = smear(&p.path, at, now, 6.0, 0.75, p.tail);
        let start = now;
        while now < start + 0.75 {
            (_, now) = go(&mut p, [at], now, 0.75, 100.0);
            let later = smear(&p.path, at, now, 6.0, 0.75, p.tail);
            // The same places, the farthest gone first.
            assert!(later.len() <= stamps.len() && later.iter().zip(&stamps).all(|(a, b)| a.0 == b.0 && a.1 <= b.1));
            stamps = later;
        }
        assert!(stamps.is_empty(), "{} left after the window", stamps.len());
        assert!(p.path.iter().all(|(q, _)| q.distance(at) <= 0.5), "nothing left to animate");
    }

    #[test]
    fn a_fast_flick_draws_back_as_soon_as_it_stops() {
        // 900 points a second at the longest distance (1.5 s, 170 points): the far end comes in from
        // the first frames, never out again, and it is gone within a third of a second.
        let (window, reach, ds) = (1.5, 170.0, 5.0);
        let (mut p, at, mut now) = moved(line(900.0, 0.4), window, reach);
        let mut far = farthest(&p, at, now, ds, window);
        assert!(far > 0.9 * reach, "{far}");
        let stop = now;
        while far > 0.0 {
            (_, now) = go(&mut p, [at], now, window, reach);
            let later = farthest(&p, at, now, ds, window);
            assert!(later <= far, "out again to {later} from {far} at +{:.3} s", now - stop);
            assert!(now - stop < 0.12 || later < 0.85 * reach, "still {later} at +{:.3} s", now - stop);
            far = later;
        }
        assert!(now - stop <= f64::from(SPEED + RETRACT) + 0.02, "gone after {:.3} s", now - stop);
        assert!(p.path.iter().all(|(q, _)| q.distance(at) <= 0.5), "nothing left to animate");
    }

    #[test]
    fn a_head_that_slows_keeps_a_short_smudge_and_a_twitch_brings_none_back() {
        let (window, reach, ds) = (0.75, 100.0, 4.0);
        // A flick, then creeping on at 30 points a second: no longer than the creep makes it.
        let (mut p, at, now) = moved(line(900.0, 0.3), window, reach);
        let (at, now) = go(&mut p, (1..=60).map(|i| at + vec2(0.5 * i as f32, 0.0)), now, window, reach);
        let far = farthest(&p, at, now, ds, window);
        assert!(far > 0.0 && far <= 30.0 * window + ds, "{far}");
        // Stopped a while (partly drawn back), then a twitch of a point: no more than that comes back.
        let (mut p, at, now) = moved(line(900.0, 0.3), window, reach);
        let (_, now) = go(&mut p, [at; 12], now, window, reach);
        let before = farthest(&p, at, now, ds, window);
        assert!(before > 0.0 && before < 0.9 * reach, "{before}");
        let (twitch, now) = go(&mut p, [at + vec2(1.0, 0.0)], now, window, reach);
        let after = farthest(&p, twitch, now, ds, window);
        assert!(after <= before + 1.0 + 1e-3, "{after} from {before}");
    }

    #[test]
    fn a_sharp_ring_lies_where_its_smudge_and_its_blurred_self_do() {
        use egui::epaint::{TessellationOptions, Tessellator};
        let (radius, band) = (20.0f32, 6.0);
        let inside = |r: f32| (radius - band - 1e-3..=radius + 1e-3).contains(&r);
        let radii = |mesh: &Mesh, least: u8| mesh.vertices.iter().filter(|v| v.color.a() >= least).map(|v| v.pos.to_vec2().length()).collect::<Vec<_>>();
        let mut sharp = Mesh::default();
        let mut tessellator = Tessellator::new(1.0, TessellationOptions::default(), [1, 1], vec![]);
        tessellator.tessellate_shape(hard_ring(pos2(0.0, 0.0), radius, band, Color32::WHITE), &mut sharp);
        // Solid from radius - band to radius, feathered half a point either side.
        assert!(radii(&sharp, 255).into_iter().all(inside), "{:?}", radii(&sharp, 255));
        assert!(radii(&sharp, 1).into_iter().all(|r| (radius - band - 0.51..=radius + 0.51).contains(&r)));
        // A stamp of the smudge, or a blurred head: at least half covered over the same band.
        let mut soft = Mesh::default();
        soft_ring(&mut soft, pos2(0.0, 0.0), radius, band, 0.5, 32, |c| Color32::from_white_alpha((c * 255.0).round() as u8));
        assert!(soft.vertices.iter().all(|v| (v.color.a() >= 128) == inside(v.pos.to_vec2().length())));
    }

    #[test]
    fn stamps_keep_their_spacing_round_a_corner() {
        // Down from (0, 40) to (0, 0), then right to the head at (30, 0).
        let path = VecDeque::from([(pos2(0.0, 40.0), 0.9), (pos2(0.0, 0.0), 0.95), (pos2(30.0, 0.0), 1.0)]);
        let stamps = smear(&path, pos2(30.0, 0.0), 1.0, 7.0, 10.0, 1000.0);
        let along = |c: Pos2| if c.y == 0.0 { 30.0 - c.x } else { 30.0 + c.y };
        assert_eq!(stamps.len(), 10);
        for (k, (c, _)) in stamps.iter().enumerate() {
            assert!((along(*c) - 7.0 * (k + 1) as f32).abs() < 1e-3, "stamp {k} at {c:?}");
        }
    }

    #[test]
    fn stamps_add_up_to_the_streaks_opacity() {
        for (radius, band) in [(20.0f32, 20.0f32), (20.0, 4.8), (20.0, 1.5)] {
            let ds = radius * SPACING;
            let chord = 2.0 * (band * (2.0 * radius - band)).sqrt();
            for (opacity, smudge, weight) in [(0.5, 5, 1.0), (0.9, 10, 1.0), (1.0, 10, 0.3), (0.05, 1, 0.5)] {
                let a = stamp_alpha(opacity, smudge, weight, chord, ds);
                let wanted = opacity * f32::from(smudge) / 10.0 * PEAK * weight;
                let streak = 1.0 - (1.0 - a).powf(chord / ds);
                assert!((streak - wanted).abs() < 1e-4, "{radius}/{band}: {streak} for {wanted}");
                assert!(a <= wanted && streak <= opacity * PEAK + 1e-6);
            }
        }
        assert_eq!(stamp_alpha(1.0, 99, 1.0, 40.0, 6.0), stamp_alpha(1.0, 10, 1.0, 40.0, 6.0));
    }

    /// The opacity a mesh's triangles give each point of a grid (`step` apart, from `origin`), drawn
    /// over one another as egui blends them.
    fn rasterise(mesh: &Mesh, origin: Pos2, size: (usize, usize), step: f32) -> Vec<f32> {
        let mut out = vec![0.0f32; size.0 * size.1];
        for t in mesh.indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
            let area = (b.pos - a.pos).x * (c.pos - a.pos).y - (b.pos - a.pos).y * (c.pos - a.pos).x;
            if area.abs() < 1e-6 {
                continue;
            }
            let (lo, hi) = (a.pos.min(b.pos).min(c.pos), a.pos.max(b.pos).max(c.pos));
            let x0 = (((lo.x - origin.x) / step).floor().max(0.0) as usize).min(size.0);
            let x1 = (((hi.x - origin.x) / step).ceil().max(0.0) as usize).min(size.0);
            let y0 = (((lo.y - origin.y) / step).floor().max(0.0) as usize).min(size.1);
            let y1 = (((hi.y - origin.y) / step).ceil().max(0.0) as usize).min(size.1);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = origin + vec2(x as f32, y as f32) * step;
                    let wb = ((p - a.pos).x * (c.pos - a.pos).y - (p - a.pos).y * (c.pos - a.pos).x) / area;
                    let wc = ((b.pos - a.pos).x * (p - a.pos).y - (b.pos - a.pos).y * (p - a.pos).x) / area;
                    let wa = 1.0 - wb - wc;
                    if wa > 0.0 && wb > 0.0 && wc > 0.0 {
                        let alpha = (wa * f32::from(a.color.a()) + wb * f32::from(b.color.a()) + wc * f32::from(c.color.a())) / 255.0;
                        let o = &mut out[y * size.0 + x];
                        *o = 1.0 - (1.0 - *o) * (1.0 - alpha);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn the_smudge_never_makes_the_pointer_more_opaque_than_it_is() {
        // A head at (200, 0) that came from the left at 400 points a second, strongest smudge.
        let at = pos2(200.0, 0.0);
        let path: VecDeque<(Pos2, f64)> = (0..=100).map(|i| (pos2(2.0 * i as f32, 0.0), f64::from(i) * 0.005)).collect();
        let (now, opacity) = (0.5, 0.8);
        for (radius, band, blur) in [(12.0f32, 12.0f32, 0.0f32), (12.0, 12.0, 3.0), (12.0, 3.0, 0.0), (12.0, 3.0, 2.0)] {
            let ds = (radius * SPACING).max(1.0);
            let stamps = smear(&path, at, now, ds, 0.75, 200.0);
            assert!(stamps.len() > 20);
            let chord = 2.0 * (band * (2.0 * radius - band)).sqrt();
            let soft = blur.max(0.5 * ds);
            let mut mesh = Mesh::default();
            for (centre, weight) in stamps {
                let a = stamp_alpha(opacity, 10, weight, chord, ds);
                soft_ring(&mut mesh, centre, radius, band, soft, 32, |c| Color32::from_white_alpha((a * c * 255.0).round() as u8));
            }
            clear_under_head(&mut mesh, at, radius, band, blur, soft);
            let (origin, step, size) = (pos2(100.25, -20.25), 0.5, (260, 82));
            let smudge = rasterise(&mesh, origin, size, step);
            let mut worst = 0.0f32;
            for (k, s) in smudge.iter().enumerate() {
                let p = origin + vec2((k % size.0) as f32, (k / size.0) as f32) * step;
                let head = opacity * ring_cover(p.distance(at), radius, band, blur.max(0.5));
                worst = worst.max(1.0 - (1.0 - head) * (1.0 - s));
            }
            assert!(worst <= opacity + 0.01, "{radius}/{band}/{blur}: {worst} over {opacity}");
            // And the streak behind the head is still there (a ring's along its rails).
            let column = ((at.x - 2.5 * radius - origin.x) / step) as usize;
            let behind = (0..size.1).map(|y| smudge[y * size.0 + column]).fold(0.0f32, f32::max);
            assert!(behind > 0.2 * opacity, "{radius}/{band}/{blur}: streak {behind}");
        }
    }

    /// Draws the voice pointer at `at` at time `now` (a 20-point line); whether it is still settling where it is.
    fn shown_at(ctx: &egui::Context, p: &mut ReadingPointer, pointer: &Pointer, at: Pos2, now: f64) -> bool {
        let colors = Colors::of(&quran_core::settings::Settings::default(), false);
        let mut settling = false;
        let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| {
            let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("p"));
            settling = p.show_at(ctx, layer, pointer, &colors, Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 200.0)), 20.0, Some(2.0), at, 1.0);
        });
        output.textures_delta.clear();
        settling
    }

    #[test]
    fn the_voice_pointer_settles_only_while_its_smudge_draws_back_at_any_frame_rate() {
        let ctx = egui::Context::default();
        let mut pointer = Pointer { enabled: true, size: 8, opacity: 90, ..Pointer::default() };
        // No smudge: moving or not, nothing of its own to settle (it does not shrink with speed).
        let mut p = ReadingPointer::default();
        assert!((1..=40).all(|k| !shown_at(&ctx, &mut p, &pointer, pos2(100.0 + 2.0 * k as f32, 50.0), f64::from(k) / 60.0)));
        // A smudge: settling while it draws back into a head that stays, as long whether the frames come 60 or 240 a
        // second or unevenly, when they are asked for.
        pointer.smudge = 8;
        let mut lasted = Vec::new();
        for frames in [&[1.0 / 60.0][..], &[1.0 / 240.0], &[0.004, 0.03, 0.012, 0.06]] {
            let mut p = ReadingPointer::default();
            let (mut t, mut x) = (0.0, 100.0);
            while t < 0.5 {
                (t, x) = (t + 1.0 / 60.0, x + 2.0);
                assert!(shown_at(&ctx, &mut p, &pointer, pos2(x, 50.0), t) || t < 0.1);
            }
            let (stop, mut k) = (t, 0);
            while shown_at(&ctx, &mut p, &pointer, pos2(x, 50.0), t) {
                (t, k) = (t + frames[k % frames.len()], k + 1);
                assert!(t < stop + 2.0, "settles");
            }
            lasted.push(t - stop);
        }
        assert!(lasted.iter().all(|d| (d - lasted[0]).abs() < 0.07 && *d <= f64::from(SPEED + RETRACT) + 0.08), "{lasted:?}");
    }

    #[test]
    fn a_scroll_moves_the_smudge_by_exactly_the_scroll() {
        let (mut p, at, now) = moved(line(400.0, 0.5), 0.75, 300.0);
        let before = smear(&p.path, at, now, 6.0, 0.75, p.tail);
        assert!(before.len() > 10);
        let by = vec2(0.0, -37.5);
        p.scrolled(by);
        let after = smear(&p.path, at + by, now, 6.0, 0.75, p.tail);
        assert_eq!(after, before.iter().map(|&(c, w)| (c + by, w)).collect::<Vec<_>>());
        // The head kept to its word, carried by the scroll: nothing is added to the path behind it.
        let n = p.path.len();
        p.trace(at + by, now + f64::from(FRAME), FRAME, true, 0.75, 300.0);
        assert!(p.path.len() <= n && p.path.back().is_some_and(|&(q, _)| q == at + by));
    }
}
