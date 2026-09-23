//! Frame rate as an exact rational so 29.97 stays 30000/1001.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const FPS24: Self = Self { num: 24, den: 1 };
    pub const FPS25: Self = Self { num: 25, den: 1 };
    pub const FPS30: Self = Self { num: 30, den: 1 };
    pub const FPS29_97: Self = Self {
        num: 30000,
        den: 1001,
    };
    pub const FPS60: Self = Self { num: 60, den: 1 };

    pub fn label(&self) -> String {
        if self.den == 1 {
            format!("{} fps", self.num)
        } else {
            format!("{:.2} fps", self.num as f64 / self.den as f64)
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::FPS24,
            Self::FPS25,
            Self::FPS30,
            Self::FPS29_97,
            Self::FPS60,
        ]
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        Self::FPS30
    }
}
