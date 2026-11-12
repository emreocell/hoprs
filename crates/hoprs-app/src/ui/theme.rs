//! The mockup v2 theme: palette tokens, the iced [`Theme`] and the widget
//! styles. Neutral near-black surfaces, a white accent, 4 px corners,
//! 1 px rules and no shadows.

// The full style set lands now; the views that use each one (tabs,
// file rows, dialogs) arrive in later tasks.
#![allow(dead_code)]

use std::sync::LazyLock;

use iced::border::Radius;
use iced::widget::overlay::menu;
use iced::widget::{
    button, checkbox as iced_checkbox, container, pick_list as iced_pick_list, progress_bar, rule,
    scrollable, text_input,
};
use iced::{Background, Border, Color, Font, Shadow, Theme, font};

/// Corner radius of every rounded surface.
pub const RADIUS: f32 = 4.0;
/// Width of every rule and border.
pub const HAIRLINE: f32 = 1.0;

/// Inter, the UI font and the application default.
pub const INTER: Font = Font::with_name("Inter");
pub const INTER_MEDIUM: Font = Font {
    weight: font::Weight::Medium,
    ..INTER
};
pub const INTER_SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..INTER
};
/// JetBrains Mono, for data: file names, paths, permissions, `user@host`
/// and protocol tags.
pub const MONO: Font = Font::with_name("JetBrains Mono");
pub const MONO_MEDIUM: Font = Font {
    weight: font::Weight::Medium,
    ..MONO
};

/// The embedded font files (SIL OFL 1.1, texts next to them).
pub const FONTS: [&[u8]; 5] = [
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf"),
];

/// Type scale (mockup: 0.75rem, 0.8125rem, 0.9375rem, mono 0.78rem,
/// tags 0.7rem).
pub const TEXT_XS: f32 = 12.0;
pub const TEXT_SM: f32 = 13.0;
pub const TEXT_MD: f32 = 15.0;
pub const TEXT_MONO: f32 = 12.5;
pub const TEXT_TAG: f32 = 11.0;

/// Spacing scale (mockup `--space-*`).
pub const SPACE_3XS: f32 = 2.0;
pub const SPACE_2XS: f32 = 4.0;
pub const SPACE_XS: f32 = 8.0;
pub const SPACE_SM: f32 = 12.0;
pub const SPACE_MD: f32 = 16.0;
pub const SPACE_LG: f32 = 24.0;

/// The mockup v2 color tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Window ground.
    pub paper: Color,
    /// Sidebar, status bar, hovered rows.
    pub paper_2: Color,
    /// Active tab, hovered sidebar items.
    pub paper_3: Color,
    pub ink: Color,
    pub ink_2: Color,
    pub muted: Color,
    pub rule: Color,
    pub rule_2: Color,
    /// Selection, primary action, active tab, focus and progress only.
    pub accent: Color,
    /// Text on the accent.
    pub accent_ink: Color,
    /// Selected rows.
    pub accent_wash: Color,
    pub error: Color,
}

impl Palette {
    /// The tokens of mockup v2, from their oklch values.
    pub fn v2() -> Self {
        let grey = |lightness| oklch_to_rgb(lightness, 0.0, 0.0);
        Self {
            paper: grey(0.13),
            paper_2: grey(0.165),
            paper_3: grey(0.21),
            ink: grey(0.97),
            ink_2: grey(0.84),
            muted: grey(0.63),
            rule: grey(0.25),
            rule_2: grey(0.34),
            accent: grey(0.98),
            accent_ink: grey(0.14),
            accent_wash: grey(0.24),
            error: oklch_to_rgb(0.70, 0.14, 25.0),
        }
    }
}

static PALETTE: LazyLock<Palette> = LazyLock::new(Palette::v2);

/// The application palette.
pub fn palette() -> &'static Palette {
    &PALETTE
}

