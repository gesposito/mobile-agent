//! Android driver: adb client, agent deploy, agent session and emulator gRPC.
//!
//! - [`adb`] speaks the adb server's smart-socket protocol with the driver's
//!   own client (design D3).
//! - [`emulator`] drives the emulator's gRPC controller, the emulator fast path
//!   (design D7).

pub mod adb;
pub mod emulator;
