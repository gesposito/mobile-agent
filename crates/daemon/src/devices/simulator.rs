//! The Simulator driver (`crates/simulator`, design D6 and D7) behind the
//! daemon's driver contract and the mirror's backend traits.

use std::time::Duration;

use mirror::Point;
use mirror::backend::NodeHandle;
use protocol::message::TextMethod as WireMethod;
use protocol::snapshot::Snapshot;
use simulator::{
    Button, Compatibility, DeviceInfo, DeviceState, ForegroundTree, Key, TargetStatus, TextMethod,
};

use crate::driver::{CompatibilityRow, DriverError, Size, TargetInfo, TargetReport};

/// The driver's name, as `status`, `doctor` and `--drivers` give it.
pub(crate) const NAME: &str = "simulator";

/// What an iOS key code of `keys.rs` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IosKey {
    /// A key of the keyboard page, with its modifiers.
    Key(Key),
    /// A button of the consumer page.
    Button(Button),
}

/// Whether a Simulator in `state` is a target: booted ones only, since
/// only they run apps and take input.
pub(crate) fn is_target(state: DeviceState) -> bool {
    let _ = state;
    todo!("task 8.8")
}

/// A Simulator as `devices` lists it.
pub(crate) fn target_info(device: DeviceInfo) -> TargetInfo {
    let _ = device;
    todo!("task 8.8")
}

/// The key or button `code` names: `(page << 16) | usage`, keyboard keys
/// on page 7 (or no page) with `meta` holding the HID modifier byte, and
/// buttons on the consumer page 0x0C.
pub(crate) fn ios_key(code: u32, meta: Option<u32>) -> Result<IosKey, DriverError> {
    let _ = (code, meta);
    todo!("task 8.8")
}

/// A Simulator driver error as the protocol carries it, with its code and
/// retryable flag.
pub(crate) fn error(error: &simulator::Error) -> DriverError {
    let _ = error;
    todo!("task 8.8")
}

/// The Simulator driver's compatibility table as `doctor`'s rows: the
/// host's components, then every private call and where it was validated.
pub(crate) fn doctor_rows(table: &Compatibility) -> Vec<CompatibilityRow> {
    let _ = table;
    todo!("task 8.8")
}

/// What `status` shows about an attached Simulator: its input transport,
/// or the host's plan before the first input, how its tree is read and by
/// which reader, its frames, and what took it out of service.
pub(crate) fn status_report(
    status: &TargetStatus,
    plan: Option<&str>,
    frames: u64,
    frame_error: Option<&str>,
) -> TargetReport {
    let _ = (status, plan, frames, frame_error);
    todo!("task 8.8")
}

/// The screen in points: the largest window of a read over its scale.
pub(crate) fn display_of(tree: &ForegroundTree) -> Option<Size> {
    let _ = tree;
    todo!("task 8.8")
}

/// The middle of the node `handle` names in `snapshot`, in points.
pub(crate) fn node_center(snapshot: &Snapshot, scale: f64, handle: NodeHandle) -> Option<Point> {
    let _ = (snapshot, scale, handle);
    todo!("task 8.8")
}

/// How text goes into a field for a request's method: key events when it
/// asks for keys, the driver's choice otherwise.
pub(crate) fn text_method(method: WireMethod) -> TextMethod {
    let _ = method;
    todo!("task 8.8")
}

/// The method a reply names for how the driver entered text: a paste sets
/// the text in one operation, as direct setting does.
pub(crate) fn wire_method(method: TextMethod) -> WireMethod {
    let _ = method;
    todo!("task 8.8")
}

