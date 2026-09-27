//! Finding a device, runtime or device type from what a caller typed.

use crate::error::{Error, Result};
use crate::model::{DeviceTypeInfo, RuntimeInfo};

/// The index of the device whose UDID is `target` (in any case), or else of
/// the one device named exactly `target`.
pub fn device(target: &str, devices: &[(String, String)]) -> Result<usize> {
    let _ = (target, devices);
    unimplemented!("task 6.1: device lookup")
}

/// The index of the runtime whose version, name or identifier is `wanted`,
/// or, with nothing wanted, of the newest iOS runtime. The error lists the
/// installed runtimes.
pub fn runtime(wanted: Option<&str>, runtimes: &[RuntimeInfo]) -> Result<usize> {
    let _ = (wanted, runtimes);
    unimplemented!("task 6.1: runtime choice")
}

/// The index of the device type whose name or identifier is `wanted`. The
/// error lists the known device types.
pub fn device_type(wanted: &str, types: &[DeviceTypeInfo]) -> Result<usize> {
    let _ = (wanted, types);
    unimplemented!("task 6.1: device type choice")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> Vec<(String, String)> {
        [
            ("7B292E1E-7A29-41A5-9C69-2ABA1CEB3683", "mobile-agent-spike"),
            ("B0647CDD-575F-45E9-9398-F6F5AFF23C1F", "iPhone 17"),
            ("756DDFC3-A343-4D73-8143-6AAD4E470AB4", "twin"),
            ("A5082EAC-5F6D-48B8-AC9D-DC9C1DA2442B", "twin"),
        ]
        .iter()
        .map(|(udid, name)| (udid.to_string(), name.to_string()))
        .collect()
    }

    fn runtime_info(name: &str, version: &str, identifier: &str) -> RuntimeInfo {
        RuntimeInfo {
            name: name.into(),
            version: version.into(),
            build: String::new(),
            identifier: identifier.into(),
        }
    }

    #[test]
    fn a_device_is_found_by_udid_in_any_case_or_by_its_unique_name() {
        let devices = devices();
        assert_eq!(
            device("7B292E1E-7A29-41A5-9C69-2ABA1CEB3683", &devices),
            Ok(0)
        );
        assert_eq!(
            device("7b292e1e-7a29-41a5-9c69-2aba1ceb3683", &devices),
            Ok(0)
        );
        assert_eq!(device("iPhone 17", &devices), Ok(1));
    }

    #[test]
    fn an_unknown_or_shared_name_is_refused() {
        let devices = devices();
        assert_eq!(
            device("iphone 17", &devices),
            Err(Error::NotFound {
                target: "iphone 17".into()
            }),
            "names match exactly"
        );
        assert_eq!(
            device("twin", &devices),
            Err(Error::Ambiguous {
                target: "twin".into(),
                matches: 2
            })
        );
    }

    #[test]
    fn a_runtime_is_chosen_by_version_name_or_identifier_or_else_the_newest_ios() {
        let runtimes = vec![
            runtime_info(
                "iOS 18.6",
                "18.6",
                "com.apple.CoreSimulator.SimRuntime.iOS-18-6",
            ),
            runtime_info(
                "iOS 26.5",
                "26.5",
                "com.apple.CoreSimulator.SimRuntime.iOS-26-5",
            ),
            runtime_info(
                "watchOS 26.0",
                "26.0",
                "com.apple.CoreSimulator.SimRuntime.watchOS-26-0",
            ),
            runtime_info(
                "iOS 26.10",
                "26.10",
                "com.apple.CoreSimulator.SimRuntime.iOS-26-10",
            ),
        ];
        assert_eq!(runtime(Some("18.6"), &runtimes), Ok(0));
        assert_eq!(runtime(Some("iOS 26.5"), &runtimes), Ok(1));
        assert_eq!(
            runtime(
                Some("com.apple.CoreSimulator.SimRuntime.watchOS-26-0"),
                &runtimes
            ),
            Ok(2)
        );
        assert_eq!(runtime(None, &runtimes), Ok(3), "26.10 is the newest iOS");
        assert!(matches!(
            runtime(Some("17.0"), &runtimes),
            Err(Error::InvalidArgument(message)) if message.contains("iOS 26.5")
        ));
        assert!(matches!(runtime(None, &[]), Err(Error::InvalidArgument(_))));
    }

    #[test]
    fn a_device_type_is_chosen_by_name_or_identifier() {
        let types = vec![
            DeviceTypeInfo {
                name: "iPhone 17".into(),
                identifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-17".into(),
            },
            DeviceTypeInfo {
                name: "iPhone 17 Pro".into(),
                identifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro".into(),
            },
        ];
        assert_eq!(device_type("iPhone 17 Pro", &types), Ok(1));
        assert_eq!(
            device_type("com.apple.CoreSimulator.SimDeviceType.iPhone-17", &types),
            Ok(0)
        );
        assert!(matches!(
            device_type("iPhone 99", &types),
            Err(Error::InvalidArgument(message)) if message.contains("iPhone 17 Pro")
        ));
    }
}