/// Converts oklch (lightness 0–1, chroma, hue in degrees) to sRGB through
/// OKLab and linear sRGB. Out-of-gamut channels are clamped.
pub fn oklch_to_rgb(l: f32, c: f32, h_deg: f32) -> Color {
    let (l, c, h) = (f64::from(l), f64::from(c), f64::from(h_deg).to_radians());
    let (a, b) = (c * h.cos(), c * h.sin());

    // OKLab -> LMS (cube roots) -> LMS -> linear sRGB (Ottosson's matrices).
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (lms_l, lms_m, lms_s) = (l_.powi(3), m_.powi(3), s_.powi(3));

    let r = 4.076_741_662_1 * lms_l - 3.307_711_591_3 * lms_m + 0.230_969_929_2 * lms_s;
    let g = -1.268_438_004_6 * lms_l + 2.609_757_401_1 * lms_m - 0.341_319_396_5 * lms_s;
    let b = -0.004_196_086_3 * lms_l - 0.703_418_614_7 * lms_m + 1.707_614_701_0 * lms_s;

    Color::from_rgb(srgb_gamma(r), srgb_gamma(g), srgb_gamma(b))
}

/// Linear -> gamma-encoded sRGB, clamped to 0–1.
fn srgb_gamma(linear: f64) -> f32 {
    let x = linear.clamp(0.0, 1.0);
    let encoded = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    encoded as f32
}

/// The iced theme built on the palette.
pub fn theme() -> Theme {
    let p = palette();
    Theme::custom(
        "hoprs",
        iced::theme::Palette {
            background: p.paper,
            text: p.ink,
            primary: p.accent,
            success: p.ink,
            warning: p.error,
            danger: p.error,
        },
    )
}

fn radius() -> Radius {
    Radius::from(RADIUS)
}

fn flat_button(background: Option<Color>, text_color: Color) -> button::Style {
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: Border {
            radius: radius(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// The main action: white fill, near-black text.
pub fn button_primary(_: &Theme, status: button::Status) -> button::Style {
    let p = palette();
    match status {
        button::Status::Active => flat_button(Some(p.accent), p.accent_ink),
        button::Status::Hovered => flat_button(Some(p.ink_2), p.accent_ink),
        button::Status::Pressed => flat_button(Some(p.muted), p.accent_ink),
        button::Status::Disabled => flat_button(None, p.rule_2),
    }
}

/// A text-only button: muted, ink on hover.
pub fn button_text(_: &Theme, status: button::Status) -> button::Style {
    let p = palette();
    match status {
        button::Status::Active => flat_button(None, p.muted),
        button::Status::Hovered | button::Status::Pressed => flat_button(None, p.ink),
        button::Status::Disabled => flat_button(None, p.rule_2),
    }
}

/// The outlined "Go to anything" button in the top bar.
pub fn button_outline(_: &Theme, status: button::Status) -> button::Style {
    let p = palette();
    let (text_color, border) = match status {
        button::Status::Active => (p.muted, p.rule),
        button::Status::Hovered | button::Status::Pressed => (p.ink, p.rule_2),
        button::Status::Disabled => (p.rule_2, p.rule),
    };
    button::Style {
        border: Border {
            color: border,
            width: HAIRLINE,
            radius: radius(),
        },
        ..flat_button(None, text_color)
    }
}

/// A session tab in the top bar; the active one also gets the accent
/// underline from [`underline`].
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let p = palette();
        let (background, text_color) = match (active, status) {
            (true, _) => (Some(p.paper_3), p.ink),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                (Some(p.paper_2), p.ink_2)
            }
            (false, _) => (None, p.muted),
        };
        button::Style {
            border: Border {
                radius: Radius::default().top(RADIUS),
                ..Border::default()
            },
            ..flat_button(background, text_color)
        }
    }
}

/// A queue filter tab (Active, Queued, Failed, Done): square, text only.
pub fn queue_tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let p = palette();
        let text_color = match (active, status) {
            (true, _) | (_, button::Status::Hovered | button::Status::Pressed) => p.ink,
            _ => p.muted,
        };
        button::Style {
            border: Border::default(),
            ..flat_button(None, text_color)
        }
    }
}

