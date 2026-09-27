//! The macOS implementation behind [`SimulatorDriver`]: CoreSimulator and
//! its companion frameworks, loaded at run time and called through objc2.

mod coresim;
mod display;
mod driver;
mod encode;
mod forward;
mod frames;
mod indigo;
mod input;
mod objc;
mod prefs;

pub use driver::SimulatorDriver;
