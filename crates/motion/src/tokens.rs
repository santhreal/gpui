//! The per-role motion parameters.

use crate::{
    DirectThenSpringModel, DurationModel, Easing, FlipModel, MotionModel, MotionRole, SpringConfig,
    SpringFadeModel, TwoStepModel,
};

/// Motion parameters for each of the seven roles.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MotionTokens {
    /// [`MotionRole::Tint`].
    pub tint: DurationModel,
    /// [`MotionRole::Reveal`].
    pub reveal: SpringConfig,
    /// [`MotionRole::Float`].
    pub float: SpringFadeModel,
    /// [`MotionRole::Panel`].
    pub panel: DirectThenSpringModel,
    /// [`MotionRole::Shift`].
    pub shift: FlipModel,
    /// [`MotionRole::Scroll`].
    pub scroll: DurationModel,
    /// [`MotionRole::Caret`].
    pub caret: TwoStepModel,
}

impl Default for MotionTokens {
    fn default() -> Self {
        Self::reference()
    }
}

impl MotionTokens {
    /// The reference parameters: tint 120 ms `ease-out`; reveal spring
    /// 220/26/1; float spring 300/24/1 with a 4 px rise and a 90 ms fade;
    /// panel spring 180/22/1; shift 200 ms `ease-out`; scroll 240 ms
    /// `ease-in-out`; caret 900 ms period.
    pub const fn reference() -> Self {
        Self {
            tint: DurationModel {
                duration_ms: 120,
                curve: Easing::EaseOut,
            },
            reveal: SpringConfig::new(220.0, 26.0, 1.0),
            float: SpringFadeModel {
                spring: SpringConfig::new(300.0, 24.0, 1.0),
                rise_px: 4.0,
                fade_duration_ms: 90,
            },
            panel: DirectThenSpringModel {
                snap_spring: SpringConfig::new(180.0, 22.0, 1.0),
            },
            shift: FlipModel {
                duration_ms: 200,
                curve: Easing::EaseOut,
            },
            scroll: DurationModel {
                duration_ms: 240,
                curve: Easing::EaseInOut,
            },
            caret: TwoStepModel { period_ms: 900 },
        }
    }

    /// The model of `role`.
    pub const fn model(&self, role: MotionRole) -> MotionModel {
        match role {
            MotionRole::Tint => MotionModel::Duration(self.tint),
            MotionRole::Reveal => MotionModel::Spring(self.reveal),
            MotionRole::Float => MotionModel::SpringFade(self.float),
            MotionRole::Panel => MotionModel::DirectThenSpring(self.panel),
            MotionRole::Shift => MotionModel::Flip(self.shift),
            MotionRole::Scroll => MotionModel::Duration(self.scroll),
            MotionRole::Caret => MotionModel::TwoStep(self.caret),
        }
    }
}
