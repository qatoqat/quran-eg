//! The voice pointer's preview in the settings: a scene of `script` played over and over through the live
//! pointer's own marker on three lines of its own, so a setting is seen at work while it is changed. It sits at the
//! bottom of the panel while the voice pointer's settings are in view (it stays put while they scroll under the
//! slider being dragged, and scrolling cannot smear its dot), plays at the page's pace and asks for frames only
//! while it plays. Sliding in, its bar uncovers the canvas where it stays, so its words never move under the
//! pointer.

use egui::{Rangef, Rect, Ui, UiBuilder, pos2, vec2};
use quran_core::settings::{Settings, Voice};

use super::dock::{self, Action, Text};
use super::marker::{Shown, Wake, frame_dt};
use super::player::{Canvas, Player};
use super::script::{self, Cadence, Scene};
use super::toast::{Click, Note, Toast};
use crate::fonts::FontStore;
use crate::theme::Colors;

/// Heights of the dock and of its folded bar; on a panel shorter than `SMALL_PANEL` it starts folded.
const DOCK: f32 = 224.0;
const BAR: f32 = 30.0;
const SMALL_PANEL: f32 = 560.0;
/// Seconds the dock takes to slide in or out.
const SLIDE: f32 = 0.15;
/// In the dock: the bar's, the canvas's and the timeline's heights, the space between them and at the sides (the
/// rest of the dock's height is under the timeline).
const ROW: f32 = 26.0;
const CANVAS: f32 = 156.0;
const TIMELINE: f32 = 18.0;
const GAP: f32 = 6.0;
const SIDE: f32 = 14.0;
/// Seconds after the last change before the preview goes where the setting acts (a slider being dragged changes
/// it every frame).
const SETTLE: f64 = 0.3;
/// Seconds between frames while nothing moves but the playhead and a countdown.
const STEP: f64 = 0.1;

/// The preview: session state of the settings panel (its listener, scene, folding and pausing stay as the reader
/// left them); what it draws with is let go while the panel is closed.
pub struct Preview {
    player: Player,
    paused: bool,
    /// Shown since the panel opened.
    open: bool,
    /// Folded or unfolded by the reader (`None`: folded on a small panel).
    folded: Option<bool>,
    /// The voice pointer's settings were in view last frame; the panel the dock is at the foot of, its height this
    /// frame (sliding in and out), and whether it is folded.
    in_view: bool,
    panel: Rect,
    height: f32,
    fold: bool,
    /// The voice settings as last seen (`None`: not since the panel opened), and the scene to go to once they have
    /// stopped changing (egui time).
    seen: Option<Voice>,
    due: Option<(f64, Option<Scene>)>,
    /// When the last frame that played was (egui time): the scene's clock follows the time since, however far
    /// apart frames are while nothing moves.
    played: Option<f64>,
    text: Option<Text>,
}

/// The settings the preview shows: all but the listener's folder and stopping at the closing formula.
fn same(a: &Voice, b: &Voice) -> bool {
    (&a.pointer, a.ring, a.lift, &a.range, &a.motion, a.auto_scroll, a.far_wait_ms) == (&b.pointer, b.ring, b.lift, &b.range, &b.motion, b.auto_scroll, b.far_wait_ms)
}

impl Default for Preview {
    fn default() -> Self {
        Preview {
            player: Player::new(Cadence::default()),
            paused: false,
            open: false,
            folded: None,
            in_view: false,
            panel: Rect::NOTHING,
            height: 0.0,
            fold: false,
            seen: None,
            due: None,
            played: None,
            text: None,
        }
    }
}

impl Preview {
    /// The panel closed: the texture goes, and the scene plays from its start when it opens again (unless paused).
    pub fn close(&mut self) {
        if std::mem::take(&mut self.open) {
            self.text = None;
            (self.seen, self.due, self.played) = (None, None, None);
        }
    }

    /// The dock's height this frame at the foot of `panel`: none while the voice pointer's settings are out of
    /// view, sliding in and out.
    pub fn height(&mut self, ctx: &egui::Context, panel: Rect) -> f32 {
        self.fold = self.folded.unwrap_or(panel.height() < SMALL_PANEL);
        let target = match (self.in_view, self.fold) {
            (false, _) => 0.0,
            (true, true) => BAR,
            (true, false) => DOCK,
        };
        self.height = ctx.animate_value_with_time(egui::Id::unique("voice-preview-dock"), target, SLIDE);
        self.panel = panel;
        self.height
    }

