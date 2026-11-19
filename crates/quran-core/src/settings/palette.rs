//! The custom theme's colours: everything is worked out from its background
//! and its accent, by the rules of quran-dx's stylesheet (`color-mix(in oklab, …)`).

use super::{CustomTheme, luminance};

pub type Rgb = [u8; 3];

/// `#rrggbb` → its three channels.
pub fn parse_hex(color: &str) -> Option<Rgb> {
    let hex = super::sanitize_color(color)?;
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([channel(1)?, channel(3)?, channel(5)?])
}

fn to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

fn to_oklab(rgb: Rgb) -> [f32; 3] {
    let [r, g, b] = rgb.map(to_linear);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab([l, a, b]: [f32; 3]) -> Rgb {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        from_linear(4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_),
        from_linear(-1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_),
        from_linear(-0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_),
    ]
}

/// `color-mix(in oklab, a share, b)`: `share` (0..=1) of `a`, the rest `b`.
pub fn mix(a: Rgb, share: f32, b: Rgb) -> Rgb {
    let (a, b) = (to_oklab(a), to_oklab(b));
    from_oklab([0, 1, 2].map(|i| a[i] * share + b[i] * (1.0 - share)))
}

const WHITE: Rgb = [255; 3];
const BLACK: Rgb = [0; 3];

/// The colours of a custom theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomPalette {
    /// White text reads better on the background than black.
    pub dark: bool,
    pub bg: Rgb,
    pub fg: Rgb,
    pub card: Rgb,
    pub border: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub on_accent: Rgb,
    /// Background of tooltips (dark in every theme).
    pub tip_bg: Rgb,
}

impl CustomTheme {
    /// The whole palette from the two colours: text is near white or near black
    /// (whichever contrasts with the background) with a touch of the background
    /// in it; cards, borders and muted text are mixes of the text and the
    /// background; text on the accent is black or white by the accent's lightness.
    pub fn palette(&self) -> CustomPalette {
        let bg = parse_hex(&self.background()).unwrap_or([0x10, 0x1a, 0x2b]);
        let accent = parse_hex(&self.accent()).unwrap_or([0x2c, 0xa4, 0xab]);
        let dark = self.is_dark();
        let (ink, lift) = if dark { (WHITE, 0.06) } else { (BLACK, 0.45) };
        let fg = mix(bg, 0.10, ink);
        CustomPalette {
            dark,
            bg,
            fg,
            card: mix(bg, 1.0 - lift, WHITE),
            border: mix(fg, 0.15, bg),
            muted: mix(fg, 0.60, bg),
            accent,
            on_accent: if luminance(&self.accent()).unwrap_or(0.0) < 0.35 { WHITE } else { [0x11; 3] },
            tip_bg: if dark { mix(bg, 0.82, WHITE) } else { fg },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_mixing() {
        // Half black, half white is the perceptual middle grey, not #808080.
        assert_eq!(mix(BLACK, 0.5, WHITE), [0x63; 3]);
        assert_eq!(mix([12, 200, 99], 1.0, WHITE), [12, 200, 99]);
        assert_eq!(mix([12, 200, 99], 0.0, [1, 2, 3]), [1, 2, 3]);
        // Round trip through oklab.
        for c in [[0, 0, 0], [255, 255, 255], [16, 26, 43], [44, 164, 171], [255, 0, 0]] {
            assert_eq!(from_oklab(to_oklab(c)), c);
        }
        assert_eq!((parse_hex("#101A2b"), parse_hex("101a2b"), parse_hex("#fff")), (Some([16, 26, 43]), None, None));
    }

    #[test]
    fn dark_background_gets_light_text() {
        let p = CustomTheme::default().palette();
        assert!(p.dark);
        assert_eq!((p.bg, p.accent, p.on_accent), ([0x10, 0x1a, 0x2b], [0x2c, 0xa4, 0xab], WHITE));
        // Text is nearly white, the card a little lighter than the background,
        // and border < muted < text in lightness.
        let light = |c: Rgb| to_oklab(c)[0];
        assert!(light(p.fg) > 0.9 && p.fg != WHITE);
        assert!(light(p.card) > light(p.bg) && light(p.card) < light(p.bg) + 0.1);
        assert!(light(p.bg) < light(p.border) && light(p.border) < light(p.muted) && light(p.muted) < light(p.fg));
        assert!(light(p.tip_bg) > light(p.bg));
    }

    #[test]
    fn light_background_gets_dark_text() {
        let theme = CustomTheme { background: "#F3E9D2".into(), accent: "#1b4f72".into() };
        let p = theme.palette();
        let light = |c: Rgb| to_oklab(c)[0];
        assert!(!p.dark && light(p.fg) < 0.3);
        assert_eq!((p.on_accent, p.tip_bg), (WHITE, p.fg), "a dark accent takes white text; tooltips stay dark");
        assert!(light(p.card) > light(p.bg));
        // A light accent takes dark text.
        let p = CustomTheme { accent: "#ffd54a".into(), ..theme }.palette();
        assert_eq!(p.on_accent, [0x11; 3]);
        // Bad colours fall back to the defaults.
        let p = CustomTheme { background: "url(x)".into(), accent: "} *{".into() }.palette();
        assert_eq!((p.bg, p.accent), ([0x10, 0x1a, 0x2b], [0x2c, 0xa4, 0xab]));
    }
}
