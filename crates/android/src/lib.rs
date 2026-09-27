//! Android driver: adb client, agent deploy, agent session and emulator gRPC.
//!
//! [`adb`] speaks the adb server's smart-socket protocol with the driver's own
//! client (design D3).

pub mod adb;