    /// Plays and draws the dock where `height` put it. `section` is where the voice pointer's settings lie, from
    /// under their heading, and `top` the top of the panel's scrolling part: they are in view (for the next frame)
    /// while the heading would show over the unfolded dock (so unfolding it never hides them) and some of them
    /// under `top`. The preview's text is the page's (`page_em`) or smaller.
    #[allow(clippy::too_many_arguments)]
    pub fn show(&mut self, ui: &mut Ui, section: Option<Rangef>, top: f32, settings: &Settings, colors: &Colors, fonts: &mut FontStore, page_em: f32) {
        let ctx = ui.ctx().clone();
        let in_view = section.is_some_and(|s| s.min < self.panel.bottom() - DOCK && s.max > top);
        if in_view != self.in_view {
            // The dock slides from the next frame on.
            ctx.request_repaint();
        }
        (self.in_view, self.open) = (in_view, true);
        let voice = &settings.voice;
        self.changes(ctx.input(|i| i.time), voice);
        if self.height < 1.0 {
            return;
        }
        let panel = self.panel;
        let shown = Rect::from_min_max(pos2(panel.left(), panel.bottom() - self.height), panel.max);
        dock::rule(&ui.painter_at(shown), shown, colors);
        let width = panel.width() - 2.0 * SIDE;
        let bar = Rect::from_center_size(pos2(panel.center().x, shown.top() + BAR / 2.0), vec2(width, ROW));
        // The canvas and the timeline where the unfolded dock has them, as much as the bar has uncovered.
        let canvas = Rect::from_min_size(pos2(panel.left() + SIDE, panel.bottom() - DOCK + BAR + GAP), vec2(width, CANVAS));
        let line = Rect::from_min_size(pos2(canvas.left(), canvas.bottom() + GAP), vec2(width, TIMELINE));
        let uncovered = Rect::from_min_max(pos2(panel.left(), shown.top() + BAR), panel.max).intersect(ui.clip_rect());
        let playing = !self.paused && !self.fold && self.in_view;
        let (mut actions, mut scale, mut drawn) = (Vec::new(), None, None);
        if !self.fold || uncovered.is_positive() {
            // The passage at the page's text size or less: the sheet for that size, then the passage on it.
            let words = Rect::from_min_max(canvas.min + vec2(12.0, 6.0), canvas.max - vec2(12.0, 22.0));
            let first = dock::place(words, page_em, settings.word_spacing, self.text.as_ref());
            Text::ensure(&mut self.text, &ctx, settings, colors, fonts, first.em);
            let placed = dock::place(words, page_em, settings.word_spacing, self.text.as_ref());
            scale = Some(placed.em / page_em);
            if uncovered.is_positive() {
                let mut ui = ui.new_child(UiBuilder::new().max_rect(uncovered));
                ui.set_clip_rect(uncovered);
                drawn = Some(self.canvas(&ui, canvas, &placed, placed.em / page_em, playing, settings, colors));
                let share = self.player.time() / self.player.length(voice);
                actions.extend(dock::timeline(&mut ui, line, colors, self.player.scene(), share));
            }
        }
        actions.extend(dock::bar(ui, bar, colors, scale.filter(|_| !self.fold), self.player.cadence(), !self.paused));
        for action in actions {
            self.act(action, voice);
        }
        if playing && self.player.time() >= self.player.end(voice) {
            self.player.restart(self.player.scene(), voice);
        }
        if let Some(drawn) = drawn.filter(|_| playing) {
            self.ask(&ctx, &drawn);
        }
    }

