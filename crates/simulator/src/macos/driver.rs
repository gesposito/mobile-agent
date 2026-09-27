//! [`SimulatorDriver`]: the driver traits over CoreSimulator.

use std::path::Path;

use crate::driver::Lifecycle;
use crate::error::Result;
use crate::model::{
    BootReport, DeviceInfo, DeviceTypeInfo, LaunchOptions, Launched, PreferencesOutcome,
    RuntimeInfo,
};

/// Drives the iOS Simulators of the default device set.
pub struct SimulatorDriver {
    _private: (),
}

impl SimulatorDriver {
    /// Loads CoreSimulator and connects to its service for the selected Xcode.
    pub fn open() -> Result<SimulatorDriver> {
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    /// The installed runtimes.
    pub fn runtimes(&self) -> Result<Vec<RuntimeInfo>> {
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    /// The device types this Xcode knows.
    pub fn device_types(&self) -> Result<Vec<DeviceTypeInfo>> {
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    /// Creates a device of `device_type` (a name or identifier) on `runtime`
    /// (a version, name or identifier; the newest iOS runtime when `None`),
    /// with its accessibility preferences written before its first boot.
    pub fn create_device(
        &self,
        name: &str,
        device_type: &str,
        runtime: Option<&str>,
    ) -> Result<DeviceInfo> {
        let _ = (name, device_type, runtime);
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    /// Deletes a device, which must be shut down.
    pub fn delete_device(&self, target: &str) -> Result<()> {
        let _ = target;
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    /// Writes the accessibility preferences into a device that is shut down;
    /// `boot` does this itself.
    pub fn prepare_accessibility(&self, target: &str) -> Result<PreferencesOutcome> {
        let _ = target;
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }
}

impl Lifecycle for SimulatorDriver {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    fn boot(&self, target: &str) -> Result<BootReport> {
        let _ = target;
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    fn shutdown(&self, target: &str) -> Result<()> {
        let _ = target;
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    fn install(&self, target: &str, app: &Path) -> Result<()> {
        let _ = (target, app);
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    fn launch(&self, target: &str, bundle_id: &str, options: &LaunchOptions) -> Result<Launched> {
        let _ = (target, bundle_id, options);
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }

    fn terminate(&self, target: &str, bundle_id: &str) -> Result<()> {
        let _ = (target, bundle_id);
        unimplemented!("task 6.1: CoreSimulator lifecycle bindings")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_driver_can_be_shared_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<SimulatorDriver>();
    }
}
