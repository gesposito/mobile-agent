//! iOS Simulator driver: lifecycle, input, frames and the accessibility tree,
//! all driven from the host (design D6, D7 and D12).
//!
//! The private Apple frameworks are loaded at run time and every call into
//! them runs inside an exception guard. [`SimulatorDriver`] implements the
//! driver traits, [`Lifecycle`], [`Frames`], [`Input`] and [`Tree`]; on
//! hosts other than macOS it exists but cannot be opened. The tree is read
//! by a reader spawned into the Simulator (`reader/`, built by `build.rs`).
//! `README.md` covers the build, the compatibility table and the map from
//! spec scenarios to tests.

// The platform-neutral helpers are used only by the macOS driver.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod compat;
mod driver;
mod error;
mod frames;
mod gate;
mod gesture;
mod keymap;
mod lookup;
mod model;
mod pipeline;
mod select;
mod tree;
mod version;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

pub use compat::{CallSite, Compatibility, Row, Status};
pub use driver::{Frames, Input, Lifecycle, Tree};
pub use error::{Error, Failure, Result, TreeFailure};
pub use frames::FrameStream;
pub use model::{
    BootReport, Button, DeviceInfo, DeviceState, DeviceTypeInfo, Diagnostics, Dispatch,
    DriverOptions, Edge, Frame, FrameOptions, FrameStats, Gesture, ImageFormat, InputStatus, Key,
    LaunchOptions, Launched, Modifiers, Point, PreferencesOutcome, ReaderState, ReaderStatus,
    RuntimeInfo, Screenshot, ScreenshotOptions, Swipe, TargetStatus, TextEntry, TextMethod,
    TransportChoice, TransportKind, TreeStatus,
};
pub use tree::{
    ALERT_CLASSES, Alert, AlertButton, Degradation, Foreground, ForegroundTree, TreeBackend,
    TreeChoice, TreeOptions,
};

// The frame thread's pixel work and its JPEG encoder, public so that
// benches/frames.rs can time them.
#[cfg(target_os = "macos")]
pub use macos::Encoder;
pub use pipeline::{
    FrameView, content_hash, fit_within, pixels_hash, scale_bgra, status_bar_rows, thumbnail_hash,
};

#[cfg(target_os = "macos")]
pub use macos::SimulatorDriver;
#[cfg(not(target_os = "macos"))]
pub use unsupported::SimulatorDriver;
