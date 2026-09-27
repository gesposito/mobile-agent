//! Android driver: adb client, agent deploy, agent session and emulator gRPC.
//!
//! - [`adb`] speaks the adb server's smart-socket protocol with the driver's
//!   own client (design D3).
//! - [`agent`] deploys the resident agent, reuses or replaces a running one
//!   and connects to it (design D2 and D3).
//! - [`emulator`] drives the emulator's gRPC controller, the emulator fast
//!   path (design D7).

pub mod adb;
pub mod agent;
pub mod emulator;

#[cfg(test)]
mod fake_agent;
