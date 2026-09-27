# simulator

Drives iOS Simulators entirely from the host (design D6, D7 and D12, spec
`simulator-driver`): the lifecycle through CoreSimulator, input through
dtuhidd or the legacy Indigo client, frames from the display surface, and the
accessibility tree through a reader spawned into the guest. Nothing is built
or installed in the Simulator.

`SimulatorDriver` implements the driver traits (`Lifecycle` so far). They
carry no Apple types, so the mirror and the daemon can hold any backend
behind them; they live in this crate until `crates/mirror` defines its own.
On hosts other than macOS the crate still builds: `SimulatorDriver` exists
there, and `open` fails with `platform_unsupported`.

## Layout

```
src/
  driver.rs        the driver traits
  model.rs         the values they take and return
  error.rs         errors, each with a stable code
  lookup.rs        finding a device, runtime or device type from user input
  version.rs       dotted version comparison
  unsupported.rs   the driver on hosts other than macOS
  macos/
    objc.rs        framework loading, the exception guard, NSError conversion
    coresim.rs     CoreSimulator: service context, device set, devices
    prefs.rs       the accessibility preferences written before a first boot
    driver.rs      SimulatorDriver
tests/
  common/          gating, fixtures, temporary devices, simctl and ps helpers
  lifecycle.rs     task 6.1
```

## Private calls

Every Objective-C call into CoreSimulator is a dynamic message send inside
`objc::guarded`, which catches an Objective-C exception as an error naming
the call and drains the autorelease pool the call filled. Classes are looked
up by name (`AnyClass::get`), never linked. The exception guard needs
`panic = "unwind"`, which is the default for every profile of this workspace.

Before booting a device that is shut down, the driver writes
`AccessibilityEnabled`, `ApplicationAccessibilityEnabled` and
`AutomationEnabled` into `Library/Preferences/com.apple.Accessibility.plist`
under the device's `-[SimDevice dataPath]`, keeping every other key; spike S4
found both tree read paths failing for seconds after a boot without them.
`create_device` writes them too, so a new device has them before its first
boot. A booted device's preferences belong to the guest and are left alone.

## Tests

`cargo test -p simulator` runs the unit tests. They need macOS with Xcode,
since some load CoreSimulator, but no Simulator.

The integration tests in `tests/` drive real Simulators. Each checks
`MOBILE_AGENT_SIM_UDID` first; when it is unset, as in CI, the test prints why
it skipped and passes. To run them, name a booted iPhone 17 Simulator with
both fixtures installed and the Expo fixture on its Login tab:

```sh
MOBILE_AGENT_SIM_UDID=<udid> cargo test -p simulator
```

Tests that create devices name them `mobile-agent-simtest-*` and delete them
when they end, pass or fail. They install the fixtures from `fixtures/*/build`,
which `just fixtures-build` produces, or from the bundles named by
`MOBILE_AGENT_SWIFTUI_APP` and `MOBILE_AGENT_EXPO_APP`. A first boot takes
about a minute.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Simulator discovery and lifecycle | Launch with environment | `lifecycle::launch_with_environment`, `lifecycle::every_simulator_is_listed_with_identifier_name_runtime_and_state`, `lifecycle::a_booted_device_is_left_alone`, `lifecycle::an_unknown_device_is_not_found` |
