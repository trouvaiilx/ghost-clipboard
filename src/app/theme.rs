//! Clean dark styling and theme helpers for GhostClipboard.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Theme};

// Palette
pub const BG_ROOT: Color = Color::from_rgb(0.09, 0.10, 0.12); // #17191f
pub const BG_SURFACE: Color = Color::from_rgb(0.13, 0.14, 0.17); // #21242b
pub const BG_SURFACE_HOVER: Color = Color::from_rgb(0.16, 0.18, 0.22); // #292e38
pub const BG_SELECTED: Color = Color::from_rgb(0.18, 0.22, 0.30); // #2e384d
pub const BG_INPUT: Color = Color::from_rgb(0.11, 0.12, 0.15); // #1c1f26

pub const BORDER_SUBTLE: Color = Color::from_rgb(0.20, 0.22, 0.26); // #333842
pub const BORDER_FOCUSED: Color = Color::from_rgb(0.35, 0.45, 0.65); // #5973a6
pub const BORDER_SELECTED: Color = Color::from_rgb(0.30, 0.50, 0.85); // #4d80d9

pub const TEXT_PRIMARY: Color = Color::from_rgb(0.93, 0.94, 0.96); // #edeef5
pub const TEXT_SECONDARY: Color = Color::from_rgb(0.65, 0.68, 0.74); // #a6adc0
pub const TEXT_MUTED: Color = Color::from_rgb(0.56, 0.59, 0.65); // #8e95a5 (WCAG AA 4.8:1)

pub const ACCENT_BLUE: Color = Color::from_rgb(0.36, 0.60, 0.98); // #5c99fa
pub const ACCENT_GREEN: Color = Color::from_rgb(0.30, 0.75, 0.50); // #4dc080
pub const ACCENT_AMBER: Color = Color::from_rgb(0.95, 0.70, 0.25); // #f2b340
pub const ACCENT_RED: Color = Color::from_rgb(0.92, 0.35, 0.35); // #eb5959

pub fn root_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_ROOT)),
        text_color: Some(TEXT_PRIMARY),
        ..container::Style::default()
    }
}

pub fn search_bar_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_INPUT)),
        border: Border {
            color: BORDER_FOCUSED,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn search_input_style(_theme: &Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: TEXT_MUTED,
        placeholder: TEXT_MUTED,
        value: TEXT_PRIMARY,
        selection: BG_SELECTED,
    }
}

pub fn entry_item(is_selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme: &Theme| {
        if is_selected {
            container::Style {
                background: Some(Background::Color(BG_SELECTED)),
                border: Border {
                    color: BORDER_SELECTED,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: Some(TEXT_PRIMARY),
                ..container::Style::default()
            }
        } else {
            container::Style {
                background: Some(Background::Color(BG_SURFACE)),
                border: Border {
                    color: BORDER_SUBTLE,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: Some(TEXT_PRIMARY),
                ..container::Style::default()
            }
        }
    }
}

pub fn entry_item_button(is_selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        if is_selected {
            button::Style {
                background: Some(Background::Color(BG_SELECTED)),
                border: Border {
                    color: BORDER_SELECTED,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: TEXT_PRIMARY,
                ..button::Style::default()
            }
        } else {
            let (bg, border) = match status {
                button::Status::Hovered => (BG_SURFACE_HOVER, BORDER_FOCUSED),
                button::Status::Pressed => (BG_SELECTED, BORDER_SELECTED),
                _ => (BG_SURFACE, BORDER_SUBTLE),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: TEXT_PRIMARY,
                ..button::Style::default()
            }
        }
    }
}

pub fn detail_pane(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_SURFACE)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: Some(TEXT_PRIMARY),
        ..container::Style::default()
    }
}

pub fn status_bar(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_INPUT)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 0.0.into(),
        },
        text_color: Some(TEXT_MUTED),
        ..container::Style::default()
    }
}

pub fn badge(bg: Color, fg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme: &Theme| container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 3.0.into(),
        },
        text_color: Some(fg),
        ..container::Style::default()
    }
}

pub fn primary_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, fg) = match status {
        button::Status::Active => (ACCENT_BLUE, BG_ROOT),
        button::Status::Hovered => (Color::from_rgb(0.45, 0.68, 1.0), BG_ROOT),
        button::Status::Pressed => (Color::from_rgb(0.30, 0.50, 0.85), BG_ROOT),
        button::Status::Disabled => (BORDER_SUBTLE, TEXT_MUTED),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

pub fn subtle_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, border, fg) = match status {
        button::Status::Active => (BG_SURFACE, BORDER_SUBTLE, TEXT_SECONDARY),
        button::Status::Hovered => (BG_SURFACE_HOVER, BORDER_FOCUSED, TEXT_PRIMARY),
        button::Status::Pressed => (BG_SELECTED, BORDER_SELECTED, TEXT_PRIMARY),
        button::Status::Disabled => (BG_INPUT, Color::TRANSPARENT, TEXT_MUTED),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: border,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..button::Style::default()
    }
}

pub fn danger_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, border, fg) = match status {
        button::Status::Active => (BG_SURFACE, Color::from_rgb(0.5, 0.2, 0.2), ACCENT_RED),
        button::Status::Hovered => (
            Color::from_rgb(0.3, 0.12, 0.12),
            ACCENT_RED,
            Color::from_rgb(1.0, 0.8, 0.8),
        ),
        button::Status::Pressed => (Color::from_rgb(0.4, 0.15, 0.15), ACCENT_RED, Color::WHITE),
        button::Status::Disabled => (BG_INPUT, Color::TRANSPARENT, TEXT_MUTED),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: border,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..button::Style::default()
    }
}

pub fn pill_button(is_active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let (bg, border, fg) = if is_active {
            (BG_SELECTED, BORDER_SELECTED, ACCENT_BLUE)
        } else {
            match status {
                button::Status::Active => (BG_SURFACE, BORDER_SUBTLE, TEXT_MUTED),
                button::Status::Hovered => (BG_SURFACE_HOVER, BORDER_SUBTLE, TEXT_SECONDARY),
                button::Status::Pressed => (BG_SELECTED, BORDER_SUBTLE, TEXT_PRIMARY),
                button::Status::Disabled => (BG_INPUT, Color::TRANSPARENT, TEXT_MUTED),
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border {
                color: border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    }
}

pub fn segmented_button(is_active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let (bg, border, fg) = if is_active {
            (BG_SELECTED, BORDER_SELECTED, ACCENT_BLUE)
        } else {
            match status {
                button::Status::Active => (BG_SURFACE, BORDER_SUBTLE, TEXT_MUTED),
                button::Status::Hovered => (BG_SURFACE_HOVER, BORDER_FOCUSED, TEXT_PRIMARY),
                button::Status::Pressed => (BG_SELECTED, BORDER_SUBTLE, TEXT_PRIMARY),
                button::Status::Disabled => (BG_INPUT, Color::TRANSPARENT, TEXT_MUTED),
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border {
                color: border,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    }
}

pub fn standard_input_style(_theme: &Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(BG_INPUT),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 4.0.into(),
        },
        icon: TEXT_MUTED,
        placeholder: TEXT_MUTED,
        value: TEXT_PRIMARY,
        selection: BG_SELECTED,
    }
}

pub fn card_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_SURFACE)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: Some(TEXT_PRIMARY),
        ..container::Style::default()
    }
}

pub fn chip_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_INPUT)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 3.0.into(),
        },
        text_color: Some(TEXT_SECONDARY),
        ..container::Style::default()
    }
}
