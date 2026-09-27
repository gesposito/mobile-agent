//! The macOS implementation behind [`SimulatorDriver`]: CoreSimulator and
//! its companion frameworks, loaded at run time and called through objc2.

mod coresim;
mod display;
mod driver;
mod dtuhid;
mod encode;
mod forward;
mod frames;
mod host;
mod indigo;
mod input;
mod objc;
mod prefs;
mod xpc;

pub use driver::SimulatorDriver;
