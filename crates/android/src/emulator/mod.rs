//! Client for the emulator's gRPC controller: the emulator fast path (design
//! D7, android-driver requirement "Emulator fast path").
//!
//! An emulator started with `-grpc <port> -grpc-use-token` serves the
//! `EmulatorController` service and writes the port and a bearer token into its
//! discovery file. [`resolve`] finds them for an adb serial, and
//! [`Controller::connect`] opens the connections and reads the hardware. When
//! the endpoint cannot be used the error is [`Unavailable`], whose warning says
//! how to enable it: the launch flag, or the console command that needs no
//! restart.
//!
//! The client handles the protocol quirks spike S7 found:
//!
//! - scaling needs both width and height ([`scaled_size`]);
//! - raw rows arrive top-down although the proto says bottom-up ([`Frame`]);
//! - the display size is only in the deprecated `hardwareConfig`
//!   ([`Hardware::from_status`]);
//! - every call needs a deadline, because some emulator ports accept TCP and
//!   never answer ([`Options`]);
//! - `sendKey` accepts and drops keys when the AVD has no keyboard, so the
//!   client sends none and reports [`Hardware::keyboard`] instead;
//! - the stream repeats unchanged frames and the status bar clock changes on
//!   its own, so changes are detected on hashes without the status bar
//!   ([`ChangeDetector`]).

mod controller;
mod discovery;
mod endpoint;
mod frame;
mod hardware;
pub mod proto;

pub use controller::{
    Controller, DOWN_PRESSURE, Error, FrameStream, MAX_MESSAGE, Options, TouchPoint,
};
pub use discovery::{
    Discovery, DiscoveryError, Found, candidate_dirs, console_port_from_serial, find_for_serial,
    pid_from_file_name, scan_dir,
};
pub use endpoint::{Endpoint, Unavailable, resolve, resolve_in};
pub use frame::{
    BYTES_PER_PIXEL, ChangeDetector, Frame, FrameError, FrameHasher, FrameKey, Observation,
};
pub use hardware::{DisplaySize, Hardware, scaled_size};

/// The proto file the client was generated from, as `build.rs` found it.
pub const PROTO_SOURCE: &str = env!("MOBILE_AGENT_EMULATOR_PROTO_SOURCE");
