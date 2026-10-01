//! Reusable UI widgets built on top of `egui`.
//!
//! Every widget funnels its dimensions through `crate::theme::tokens`,
//! so visual changes happen in one place. Never hardcode a color or size
//! in a panel — reach for a widget here, or add one.

pub mod button;
pub mod section;
pub mod toolbar;
