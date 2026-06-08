//! Shared palette + reusable widget styles for the Guix GUI.
//!
//! Modeled after Jota's `styles.rs` — a single set of style closures that
//! views reach for via `.style(styles::card)`, `.style(styles::nav_btn(active))`,
//! etc. Keeps the color choices in one place so the whole app reads as one
//! coherent design instead of per-view ad-hoc styling.
#![allow(dead_code)] // lifted design system; some styles land in later milestones

use iced::font::Weight;
use iced::widget::{Space, button, container};
use iced::{Background, Border, Color, Element, Fill, Font, Shadow, Vector};

// -- Palette (gold accent on dark slate) --

pub const BG: Color = Color::from_rgb(0.051, 0.067, 0.090); // #0d1117
pub const SIDEBAR: Color = Color::from_rgb(0.024, 0.039, 0.063); // #060a10
pub const SURFACE: Color = Color::from_rgb(0.114, 0.157, 0.227); // #1d283a
pub const BORDER: Color = Color::from_rgb(0.204, 0.259, 0.337); // #344256
pub const ACTIVE: Color = Color::from_rgb(0.165, 0.125, 0.063); // #2a2010
pub const MUTED: Color = Color::from_rgb(0.396, 0.459, 0.545); // #65758b
pub const PRIMARY: Color = Color::from_rgb(0.961, 0.651, 0.137); // #f5a623
pub const TEXT: Color = Color::from_rgb(0.91, 0.92, 0.94);
pub const SUCCESS: Color = Color::from_rgb(0.290, 0.871, 0.502); // #4ade80
pub const DANGER: Color = Color::from_rgb(0.902, 0.192, 0.192); // #e63131
pub const WARNING: Color = Color::from_rgb(1.0, 0.757, 0.027);

// -- Fonts --

pub const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

// -- Container styles --

pub fn card(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 12.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.15),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..Default::default()
    }
}

pub fn card_flat(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

pub fn sidebar(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SIDEBAR)),
        ..Default::default()
    }
}

/// Tinted banner for the destructive-action warning on the Summary screen.
pub fn danger_banner(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(
            0.902, 0.192, 0.192, 0.12,
        ))),
        border: Border {
            color: Color::from_rgba(0.902, 0.192, 0.192, 0.45),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Tinted note used for the custom-system.scm caveat on the Summary screen.
pub fn warn_note(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(1.0, 0.757, 0.027, 0.10))),
        border: Border {
            color: Color::from_rgba(1.0, 0.757, 0.027, 0.35),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

// -- Button styles --

pub fn btn_primary(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        text_color: Color::BLACK,
        border: Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(PRIMARY)),
            shadow: Shadow {
                color: Color::from_rgba(0.961, 0.651, 0.137, 0.25),
                offset: Vector::new(0.0, 2.0),
                blur_radius: 6.0,
            },
            ..base
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgb(1.0, 0.72, 0.20))),
            shadow: Shadow {
                color: Color::from_rgba(0.961, 0.651, 0.137, 0.4),
                offset: Vector::new(0.0, 3.0),
                blur_radius: 10.0,
            },
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.85, 0.57, 0.10))),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.15, 0.19, 0.25))),
            text_color: Color::from_rgba(1.0, 1.0, 1.0, 0.35),
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

pub fn btn_secondary(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let base_border = Border {
        color: BORDER,
        width: 1.0,
        radius: 8.0.into(),
    };
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(Color::TRANSPARENT)),
            text_color: TEXT,
            border: base_border,
            ..Default::default()
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(ACTIVE)),
            text_color: Color::WHITE,
            border: base_border,
            ..Default::default()
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(SURFACE)),
            text_color: Color::WHITE,
            border: base_border,
            ..Default::default()
        },
        button::Status::Disabled => button::Style {
            text_color: Color::from_rgba(1.0, 1.0, 1.0, 0.3),
            border: Border {
                color: Color::from_rgba(0.204, 0.259, 0.337, 0.5),
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        },
    }
}

pub fn btn_danger(_theme: &iced::Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button::Style {
            background: Some(Background::Color(Color::from_rgba(
                0.902, 0.192, 0.192, 0.12,
            ))),
            text_color: DANGER,
            border: Border {
                color: Color::from_rgba(0.902, 0.192, 0.192, 0.25),
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(DANGER)),
            text_color: Color::WHITE,
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.75, 0.15, 0.15))),
            text_color: Color::WHITE,
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.15, 0.19, 0.25))),
            text_color: Color::from_rgba(1.0, 1.0, 1.0, 0.35),
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

pub fn btn_ghost(_theme: &iced::Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button::Style {
            background: None,
            text_color: TEXT,
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.05))),
            text_color: Color::WHITE,
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
        _ => button::Style {
            text_color: MUTED,
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

/// Search-results row. Selected rows render with the same subtle
/// highlight as a hover state, so the click target the user just
/// committed to is visually distinct from the rest of the list.
pub fn result_row_btn(selected: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let highlighted =
            selected || matches!(status, button::Status::Hovered | button::Status::Pressed);
        let background = if highlighted {
            Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.06)))
        } else {
            None
        };
        let text_color = if highlighted { Color::WHITE } else { TEXT };
        button::Style {
            background,
            text_color,
            border: Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// Sidebar nav row. `active` flips the row to a filled, gold-accented state.
pub fn nav_btn(active: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        if active {
            button::Style {
                background: Some(Background::Color(ACTIVE)),
                text_color: PRIMARY,
                border: Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        } else {
            match status {
                button::Status::Hovered => button::Style {
                    background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.04))),
                    text_color: Color::WHITE,
                    border: Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                _ => button::Style {
                    background: None,
                    text_color: MUTED,
                    border: Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            }
        }
    }
}

// -- Helpers --

pub fn separator<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new())
        .width(Fill)
        .height(1)
        .style(|_theme| container::Style {
            background: Some(Background::Color(Color::from_rgba(
                0.204, 0.259, 0.337, 0.5,
            ))),
            ..Default::default()
        })
        .into()
}
