//! The macOS implementation behind [`SimulatorDriver`]: CoreSimulator and
//! its companion frameworks, loaded at run time and called through objc2.

mod driver;
mod prefs;

pub use driver::SimulatorDriver;