/// How long reads retry the failures a launching app causes (no process
/// in front yet, a snapshot the runtime refused): an app answers 1.5 to
/// 2 s after its launch.
pub(crate) const READ_BUDGET: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    use protocol::snapshot::{
        Bounds, Node, NodeFlags, Truncation, Window, WindowFlags, WindowType,
    };
    use simulator::{
        CallSite, DeviceTypeInfo, InputStatus, Modifiers, ReaderState, ReaderStatus, Row,
        RuntimeInfo, Status, TransportKind, TreeBackend, TreeFailure, TreeStatus,
    };

    fn device(state: DeviceState) -> DeviceInfo {
        DeviceInfo {
            udid: "7B292E1E-7A29-41A5-9C69-2ABA1CEB3683".into(),
            name: "mobile-agent-spike".into(),
            state,
            runtime: Some(RuntimeInfo {
                name: "iOS 26.5".into(),
                version: "26.5".into(),
                build: "23F77".into(),
                identifier: "com.apple.CoreSimulator.SimRuntime.iOS-26-5".into(),
            }),
            device_type: Some(DeviceTypeInfo {
                name: "iPhone 17".into(),
                identifier: "com.apple.CoreSimulator.SimDeviceType.iPhone-17".into(),
            }),
        }
    }

    #[test]
    fn booted_simulators_are_listed_with_their_runtime_model_and_state() {
        assert!(is_target(DeviceState::Booted));
        for state in [
            DeviceState::Shutdown,
            DeviceState::Booting,
            DeviceState::ShuttingDown,
            DeviceState::Creating,
        ] {
            assert!(!is_target(state), "{state}");
        }
        let target = target_info(device(DeviceState::Booted));
        assert_eq!(target.id, "7B292E1E-7A29-41A5-9C69-2ABA1CEB3683");
        assert_eq!(target.platform, crate::driver::Platform::Ios);
        assert_eq!(target.class, crate::driver::TargetClass::Simulator);
        assert_eq!(target.name, "mobile-agent-spike");
        assert_eq!(target.model.as_deref(), Some("iPhone 17"));
        assert_eq!(target.os.as_deref(), Some("iOS 26.5"));
        assert_eq!(target.state, "booted");
        assert_eq!(target.advice, None);
        assert_eq!(target.driver, NAME);
    }

    #[test]
    fn keyboard_codes_become_keys_and_consumer_codes_buttons() {
        let key = |usage, modifiers| IosKey::Key(Key { usage, modifiers });
        assert_eq!(
            ios_key(0x07 << 16 | 0x28, None).unwrap(),
            key(0x28, Modifiers::default())
        );
        assert_eq!(
            ios_key(0x04, Some(0x08 | 0x02)).unwrap(),
            key(
                0x04,
                Modifiers {
                    shift: true,
                    command: true,
                    ..Modifiers::default()
                }
            ),
            "a code without a page is a keyboard usage"
        );
        assert_eq!(
            ios_key(0x07 << 16 | 0x04, Some(0x01 | 0x40)).unwrap(),
            key(
                0x04,
                Modifiers {
                    control: true,
                    option: true,
                    ..Modifiers::default()
                }
            ),
            "right-hand modifiers count as the left ones"
        );
        let button = |code: u32| ios_key(0x0C << 16 | code, None).unwrap();
        assert_eq!(button(0x40), IosKey::Button(Button::Home));
        assert_eq!(button(0x30), IosKey::Button(Button::Lock));
        assert_eq!(button(0xE9), IosKey::Button(Button::VolumeUp));
        assert_eq!(button(0xEA), IosKey::Button(Button::VolumeDown));
        for unknown in [0x0C << 16 | 0x99, 0x09 << 16 | 0x01] {
            let refused = ios_key(unknown, None).unwrap_err();
            assert_eq!(refused.code, "unknown_key", "{refused:?}");
        }
    }

    #[test]
    fn driver_errors_keep_their_code_and_retryable_flag() {
        let early = error(&simulator::Error::Tree {
            kind: TreeFailure::NoForeground,
            message: "starting".into(),
        });
        assert_eq!(early.code, "no_foreground");
        assert!(early.retryable);
        assert!(early.message.contains("starting"), "{early:?}");
        let gone = error(&simulator::Error::NotFound { target: "X".into() });
        assert_eq!(gone.code, "not_found");
        assert!(!gone.retryable);
    }

    #[test]
    fn the_compatibility_table_becomes_doctor_rows_with_every_private_call() {
        let table = Compatibility {
            rows: vec![Row {
                component: "Xcode".into(),
                detected: "27.0 (27A266a)".into(),
                expected: "Xcode 26.0 up to but not including 28".into(),
                status: Status::Validated,
                note: "validated on Xcode 27.0".into(),
            }],
            call_sites: vec![CallSite {
                call: "-[SimDevice lookup:error:]",
                image: "CoreSimulator",
                validated_on: "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S3",
            }],
        };
        let rows = doctor_rows(&table);
        assert_eq!(
            rows,
            [
                CompatibilityRow {
                    subject: "Xcode 27.0 (27A266a)".into(),
                    feature: "Xcode 26.0 up to but not including 28".into(),
                    status: "validated".into(),
                    note: Some("validated on Xcode 27.0".into()),
                },
                CompatibilityRow {
                    subject: "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S3".into(),
                    feature: "-[SimDevice lookup:error:] (CoreSimulator)".into(),
                    status: "validated".into(),
                    note: None,
                },
            ]
        );
    }

    fn status(input: Option<InputStatus>) -> TargetStatus {
        TargetStatus {
            device: device(DeviceState::Booted),
            input,
            tree: Some(TreeStatus {
                backend: TreeBackend::Reader,
                reason: "simulator-reader runs in the Simulator".into(),
                reader: Some(ReaderStatus {
                    name: "simulator-reader 0.1.0".into(),
                    path: PathBuf::from("/cache/simulator-reader-00ff"),
                    pid: 4242,
                    spawned_at: SystemTime::UNIX_EPOCH,
                    ready_in: Some(Duration::from_micros(292_400)),
                    state: ReaderState::Ready,
                    spawns: 1,
                }),
            }),
            unavailable: None,
        }
    }

    /// Spec scenarios "Two runtimes on one host" and "Spawned helper is
    /// reported" (simulator-driver): the status names the transport and
    /// the reader with its readiness time.
    #[test]
    fn the_status_names_the_transport_and_the_reader_with_its_readiness() {
        let report = status_report(
            &status(Some(InputStatus {
                transport: TransportKind::Dtuhid,
                activation: Some(Duration::from_micros(9_100)),
                connections: 1,
                reason: "CoreSimulator 1171.7 is 1155.4 or later".into(),
            })),
            Some("dtuhidd, then Indigo"),
            34,
            None,
        );
        let data = serde_json::Value::Object(report.data.clone());
        assert_eq!(data["input"], "dtuhidd", "{data}");
        assert_eq!(data["activationMs"], 9.1, "{data}");
        assert_eq!(data["connections"], 1, "{data}");
        assert_eq!(data["tree"], "simulator-reader", "{data}");
        assert_eq!(data["reader"]["pid"], 4242, "{data}");
        assert_eq!(data["reader"]["readyMs"], 292.4, "{data}");
        assert_eq!(data["reader"]["state"], "ready", "{data}");
        assert_eq!(data["framesReceived"], 34, "{data}");
        let text = report.lines.join("\n");
        assert!(
            text.contains("input: dtuhidd") && text.contains("1155.4"),
            "{text}"
        );
        assert!(
            text.contains("simulator-reader 0.1.0") && text.contains("292.4 ms"),
            "{text}"
        );
        assert!(text.contains("frames: 34"), "{text}");

        let before = status_report(&status(None), Some("dtuhidd, then Indigo"), 0, Some("gone"));
        let data = serde_json::Value::Object(before.data.clone());
        assert_eq!(data["input"], serde_json::Value::Null, "{data}");
        assert_eq!(data["inputPlan"], "dtuhidd, then Indigo", "{data}");
        let text = before.lines.join("\n");
        assert!(
            text.contains("input: dtuhidd, then Indigo") && text.contains("gone"),
            "{text}"
        );
    }

    fn node(id: u32, bounds: Bounds, children: Vec<Node>) -> Node {
        Node {
            id,
            class: 0,
            res_pkg: None,
            res_id: None,
            text: None,
            desc: None,
            bounds,
            flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
            children,
            scroll: None,
        }
    }

    fn window(id: u32, kind: WindowType, bounds: Bounds, root: Node) -> Window {
        Window {
            id,
            kind,
            layer: 0,
            package: None,
            title: Some("Fixture".into()),
            bounds,
            flags: WindowFlags::default(),
            root: Some(root),
            emits_events: Some(false),
            owner: None,
            modal: false,
        }
    }

    fn read() -> Snapshot {
        let screen = Bounds::new(0, 0, 1206, 2622);
        Snapshot {
            strings: vec!["XCUIElementTypeButton".into()],
            windows: vec![
                window(
                    7,
                    WindowType::INPUT_METHOD,
                    Bounds::new(0, 1700, 1206, 2622),
                    node(1, Bounds::new(0, 1700, 1206, 2622), Vec::new()),
                ),
                window(
                    4242,
                    WindowType::APPLICATION,
                    screen,
                    node(
                        2,
                        screen,
                        vec![node(3, Bounds::new(48, 534, 274, 637), Vec::new())],
                    ),
                ),
            ],
            node_count: 3,
            truncated: Truncation::default(),
            seq: 0,
        }
    }

    #[test]
    fn the_display_is_the_largest_window_in_points() {
        let tree = ForegroundTree::new(
            read(),
            3.0,
            TreeBackend::Reader,
            None,
            Duration::from_millis(14),
        )
        .expect("a foreground");
        assert_eq!(
            display_of(&tree),
            Some(Size {
                width: 402.0,
                height: 874.0
            })
        );
    }

    #[test]
    fn a_node_handle_is_found_in_the_read_that_numbered_it() {
        let snapshot = read();
        let center = |window, node| node_center(&snapshot, 3.0, NodeHandle { window, node });
        assert_eq!(center(4242, 3), Some(Point::new(161.0 / 3.0, 585.5 / 3.0)));
        assert_eq!(center(7, 3), None, "the node is in another window");
        assert_eq!(center(4242, 9), None);
    }

    #[test]
    fn keys_are_asked_for_by_name_and_a_paste_reads_as_direct_setting() {
        assert_eq!(text_method(WireMethod::KEYS), TextMethod::Keyboard);
        assert_eq!(text_method(WireMethod::AUTO), TextMethod::Auto);
        assert_eq!(text_method(WireMethod::DIRECT), TextMethod::Auto);
        assert_eq!(wire_method(TextMethod::Keyboard), WireMethod::KEYS);
        assert_eq!(wire_method(TextMethod::Paste), WireMethod::DIRECT);
    }
}