    /// The canvas: the passage, the marks, the pointer over them and a line on what happens.
    #[allow(clippy::too_many_arguments)]
    fn canvas(&mut self, ui: &Ui, canvas: Rect, placed: &dock::Placed, scale: f32, playing: bool, settings: &Settings, colors: &Colors) -> Shown {
        let painter = ui.painter_at(canvas);
        dock::frame(&painter, canvas, colors);
        dock::legend(ui.painter(), canvas, colors);
        // The clock goes by the time since the last frame that played; the pointer moves by egui's steady measure,
        // as on the page (a fade that starts after frames far apart starts then, not most of the way through).
        let now = ui.ctx().input(|i| i.time);
        let (step, dt) = match (playing, self.played) {
            (false, _) => (0.0, 0.0),
            (true, Some(last)) => (((now - last) as f32).clamp(0.0, 0.25), frame_dt(ui.ctx())),
            (true, None) => (frame_dt(ui.ctx()), frame_dt(ui.ctx())),
        };
        self.played = playing.then_some(now);
        self.player.advance(step, &settings.voice);
        dock::draw_text(&painter, placed, self.text.as_ref(), colors);
        dock::draw_marks(&painter, placed, self.player.truth(), self.player.ticks(), colors);
        let on = Canvas { area: painter.clip_rect(), layer: ui.layer_id(), scale, slots: &placed.slots };
        let shown = self.player.frame(ui.ctx(), dt, &on, &settings.voice, colors);
        // The far place's toast, as over the page (its buttons work here too); at the top, clear of the caption.
        if let Some(offer) = shown.offer {
            let toast = Toast::new(ui.ctx(), &Note::of(offer), &[], colors, canvas.width() - 8.0, 0.8);
            match toast.show(ui.ctx(), egui::Id::unique("voice-preview-toast"), canvas, canvas.top() + 4.0, colors) {
                Some(Click::Go) => self.player.choose(true),
                Some(Click::Stay) => self.player.choose(false),
                Some(Click::Dismiss) | None => {}
            }
        }
        if self.player.turned().is_some() {
            dock::turning(&painter, canvas, colors);
        } else {
            dock::subtitle(&painter, canvas.shrink2(vec2(12.0, 6.0)), &self.player.caption(&settings.voice), colors);
        }
        shown
    }

    /// Asks for the next frame while it plays: at once while anything on the canvas moves, else in time for the
    /// listener's next message or the pointer's next change, and for the playhead and a countdown now and then.
    fn ask(&self, ctx: &egui::Context, shown: &Shown) {
        let after = match shown.wake {
            Wake::Now => 0.0,
            _ if !self.player.still() => 0.0,
            Wake::After(secs) => f64::from(secs),
            Wake::Rest => f64::INFINITY,
        };
        ctx.request_repaint_after_secs(after.min(self.player.next_in()).clamp(0.0, STEP) as f32);
    }

    /// A change of the settings goes, once they have stopped changing, to the scene where the change acts (a look
    /// or a glide anywhere the pointer shows, else reading); opened, the scene starts again.
    fn changes(&mut self, now: f64, voice: &Voice) {
        let Some(seen) = &mut self.seen else {
            self.player.restart(self.player.scene(), voice);
            self.seen = Some(voice.clone());
            return;
        };
        if !same(seen, voice) {
            let scene = script::scene_for(seen, voice).or(self.due.and_then(|(_, scene)| scene));
            self.due = Some((now + SETTLE, scene));
            *seen = voice.clone();
        }
        if let Some((at, scene)) = self.due
            && now >= at
            && !self.paused
        {
            self.due = None;
            match scene {
                Some(scene) => self.player.restart(scene, voice),
                None if self.player.dot().is_none() => self.player.restart(Scene::Read, voice),
                None => {}
            }
        }
    }

