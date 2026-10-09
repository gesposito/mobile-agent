//! The macOS side of the driver: everything that talks to usbmuxd and the
//! device through the `idevice` crate, on the driver's tokio runtime.

mod driver;
mod host;

pub use driver::IosDriver;