/// A list row (sidebar item, file row).
pub fn row(_: &Theme, status: button::Status) -> button::Style {
    let p = palette();
    match status {
        button::Status::Hovered | button::Status::Pressed => flat_button(Some(p.paper_3), p.ink),
        button::Status::Active => flat_button(None, p.ink_2),
        button::Status::Disabled => flat_button(None, p.muted),
    }
}

/// A selected list row: the accent wash.
pub fn row_selected(_: &Theme, _: button::Status) -> button::Style {
    let p = palette();
    flat_button(Some(p.accent_wash), p.ink)
}

fn hairline(color: Color) -> rule::Style {
    rule::Style {
        color,
        radius: Radius::default(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

/// A 1 px rule in the `rule` token.
pub fn rule_1px(_: &Theme) -> rule::Style {
    hairline(palette().rule)
}

/// A 1 px rule in the stronger `rule_2` token (above the queue).
pub fn rule_strong(_: &Theme) -> rule::Style {
    hairline(palette().rule_2)
}

/// The 2 px accent line under the active tab.
pub fn underline(_: &Theme) -> container::Style {
    surface(palette().accent)
}

fn surface(background: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(background)),
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

/// The window ground.
pub fn paper(_: &Theme) -> container::Style {
    surface(palette().paper)
}

/// The sidebar surface.
pub fn sidebar(_: &Theme) -> container::Style {
    surface(palette().paper_2)
}

/// The status bar: second surface, muted text.
pub fn status_bar(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(palette().muted),
        ..surface(palette().paper_2)
    }
}

/// A 1 px outlined box (the palette slot in the top bar).
pub fn outline_box(_: &Theme) -> container::Style {
    let p = palette();
    container::Style {
        text_color: Some(p.muted),
        border: Border {
            color: p.rule,
            width: HAIRLINE,
            radius: radius(),
        },
        ..container::Style::default()
    }
}

/// Behind a dialog: the window dimmed (no blur, no shadow).
pub fn backdrop(_: &Theme) -> container::Style {
    let paper = palette().paper;
    container::Style {
        background: Some(Background::Color(Color { a: 0.72, ..paper })),
        ..container::Style::default()
    }
}

/// A dialog: the second surface, a 1 px `rule_2` outline, 4 px corners, no shadow.
pub fn dialog_box(_: &Theme) -> container::Style {
    let p = palette();
    container::Style {
        text_color: Some(p.ink),
        border: Border {
            color: p.rule_2,
            width: HAIRLINE,
            radius: radius(),
        },
        ..surface(p.paper_2)
    }
}

/// A key cap, e.g. "Ctrl K".
pub fn kbd(_: &Theme) -> container::Style {
    let p = palette();
    container::Style {
        text_color: Some(p.ink_2),
        border: Border {
            color: p.rule_2,
            width: HAIRLINE,
            radius: Radius::from(3.0),
        },
        ..container::Style::default()
    }
}

/// The sidebar filter field.
pub fn search(_: &Theme, status: text_input::Status) -> text_input::Style {
    let p = palette();
    let border = match status {
        text_input::Status::Focused { .. } => p.accent,
        _ => p.rule_2,
    };
    text_input::Style {
        background: Background::Color(p.paper),
        border: Border {
            color: border,
            width: HAIRLINE,
            radius: radius(),
        },
        icon: p.muted,
        placeholder: p.muted,
        value: p.ink,
        selection: p.rule_2,
    }
}

/// A loading placeholder bar: `paper_3`, rounded.
pub fn skeleton(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: radius(),
            ..Border::default()
        },
        ..surface(palette().paper_3)
    }
}

/// A breadcrumb segment: `ink_2`, ink on hover, no fill.
pub fn crumb(_: &Theme, status: button::Status) -> button::Style {
    let p = palette();
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => p.ink,
        button::Status::Active | button::Status::Disabled => p.ink_2,
    };
    button::Style {
        border: Border::default(),
        ..flat_button(None, text_color)
    }
}

/// A sortable column header (mockup `th`): muted on the ground, ink on hover.
pub fn header_cell(_: &Theme, status: button::Status) -> button::Style {
    let p = palette();
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => p.ink,
        button::Status::Active | button::Status::Disabled => p.muted,
    };
    button::Style {
        border: Border::default(),
        ..flat_button(None, text_color)
    }
}