    fn act(&mut self, action: Action, voice: &Voice) {
        match action {
            Action::Cadence(cadence) if cadence != self.player.cadence() => self.player.hear(cadence),
            Action::Cadence(_) => {}
            Action::Play => self.paused = false,
            Action::Pause => self.paused = true,
            Action::Fold(fold) => self.folded = Some(fold),
            Action::Jump(scene) => {
                self.player.restart(scene, voice);
                self.paused = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use egui::epaint::Shape;
    use egui::{Color32, Pos2};
    use quran_core::settings::{PointerShape, VoiceCurve};

    use super::*;

    fn colors() -> Colors {
        Colors::of(&Settings::default(), false)
    }

    const PANEL: Rect = Rect { min: Pos2 { x: 748.0, y: 0.0 }, max: Pos2 { x: 1100.0, y: 860.0 } };

    /// A preview in a panel at the right of a headless window, `fps` frames a second, the voice pointer a green
    /// disc with a smudge.
    struct Bench {
        ctx: egui::Context,
        preview: Preview,
        settings: Settings,
        fonts: FontStore,
        page_em: f32,
        /// Where the voice pointer's settings are (from under their heading).
        section: Option<Rangef>,
        fps: f64,
        now: f64,
    }

    /// What a frame drew and asked: the green dot (centre, opacity), the tallest stretch of green mesh (the
    /// smudge), the first word's box, the wait before the next frame.
    #[derive(Debug, Clone, Copy)]
    struct Drawn {
        dot: Option<(Pos2, u8)>,
        smudge: f32,
        delay: Duration,
    }

    const IN_VIEW: Rangef = Rangef { min: 300.0, max: 1200.0 };

    impl Bench {
        fn new() -> Self {
            let ctx = egui::Context::default();
            let mut settings = Settings::default();
            let pointer = &mut settings.voice.pointer;
            (pointer.shape, pointer.color, pointer.opacity, pointer.smudge) = (PointerShape::Circle, "#00ff00".into(), 100, 8);
            // No font source: placeholders at the measured widths, nothing fetched.
            let fonts = FontStore::new(&ctx, Vec::new(), None);
            Bench { preview: Preview::default(), ctx, settings, fonts, page_em: 50.0, section: Some(IN_VIEW), fps: 60.0, now: 100.0 }
        }

        /// Runs `n` frames; what the last one drew and asked.
        fn frames(&mut self, n: usize) -> Drawn {
            let mut drawn = None;
            for _ in 0..n {
                drawn = Some(self.frame());
            }
            drawn.unwrap()
        }

        fn frame(&mut self) -> Drawn {
            let input = egui::RawInput { time: Some(self.now), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 860.0))), ..Default::default() };
            let mut output = self.ctx.clone().run_pass(input, |ctx| {
                egui::Area::new(egui::Id::unique("bench")).fixed_pos(PANEL.min).show(ctx, |ui| {
                    self.preview.height(ctx, PANEL);
                    self.preview.show(ui, self.section, 60.0, &self.settings, &colors(), &mut self.fonts, self.page_em);
                });
            });
            output.textures_delta.clear();
            self.now += 1.0 / self.fps;
            let green = |c: Color32| c.g() > 0 && c.r() == 0 && c.b() == 0;
            let mut drawn = Drawn { dot: None, smudge: 0.0, delay: output.viewport_output[&egui::ViewportId::ROOT].repaint_delay };
            for shape in output.shapes.iter().map(|s| &s.shape) {
                match shape {
                    Shape::Circle(c) if green(c.fill) => drawn.dot = Some((c.center, c.fill.g())),
                    Shape::Mesh(m) if m.vertices.iter().any(|v| green(v.color)) => {
                        let ys = m.vertices.iter().filter(|v| green(v.color)).map(|v| v.pos.y);
                        let (lo, hi) = ys.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), y| (lo.min(y), hi.max(y)));
                        drawn.smudge = drawn.smudge.max(hi - lo);
                    }
                    _ => {}
                }
            }
            drawn
        }

        /// Plays to `scene`'s time `t`.
        fn to(&mut self, scene: Scene, t: f64) {
            self.preview.act(Action::Jump(scene), &self.settings.voice);
            while self.preview.player.time() < t {
                self.frame();
            }
        }
    }

    #[test]
    fn settings_change_what_plays_without_starting_it_over() {
        let mut bench = Bench::new();
        bench.frames(2);
        bench.to(Scene::Read, 6.0);
        let (t, x) = (bench.preview.player.time(), bench.preview.player.dot().unwrap().x);
        // Another curve, mid-glide: from where the pointer is, the scene going on.
        bench.settings.voice.motion.pointer.curve = VoiceCurve::Spring;
        bench.frames(1);
        let word = dock::place(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 128.0)), bench.page_em, 2, None).slots[4].1.width();
        assert!((bench.preview.player.dot().unwrap().x - x).abs() < 0.2 * word);
        bench.frames(40);
        assert!(bench.preview.player.time() > t + 0.6, "not started over: {}", bench.preview.player.time());
        // A smaller text: the passage laid out again, the scene going on, the dot on the smaller passage at once.
        let (t, before) = (bench.preview.player.time(), bench.frames(1).dot.unwrap().1);
        bench.page_em = 16.0;
        let drawn = bench.frames(1);
        assert!(bench.preview.player.time() > t && drawn.dot.is_some_and(|(_, alpha)| alpha >= before.min(250) - 5), "{drawn:?}");
        let at = bench.preview.player.dot().unwrap();
        let canvas = PANEL.shrink2(vec2(SIDE, 0.0));
        assert!(at.x > canvas.center().x - 100.0 && at.x < canvas.center().x + 100.0, "on the smaller passage: {at:?}");
        // A wait changed: to the slips, once it has stopped changing, held as long as the wait needs.
        bench.settings.voice.motion.wait_ms = 4000;
        bench.frames(5);
        assert_eq!(bench.preview.player.scene(), Scene::Read);
        bench.frames(20);
        assert_eq!(bench.preview.player.scene(), Scene::Slips);
        assert!(bench.preview.player.time() < 0.3 && bench.preview.player.length(&bench.settings.voice) > 4.0 + 1.6 + 7.0);
    }

    #[test]
    fn a_scene_plays_again_until_another_is_chosen() {
        let mut bench = Bench::new();
        bench.frames(2);
        bench.to(Scene::Hold, 0.5);
        let length = bench.preview.player.length(&bench.settings.voice);
        let mut wrapped = false;
        for _ in 0..((length + 1.0) * 60.0) as usize {
            let before = bench.preview.player.time();
            bench.frame();
            wrapped |= bench.preview.player.time() < before;
            assert_eq!(bench.preview.player.scene(), Scene::Hold);
        }
        assert!(wrapped, "played again from its start");
        // The far scene: until the page has turned and its card has shown.
        bench.to(Scene::Far, 0.0);
        let length = bench.preview.player.length(&bench.settings.voice);
        bench.frames(((length - 0.3) * 60.0) as usize);
        assert!(bench.preview.player.turned().is_some() && bench.preview.player.time() > length - 0.5, "{}", bench.preview.player.time());
        bench.frames(60);
        assert!(bench.preview.player.time() < 1.0 && bench.preview.player.scene() == Scene::Far && bench.preview.player.turned().is_none());
    }

    #[test]
    fn the_dock_comes_up_once_the_voice_heading_would_show_over_it_and_says_so_at_once() {
        let never = Duration::MAX;
        let mut bench = Bench::new();
        // Under where the unfolded dock reaches: not in view, folded or not.
        for folded in [false, true] {
            bench.preview.act(Action::Fold(folded), &bench.settings.voice);
            bench.section = Some(Rangef::new(PANEL.bottom() - DOCK + 10.0, 1500.0));
            assert_eq!(bench.frames(30).delay, never, "folded {folded}");
            assert!(bench.preview.height < 1.0);
        }
        bench.preview.act(Action::Fold(false), &bench.settings.voice);
        // The frame it comes into view asks for the next, which starts sliding the dock in.
        bench.section = Some(Rangef::new(PANEL.bottom() - DOCK - 10.0, 1500.0));
        assert_eq!(bench.frame().delay, Duration::ZERO);
        bench.frames(30);
        assert_eq!(bench.preview.height, DOCK);
        // Scrolled past: the frame it goes out of view asks for the next too, even paused.
        bench.preview.act(Action::Pause, &bench.settings.voice);
        bench.frames(5);
        bench.section = Some(Rangef::new(-900.0, 40.0));
        assert_eq!(bench.frame().delay, Duration::ZERO);
        assert_eq!(bench.frames(30).delay, never);
        assert!(bench.preview.height < 1.0);
    }

    #[test]
    fn it_asks_for_frames_only_while_it_plays_in_view() {
        let never = Duration::MAX;
        let mut bench = Bench::new();
        assert!(bench.frames(30).delay <= Duration::from_secs_f64(STEP + 1e-6), "playing");
        bench.preview.act(Action::Pause, &bench.settings.voice);
        assert_eq!(bench.frames(30).delay, never, "paused");
        bench.preview.act(Action::Play, &bench.settings.voice);
        bench.preview.act(Action::Fold(true), &bench.settings.voice);
        assert_eq!(bench.frames(30).delay, never, "folded");
        bench.preview.act(Action::Fold(false), &bench.settings.voice);
        bench.section = None;
        assert_eq!(bench.frames(30).delay, never, "out of view");
        // Closed and opened again: from the start of its scene, still as the reader left it.
        bench.preview.act(Action::Jump(Scene::Hold), &bench.settings.voice);
        bench.preview.act(Action::Cadence(Cadence::Even), &bench.settings.voice);
        bench.preview.act(Action::Pause, &bench.settings.voice);
        bench.preview.close();
        bench.section = Some(IN_VIEW);
        bench.frames(2);
        let p = &bench.preview;
        assert_eq!((p.player.time(), p.player.scene(), p.player.cadence(), p.paused), (0.0, Scene::Hold, Cadence::Even, true));
    }

    #[test]
    fn with_nothing_moving_it_asks_for_a_frame_a_tenth_of_a_second_on() {
        let mut bench = Bench::new();
        bench.frames(2);
        // The far place not followed yet: the dot hidden, the caret gone, the ticks faded; a frame in time for the
        // next message, or a tenth of a second on.
        let moved = script::Script::of(Scene::Far, &bench.settings.voice).moved().unwrap();
        bench.to(Scene::Far, moved + 3.0);
        let delays: Vec<Duration> = (0..20).map(|_| bench.frame().delay).collect();
        let step = Duration::from_secs_f64(STEP + 1e-6);
        assert!(delays.iter().all(|d| *d > Duration::ZERO && *d <= step) && delays.iter().any(|d| *d > step / 2), "{delays:?}");
        // Reading: every frame.
        bench.to(Scene::Read, 5.0);
        assert_eq!(bench.frames(1).delay, Duration::ZERO);
    }

    #[test]
    fn the_scene_keeps_time_however_far_apart_frames_are() {
        // Frames come when asked (at most 60 a second): far apart while nothing moves, as in eframe.
        let mut bench = Bench::new();
        bench.frames(20);
        for (scene, from) in [(Scene::Read, 0.0), (Scene::Far, 6.0)] {
            bench.to(scene, from);
            let (start, mut spaced, mut frames) = (bench.now, 0, Vec::new());
            while bench.now < start + 1.5 {
                let at = bench.now;
                let delay = bench.frame().delay;
                frames.push((at, bench.preview.player.time()));
                bench.now += delay.as_secs_f64().clamp(1.0 / 60.0, 1.0) - 1.0 / 60.0;
                spaced += usize::from(delay > Duration::from_millis(30));
            }
            let ((a, t), (b, u)) = (frames[0], frames[frames.len() - 1]);
            assert!(spaced > 2 && ((u - t) - (b - a)).abs() < 1e-3, "{scene:?}: {} s played in {} s, {spaced} frames far apart", u - t, b - a);
        }
    }

    #[test]
    fn sliding_in_or_out_never_moves_the_words_under_the_pointer() {
        for fps in [60.0, 144.0] {
            for paused in [true, false] {
                let mut bench = Bench::new();
                bench.fps = fps;
                bench.frames(2);
                bench.to(Scene::Read, 6.0);
                let reading = (0..(0.5 * fps) as usize).map(|_| bench.frame().smudge).fold(0.0, f32::max);
                if paused {
                    bench.preview.act(Action::Pause, &bench.settings.voice);
                }
                let before = bench.frames(2);
                let (at, alpha) = before.dot.expect("a dot on the line");
                // Folded and unfolded, then scrolled away and back.
                let mut seen = Vec::new();
                bench.preview.act(Action::Fold(true), &bench.settings.voice);
                seen.extend((0..(0.3 * fps) as usize).map(|_| bench.frame()));
                bench.preview.act(Action::Fold(false), &bench.settings.voice);
                seen.extend((0..(0.3 * fps) as usize).map(|_| bench.frame()));
                bench.section = None;
                seen.extend((0..(0.3 * fps) as usize).map(|_| bench.frame()));
                bench.section = Some(IN_VIEW);
                seen.extend((0..(0.3 * fps) as usize).map(|_| bench.frame()));
                let last = *seen.last().unwrap();
                for d in seen.iter().filter_map(|d| d.dot.map(|dot| (dot, d.smudge))) {
                    let ((p, a), smudge) = d;
                    assert!((p.y - at.y).abs() < 0.5, "{fps} paused {paused}: the dot moved off its line to {p:?} from {at:?}");
                    assert!(smudge <= reading + 1.0, "{fps} paused {paused}: a smudge {smudge} tall, {reading} reading");
                    if paused {
                        assert!(p == at && a == alpha, "{fps}: paused, the dot changed to {p:?} {a} from {at:?} {alpha}");
                    }
                }
                let (p, a) = last.dot.expect("shown again");
                assert!(a >= alpha.min(250) - 5 && (!paused || p == at), "{fps} paused {paused}: {a} from {alpha}");
            }
        }
    }
}
