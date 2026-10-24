//! Standard Widget Conventions, States, and Behavioral Contracts for NovaOS
//!
//! Provides the core state models, variant specifications, and standard
//! visual properties used across all NovaOS desktop components.

use crate::color::{Color, ThemePalette, WHITE};
use crate::spacing::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WidgetState {
    Normal,
    Hover,
    Pressed,
    Focused,
    Disabled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonVariant {
    Primary,       // Brand Azure background, white text
    Secondary,     // Neutral surface, border, primary text
    Danger,        // Crimson background, white text (destructive actions)
    Ghost,         // Transparent background until hovered
    WindowControl(WindowControlKind), // Titlebar window action buttons
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowControlKind {
    Close,
    Minimize,
    Maximize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckboxState {
    Unchecked,
    Checked,
    Indeterminate,
}

/// Resolved color set for rendering a widget in a specific state
#[derive(Clone, Copy, Debug)]
pub struct WidgetColorSet {
    pub bg: Color,
    pub border: Color,
    pub text: Color,
    pub radius: i32,
}

/// Computes unified widget color sets conforming to the design system rules
pub struct WidgetThemeSpec;

impl WidgetThemeSpec {
    pub fn button_colors(
        variant: ButtonVariant,
        state: WidgetState,
        palette: &ThemePalette,
    ) -> WidgetColorSet {
        match variant {
            ButtonVariant::Primary => match state {
                WidgetState::Normal => WidgetColorSet {
                    bg: palette.accent_primary,
                    border: palette.accent_primary,
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Hover => WidgetColorSet {
                    bg: palette.accent_hover,
                    border: palette.accent_hover,
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Pressed => WidgetColorSet {
                    bg: palette.accent_pressed,
                    border: palette.accent_pressed,
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Focused => WidgetColorSet {
                    bg: palette.accent_primary,
                    border: WHITE,
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Disabled => WidgetColorSet {
                    bg: palette.surface_sunken,
                    border: palette.border_subtle,
                    text: palette.text_muted,
                    radius: RADIUS_SM,
                },
            },
            ButtonVariant::Secondary => match state {
                WidgetState::Normal => WidgetColorSet {
                    bg: palette.surface_raised,
                    border: palette.border_normal,
                    text: palette.text_primary,
                    radius: RADIUS_SM,
                },
                WidgetState::Hover => {
                    let hover_bg = if palette.surface_raised.r > 200 {
                        palette.surface_raised.darken(18)
                    } else {
                        palette.surface_raised.lighten(25)
                    };
                    WidgetColorSet {
                        bg: hover_bg,
                        border: palette.border_focused,
                        text: palette.text_primary,
                        radius: RADIUS_SM,
                    }
                }
                WidgetState::Pressed => WidgetColorSet {
                    bg: palette.surface_sunken,
                    border: palette.border_focused,
                    text: palette.text_primary,
                    radius: RADIUS_SM,
                },
                WidgetState::Focused => WidgetColorSet {
                    bg: palette.surface_raised,
                    border: palette.accent_primary,
                    text: palette.text_primary,
                    radius: RADIUS_SM,
                },
                WidgetState::Disabled => WidgetColorSet {
                    bg: palette.window_bg,
                    border: palette.border_subtle,
                    text: palette.text_muted,
                    radius: RADIUS_SM,
                },
            },
            ButtonVariant::Danger => match state {
                WidgetState::Normal => WidgetColorSet {
                    bg: palette.status_danger,
                    border: palette.status_danger,
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Hover => WidgetColorSet {
                    bg: palette.status_danger.lighten(20),
                    border: palette.status_danger.lighten(20),
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Pressed => WidgetColorSet {
                    bg: palette.status_danger.darken(25),
                    border: palette.status_danger.darken(25),
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Focused => WidgetColorSet {
                    bg: palette.status_danger,
                    border: WHITE,
                    text: WHITE,
                    radius: RADIUS_SM,
                },
                WidgetState::Disabled => WidgetColorSet {
                    bg: palette.surface_sunken,
                    border: palette.border_subtle,
                    text: palette.text_muted,
                    radius: RADIUS_SM,
                },
            },
            ButtonVariant::Ghost => match state {
                WidgetState::Normal => WidgetColorSet {
                    bg: crate::color::TRANSPARENT,
                    border: crate::color::TRANSPARENT,
                    text: palette.text_primary,
                    radius: RADIUS_SM,
                },
                WidgetState::Hover => WidgetColorSet {
                    bg: palette.surface_raised,
                    border: palette.border_subtle,
                    text: palette.text_primary,
                    radius: RADIUS_SM,
                },
                WidgetState::Pressed => WidgetColorSet {
                    bg: palette.surface_sunken,
                    border: palette.border_normal,
                    text: palette.text_primary,
                    radius: RADIUS_SM,
                },
                WidgetState::Focused => WidgetColorSet {
                    bg: palette.surface_raised,
                    border: palette.accent_primary,
                    text: palette.text_primary,
                    radius: RADIUS_SM,
                },
                WidgetState::Disabled => WidgetColorSet {
                    bg: crate::color::TRANSPARENT,
                    border: crate::color::TRANSPARENT,
                    text: palette.text_muted,
                    radius: RADIUS_SM,
                },
            },
            ButtonVariant::WindowControl(ctrl) => {
                let base_color = match ctrl {
                    WindowControlKind::Close => palette.status_danger,
                    WindowControlKind::Minimize => palette.status_warning,
                    WindowControlKind::Maximize => palette.status_success,
                };
                match state {
                    WidgetState::Normal => WidgetColorSet {
                        bg: base_color,
                        border: base_color.darken(20),
                        text: WHITE,
                        radius: RADIUS_FULL,
                    },
                    WidgetState::Hover => WidgetColorSet {
                        bg: base_color.lighten(30),
                        border: base_color,
                        text: WHITE,
                        radius: RADIUS_FULL,
                    },
                    WidgetState::Pressed => WidgetColorSet {
                        bg: base_color.darken(30),
                        border: base_color.darken(40),
                        text: WHITE,
                        radius: RADIUS_FULL,
                    },
                    _ => WidgetColorSet {
                        bg: base_color,
                        border: base_color,
                        text: WHITE,
                        radius: RADIUS_FULL,
                    },
                }
            }
        }
    }

    pub fn input_colors(state: WidgetState, palette: &ThemePalette) -> WidgetColorSet {
        match state {
            WidgetState::Focused => WidgetColorSet {
                bg: palette.surface_sunken,
                border: palette.border_focused,
                text: palette.text_primary,
                radius: RADIUS_SM,
            },
            WidgetState::Disabled => WidgetColorSet {
                bg: palette.window_bg,
                border: palette.border_subtle,
                text: palette.text_muted,
                radius: RADIUS_SM,
            },
            _ => WidgetColorSet {
                bg: palette.surface_sunken,
                border: palette.border_normal,
                text: palette.text_primary,
                radius: RADIUS_SM,
            },
        }
    }
}