/// A file row: hovered on `paper_2`, selected on the accent wash (mockup `tbody tr`).
/// The keyboard cursor of the focused pane gets a 1 px `muted` outline, which shows on
/// selected and plain rows alike.
pub fn file_row(
    selected: bool,
    hovered: bool,
    cursor: bool,
) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let p = palette();
        let (background, text_color) = match (selected, hovered) {
            (true, _) => (Some(p.accent_wash), p.ink),
            (false, true) => (Some(p.paper_2), p.ink),
            (false, false) => (None, p.ink_2),
        };
        container::Style {
            text_color: Some(text_color),
            background: background.map(Background::Color),
            border: if cursor {
                Border {
                    color: p.muted,
                    width: HAIRLINE,
                    radius: Radius::default(),
                }
            } else {
                Border::default()
            },
            ..container::Style::default()
        }
    }
}

/// The 1 px line on top of a pane: the accent on the focused pane, the ground on the
/// other (so both keep the same height).
pub fn pane_focus(focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let p = palette();
        surface(if focused { p.accent } else { p.paper })
    }
}

/// The thin bars above a pane's table (error, confirmation, permissions): `paper_2`.
pub fn pane_bar(_: &Theme) -> container::Style {
    container::Style {
        text_color: Some(palette().ink_2),
        ..surface(palette().paper_2)
    }
}

/// The scrollbar of the file lists: a `rule_2` thumb on the ground, no rail.
pub fn scrollbar(_: &Theme, status: scrollable::Status) -> scrollable::Style {
    let p = palette();
    let thumb = match status {
        scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered: true,
            ..
        }
        | scrollable::Status::Dragged {
            is_vertical_scrollbar_dragged: true,
            ..
        } => p.muted,
        _ => p.rule_2,
    };
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: Background::Color(thumb),
            border: Border {
                radius: radius(),
                ..Border::default()
            },
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(p.paper_3),
            border: Border {
                color: p.rule_2,
                width: HAIRLINE,
                radius: radius(),
            },
            shadow: Shadow::default(),
            icon: p.ink_2,
        },
    }
}

/// A drop-down (protocol, open as, transfer mode): like the text fields.
pub fn pick_list(_: &Theme, status: iced_pick_list::Status) -> iced_pick_list::Style {
    let p = palette();
    let border = match status {
        iced_pick_list::Status::Active => p.rule_2,
        iced_pick_list::Status::Hovered => p.muted,
        iced_pick_list::Status::Opened { .. } => p.accent,
    };
    iced_pick_list::Style {
        text_color: p.ink,
        placeholder_color: p.muted,
        handle_color: p.muted,
        background: Background::Color(p.paper),
        border: Border {
            color: border,
            width: HAIRLINE,
            radius: radius(),
        },
    }
}

/// The open list of a drop-down: `paper_3`, 1 px rule, no shadow.
pub fn menu(_: &Theme) -> menu::Style {
    let p = palette();
    menu::Style {
        background: Background::Color(p.paper_3),
        border: Border {
            color: p.rule_2,
            width: HAIRLINE,
            radius: radius(),
        },
        text_color: p.ink_2,
        selected_text_color: p.ink,
        selected_background: Background::Color(p.accent_wash),
        shadow: Shadow::default(),
    }
}

/// A queue row's action (mockup `.act`): muted, ink on `paper_3` when hovered.
pub fn queue_action(_: &Theme, status: button::Status) -> button::Style {
    let p = palette();
    match status {
        button::Status::Active => flat_button(None, p.muted),
        button::Status::Hovered | button::Status::Pressed => flat_button(Some(p.paper_3), p.ink),
        button::Status::Disabled => flat_button(None, p.rule_2),
    }
}

/// The queue's 4 px progress bar: the accent over a `rule` track (mockup `.track`,
/// `.fill`).
pub fn progress(_: &Theme) -> progress_bar::Style {
    let p = palette();
    progress_bar::Style {
        background: Background::Color(p.rule),
        bar: Background::Color(p.accent),
        border: Border {
            radius: Radius::from(2.0),
            ..Border::default()
        },
    }
}

