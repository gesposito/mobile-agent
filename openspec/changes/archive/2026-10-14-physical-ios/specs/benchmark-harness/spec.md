# Spec Delta

## MODIFIED Requirements

### Requirement: Latency gates
Gates SHALL be configurable per target class with these defaults: CLI process start under 20 ms; Simulator warm snapshot p95 under 150 ms on the SwiftUI reference screen and under 250 ms on the React Native one, whose own accessibility snapshot costs about 236 ms, and tap dispatch p95 under 20 ms; Android emulator warm snapshot p95 under 100 ms on the reference screen and, with the guest rendering on the host GPU, tap-to-frame-change p95 under 67 ms, four frames of the AVD's 60 Hz display, since the guest's own frame pipeline takes about three at p95; under software rendering tap-to-frame-change is reported and not gated; cold attach under 2 s on the Simulator and under 1.5 s on Android; and for a physical iOS device, warm snapshot p95 under 1 s on either reference screen, tap dispatch p95 under 150 ms through the host's input path, cold attach under 5 s with the runner already installed, and tap-to-frame-change reported and not gated while the device's frames are polled screenshots. A run that misses any gate SHALL exit non-zero and print the failing gates in a table.

#### Scenario: Gate missed
- **WHEN** Simulator warm snapshot p95 measures 210 ms
- **THEN** the run exits non-zero and the table names the gate, the limit and the measured value

#### Scenario: All gates met
- **WHEN** every measured value is within its gate
- **THEN** the run exits zero and the table shows each gate as passed

#### Scenario: Emulator tap-to-frame gate under host rendering
- **WHEN** an emulator rendering on the host GPU measures tap-to-frame-change p95 at 59 ms
- **THEN** the gate passes and the table shows its 67 ms limit

#### Scenario: iOS device gates
- **WHEN** a run on a physical iPhone measures warm snapshot p95 at 640 ms, tap dispatch p95 at 90 ms and cold attach at 3.1 s
- **THEN** the three gates pass, the table shows their limits, and tap-to-frame-change is listed with its value and no verdict
