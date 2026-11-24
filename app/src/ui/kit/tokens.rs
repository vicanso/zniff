//! Shared numeric scales and semantic control variants for the zniff UI layer.

#[allow(dead_code)]
pub(crate) const TEXT_XS: f32 = 11.0;
#[allow(dead_code)]
pub(crate) const TEXT_SM: f32 = 12.0;
#[allow(dead_code)]
pub(crate) const TEXT_MD: f32 = 13.0;
#[allow(dead_code)]
pub(crate) const TEXT_LG: f32 = 15.0;

#[allow(dead_code)]
pub(crate) const RADIUS_SM: f32 = 4.0;
pub(crate) const RADIUS_MD: f32 = 6.0;
#[allow(dead_code)]
pub(crate) const RADIUS_LG: f32 = 10.0;

#[allow(dead_code)]
pub(crate) const MOTION_PRESS_MS: f32 = 90.0;
#[allow(dead_code)]
pub(crate) const MOTION_EXIT_MS: f32 = 110.0;
#[allow(dead_code)]
pub(crate) const MOTION_INSTANT_MS: f32 = 120.0;
#[allow(dead_code)]
pub(crate) const MOTION_FAST_MS: f32 = 160.0;
#[allow(dead_code)]
pub(crate) const MOTION_BASE_MS: f32 = 220.0;
#[allow(dead_code)]
pub(crate) const MOTION_PULSE_MS: f32 = 1400.0;
#[allow(dead_code)]
pub(crate) const PRESS_SCALE: f32 = 0.97;
#[allow(dead_code)]
pub(crate) const PRESS_SCALE_ICON: f32 = 0.94;

/// Returns `0.0` when the system "Reduce motion" accessibility setting is enabled.
#[allow(dead_code)]
pub(crate) fn motion_ms(ms: f32) -> f32 {
    if nsh_core::prelude::reduce_motion() {
        0.0
    } else {
        ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ControlSize {
    Xs,
    Sm,
    Md,
}

impl ControlSize {
    #[allow(dead_code)]
    pub(crate) const fn height(self) -> f32 {
        match self {
            Self::Xs => 22.0,
            Self::Sm => 28.0,
            Self::Md => 32.0,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn padding_x(self) -> f32 {
        match self {
            Self::Xs => 8.0,
            Self::Sm => 10.0,
            Self::Md => 12.0,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn text_size(self) -> f32 {
        match self {
            Self::Xs => TEXT_XS,
            Self::Sm | Self::Md => TEXT_SM,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn icon_size(self) -> f32 {
        match self {
            Self::Xs => 12.0,
            Self::Sm => 14.0,
            Self::Md => 16.0,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn radius(self) -> f32 {
        match self {
            Self::Xs => RADIUS_SM,
            Self::Sm | Self::Md => RADIUS_MD,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn gap(self) -> f32 {
        match self {
            Self::Xs | Self::Sm => 4.0,
            Self::Md => 6.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ButtonVariant {
    Primary,
    Secondary,
    Outline,
    Ghost,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum BadgeTone {
    Neutral,
    Accent,
    Ok,
    Warn,
    Err,
}