/// The pane a drag (or files from the system) would drop on: a 1 px accent outline.
pub fn drop_target(active: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        border: if active {
            Border {
                color: palette().accent,
                width: HAIRLINE,
                radius: radius(),
            }
        } else {
            Border::default()
        },
        ..container::Style::default()
    }
}

/// A checkbox: white when checked (the accent marks selection).
pub fn checkbox(_: &Theme, status: iced_checkbox::Status) -> iced_checkbox::Style {
    let p = palette();
    let (checked, hovered, disabled) = match status {
        iced_checkbox::Status::Active { is_checked } => (is_checked, false, false),
        iced_checkbox::Status::Hovered { is_checked } => (is_checked, true, false),
        iced_checkbox::Status::Disabled { is_checked } => (is_checked, false, true),
    };
    let border = match (checked, hovered) {
        (true, _) => p.accent,
        (false, true) => p.muted,
        (false, false) => p.rule_2,
    };
    iced_checkbox::Style {
        background: Background::Color(if checked { p.accent } else { p.paper }),
        icon_color: p.accent_ink,
        border: Border {
            color: border,
            width: HAIRLINE,
            radius: radius(),
        },
        text_color: Some(if disabled { p.muted } else { p.ink_2 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_u8(channel: f32) -> i32 {
        (channel * 255.0).round() as i32
    }

    fn assert_close(color: Color, hex: [u8; 3]) {
        let got = [to_u8(color.r), to_u8(color.g), to_u8(color.b)];
        for (got, want) in got.iter().zip(hex) {
            assert!(
                (got - i32::from(want)).abs() <= 2,
                "got {got:?}, want {hex:?} (±2/255)"
            );
        }
    }

    #[test]
    fn oklch_conversion() {
        assert_close(oklch_to_rgb(0.0, 0.0, 0.0), [0, 0, 0]);
        assert_close(oklch_to_rgb(1.0, 0.0, 0.0), [255, 255, 255]);
        // Hue is irrelevant without chroma.
        assert_close(oklch_to_rgb(1.0, 0.0, 200.0), [255, 255, 255]);
        // oklch(70% 0.14 25) = #e97871 (232.5, 120.4, 113.3), from the
        // reference OKLab -> linear sRGB matrices (Ottosson); the plan's
        // #e4776a was an estimate and is off by 4-7/255.
        assert_close(oklch_to_rgb(0.70, 0.14, 25.0), [0xe9, 0x78, 0x71]);
        // oklch(13% 0 0), the window ground, = #070707.
        assert_close(oklch_to_rgb(0.13, 0.0, 0.0), [0x07, 0x07, 0x07]);
        assert_eq!(oklch_to_rgb(0.5, 0.0, 0.0).a, 1.0);
    }

    #[test]
    fn palette_tokens_are_achromatic_except_error() {
        let p = Palette::v2();
        let greys = [
            ("paper", p.paper),
            ("paper_2", p.paper_2),
            ("paper_3", p.paper_3),
            ("ink", p.ink),
            ("ink_2", p.ink_2),
            ("muted", p.muted),
            ("rule", p.rule),
            ("rule_2", p.rule_2),
            ("accent", p.accent),
            ("accent_ink", p.accent_ink),
            ("accent_wash", p.accent_wash),
        ];
        for (name, c) in greys {
            assert!(
                (c.r - c.g).abs() < 1e-4 && (c.g - c.b).abs() < 1e-4,
                "{name} is not grey: {c:?}"
            );
        }
        // The surfaces step up and the ink sits far above them.
        assert!(p.paper.r < p.paper_2.r && p.paper_2.r < p.paper_3.r);
        assert!(p.rule.r < p.rule_2.r && p.muted.r < p.ink_2.r);
        assert!(p.ink_2.r < p.ink.r && p.ink.r < p.accent.r);
        // The one chromatic token: a warm red.
        assert!(p.error.r > p.error.g + 0.3 && p.error.r > p.error.b + 0.3);
        assert_eq!(p, *palette());
    }
}
