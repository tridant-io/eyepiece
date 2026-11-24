//! macos: the metal renderer, as the player (`crate::player`) knows it. the window is not written yet.

mod metal;
mod paint;

pub use metal::Gpu;
pub use paint::Paint;
