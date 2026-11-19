//! Colours: the light, dark and sepia themes of quran-dx's stylesheet, and
//! egui's own widgets dressed in them.

use egui::{Color32, CornerRadius, Stroke};
use quran_core::settings::{CustomTheme, Settings, Theme};

use crate::glyphs::Palette;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub dark: bool,
    pub bg: Color32,
    pub fg: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub card: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub ornament: Color32,
    /// Warning banners (integrity, fonts).
    pub warn_bg: Color32,
    pub warn_fg: Color32,
    pub warn_border: Color32,
    /// Error boxes.
    pub error_bg: Color32,
    pub error_fg: Color32,
    pub error_border: Color32,
    /// Tooltips: dark in every theme.
    pub tip_bg: Color32,
    pub tip_border: Color32,
    /// The palette of the tajweed fonts that goes with the background.
    pub glyphs: Palette,
}

/// Text on a tooltip.
pub const TIP_FG: Color32 = rgb(0xf2f2f2);

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const ACCENT: Color32 = rgb(0x2ca4ab);

const LIGHT: Colors = Colors {
    dark: false,
    bg: rgb(0xfdfcf9),
    fg: rgb(0x1f2328),
    muted: rgb(0x6a737d),
    border: rgb(0xe5e2da),
    card: rgb(0xffffff),
    accent: ACCENT,
    on_accent: Color32::WHITE,
    danger: rgb(0xc0392b),
    ornament: rgb(0xb8860b),
    warn_bg: rgb(0xfff8e1),
    warn_fg: rgb(0x6b5200),
    warn_border: rgb(0xe5c97a),
    error_bg: rgb(0xfff1f1),
    error_fg: rgb(0x8a1f1f),
    error_border: rgb(0xe0a3a3),
    tip_bg: rgb(0x1f2328),
    tip_border: Color32::TRANSPARENT,
    glyphs: Palette::Light,
};

const DARK: Colors = Colors {
    dark: true,
    bg: rgb(0x16181d),
    fg: rgb(0xe6e6e6),
    muted: rgb(0x9aa0a6),
    border: rgb(0x2d3139),
    card: rgb(0x1d2026),
    accent: ACCENT,
    on_accent: Color32::WHITE,
    danger: rgb(0xff8a80),
    ornament: rgb(0xd4b25c),
    warn_bg: rgb(0x2e2814),
    warn_fg: rgb(0xf0dc9c),
    warn_border: rgb(0x6b5a24),
    error_bg: rgb(0x2b1616),
    error_fg: rgb(0xf3b1b1),
    error_border: rgb(0x7f2a2a),
    tip_bg: rgb(0x2a2e37),
    tip_border: rgb(0x414653),
    glyphs: Palette::Dark,
};

/// Sepia: the light colours on warm paper. The tajweed fonts have a palette for it (2).
const SEPIA: Colors = Colors {
    bg: rgb(0xf4ecd8),
    fg: rgb(0x3b2f20),
    muted: rgb(0x7b6a55),
    border: rgb(0xdccfb2),
    card: rgb(0xfbf5e4),
    ornament: rgb(0x96690f),
    tip_bg: rgb(0x3b2f20),
    glyphs: Palette::Index(2),
    ..LIGHT
};

impl Colors {
    /// The colours of the chosen theme; `system_dark` decides for `Auto`.
    pub fn of(settings: &Settings, system_dark: bool) -> Self {
        match settings.theme {
            Theme::Sepia => SEPIA,
            Theme::Custom => Self::custom(&settings.custom_theme),
            _ if settings.dark().unwrap_or(system_dark) => DARK,
            _ => LIGHT,
        }
    }

    /// The custom theme: its colours are worked out from a background and an
    /// accent; what they don't cover (warnings, ornaments, the tajweed palette)
    /// is the light or the dark theme's, whichever the background calls for.
    fn custom(theme: &CustomTheme) -> Self {
        let p = theme.palette();
        let color = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
        Self {
            bg: color(p.bg),
            fg: color(p.fg),
            muted: color(p.muted),
            border: color(p.border),
            card: color(p.card),
            accent: color(p.accent),
            on_accent: color(p.on_accent),
            tip_bg: color(p.tip_bg),
            ..if p.dark { DARK } else { LIGHT }
        }
    }

    /// `color` at `alpha` (0..=1) over nothing.
    pub fn faint(color: Color32, alpha: f32) -> Color32 {
        color.gamma_multiply(alpha)
    }

    pub fn accent_tint(&self) -> Color32 {
        Self::faint(self.accent, 0.12)
    }

    /// Dress egui's stock widgets (text fields, sliders, checkboxes, scroll bars).
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        visuals.panel_fill = self.bg;
        visuals.window_fill = self.card;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.extreme_bg_color = self.card;
        visuals.faint_bg_color = self.card;
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = Self::faint(self.accent, 0.35);
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        visuals.override_text_color = Some(self.fg);
        let radius = CornerRadius::same(8);
        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.fg);
        for (w, stroke) in [
            (&mut widgets.inactive, self.border),
            (&mut widgets.hovered, self.accent),
            (&mut widgets.active, self.accent),
            (&mut widgets.open, self.accent),
        ] {
            w.bg_fill = self.bg;
            w.weak_bg_fill = self.bg;
            w.bg_stroke = Stroke::new(1.0, stroke);
            w.fg_stroke = Stroke::new(1.0, self.fg);
            w.corner_radius = radius;
            w.expansion = 0.0;
        }
        ctx.set_visuals(visuals);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_picks_colours_and_glyph_palette() {
        let mut s = Settings::default();
        assert_eq!(Colors::of(&s, false), LIGHT, "auto follows the system");
        assert_eq!(Colors::of(&s, true), DARK);
        s.theme = Theme::Light;
        assert_eq!(Colors::of(&s, true).glyphs, Palette::Light);
        s.theme = Theme::Dark;
        assert_eq!(Colors::of(&s, false).glyphs, Palette::Dark);
        s.theme = Theme::Sepia;
        let sepia = Colors::of(&s, true);
        assert!(!sepia.dark && sepia.glyphs == Palette::Index(2) && sepia.bg == rgb(0xf4ecd8));
        // The custom theme's default background is dark: light text, the dark tajweed palette.
        s.theme = Theme::Custom;
        let custom = Colors::of(&s, false);
        assert!(custom.dark && custom.glyphs == Palette::Dark);
        assert_eq!((custom.bg, custom.accent), (rgb(0x101a2b), rgb(0x2ca4ab)));
        assert!(custom.fg.r() > 220 && custom.card != custom.bg);
        s.custom_theme.background = "#f3e9d2".into();
        let custom = Colors::of(&s, true);
        assert!(!custom.dark && custom.glyphs == Palette::Light && custom.fg.r() < 60);
        assert_eq!(custom.tip_bg, custom.fg, "tooltips stay dark on a light page");
    }
}
