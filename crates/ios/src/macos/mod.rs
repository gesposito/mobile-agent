//! The macOS side of the driver: everything that talks to usbmuxd and the
//! device through the `idevice` crate, on the driver's tokio runtime.

mod connection;
mod driver;
mod dvt;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod fake_runner;
mod host;
mod image;
mod runner;
mod tunnel;

pub use driver::IosDriver;
