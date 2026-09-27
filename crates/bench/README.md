# bench

The benchmark harness of design decision D10 and the `benchmark-harness` spec. It measures the spec's primitives through adapters, one per tool, writes a JSON report and a markdown summary from the same data, judges the latency gates, and runs the same flows through Maestro, maestro-runner and agent-device for comparison.

The binary is `mobile-agent-bench`; `just bench <arguments>` runs it in release mode. Its `run` measures `mobile-agent` through the CLI adapter on the targets `--target` names, or, with `--fake`, the scripted fake adapter described below.

```sh
cargo build --release -p cli -p daemon -p bench       # mobile-agent, its daemon and the bench, side by side
target/release/mobile-agent-bench run --target emulator-5554 --out reports/emulator
target/release/mobile-agent-bench run --target <UDID> --cli target/release/mobile-agent --out reports/simulator
just bench run --fake run.json --out reports/my-run   # the scripted fake as the tool under test
just bench gates reports/my-run/report.json           # judge an existing report again
just bench gates --gates gates.json reports/my-run/report.json
just bench default-gates > gates.json                 # the spec's defaults, as a gate file
just bench baselines                                  # which baseline tools are on PATH
```

`run --target <id>` measures `mobile-agent` (`--tool mobile-agent`, the only tool under test there is) on the target `mobile-agent devices` lists with that id, a Simulator's UDID or an adb serial; `--target` repeats for several targets. `--cli <path>` names the `mobile-agent` to drive, by default the one beside `mobile-agent-bench`, where a build of the workspace puts it, else `mobile-agent` on `PATH`; it starts its daemon as usual, with the environment the bench runs in. CLI process start times that executable's process, so `--cli` names `mobile-agent` itself, not a wrapper, whose own start the span would include. Each target runs the fixtures of its platform, SwiftUI and Expo on a Simulator and Expo and Compose on Android, under their bundle identifiers and package names `dev.mobileagent.fixture.swiftui`, `.expo` and `.compose`; `--fixture swiftui|expo|compose`, repeatable, runs the named ones instead. The run's series are `plan::series` for each target and its fixtures, and its flows the fixtures' reference flows: the golden checks run first, then the timed series, each through `mobile-agent` and then, unless `--no-baselines`, the baselines, and between one tool and the next the harness hands the target over (see "Hand-offs between tools"). `run` refuses, with exit status 2 and before anything runs, a target `mobile-agent devices` does not list (naming the ones it lists), a fixture a target's platform lacks and a `--cli` that is not an executable. `--fake <script>` replaces `--target`, `--fixture` and `--cli`: the script's tool, targets, series and flows make up the run.

`run` also takes `--iterations` (default 50), `--warmup` (default 3), `--retries` (default 1), `--gates <file>` and `--no-baselines`; without `--out` it writes to `target/bench/<UTC start time>/`, and the CLI adapter's screenshots go to `cli/` and the baselines' flow files to `flows/` inside it. It prints where the reports went and the gate table. Exit status: 0 when every gate passed, 1 when any failed, 2 when the harness itself could not run.

## What is measured

A run is a list of series. A series is one subject, a primitive or a flow, on one target, in a scenario (a fixture app and one of its screens), and every tool that supports it measures it: first the warm-up iterations, then the measured ones. A failed attempt is retried within its iteration up to `--retries` times.

| Primitive | Spec wording | Scenario (`plan::scenario`) |
|---|---|---|
| `cli-start` | CLI process start | none |
| `cold-attach` | cold attach | a fixture, no screen |
| `first-snapshot` | first snapshot after application launch | a fixture's `big` screen |
| `warm-snapshot` | warm snapshot | a fixture's `big` screen, the spec's reference screen |
| `tap-dispatch` | tap dispatch, the Simulator gate's span | a fixture's `list` screen |
| `tap-to-frame-change` | tap-to-frame-change | a fixture's `list` screen |
| `tap-to-tree-delta` | tap-to-tree-delta | a fixture's `list` screen |
| `text-entry` | text entry of a fixed string | a fixture's `login` screen; the string is `TEXT_ENTRY`, `abcdefghijklmnopqrstuvwxyz0123456789` |
| `screenshot` | screenshot | a fixture's `big` screen |
| `settle` | settle detection | a fixture's `list` screen |

The spec lists nine primitives; `tap-dispatch` is the tenth because the Simulator gate is on dispatch, as spike S3 recommended. The scenario column is the run plan's: `plan::series(target, fixtures)` lists a target's series, CLI process start once, then for each fixture every other primitive in its scenario and the three reference flows, and `plan::fixtures(platform)` the fixtures a platform has. The harness itself takes any scenario a series names, and the default gates look for warm snapshots on the `big` screen. Fixtures and screens are those of `fixtures/CONTRACT.md`: `swiftui` (iOS), `expo` (both platforms) and `compose` (Android); `login`, `list`, `big`, `anim` and `system`.

Statistics follow the spikes: nearest-rank percentiles, so every reported value is a measured sample, over whole microseconds, with warm-up iterations excluded. Each series reports `n`, p50, p95 and max, the node counts the tool returned, a failure rate and a flake rate. An iteration whose first attempt succeeds passes; one that succeeds only on a retry is a flake, and its latency is the successful attempt's; one in which no attempt succeeds is a failure. Rates are percentages of the measured iterations, rounded half up to a tenth. A series whose preparation failed counts every measured iteration as failed. A series the tool cannot measure at all, which its adapter says with `AdapterError::not_measured` while preparing or in any attempt, ends there: it has no samples, no failures and no statistics, and the report lists it as not measured with the reason.

## Reports

`report.json` and `report.md` are rendered from one value, so they cannot disagree: the JSON holds durations as integer microseconds (`_us` keys) and the markdown prints the same values as milliseconds with three decimals, an exact rendering. The JSON also keeps every iteration, warm-up included; the markdown summarizes.

JSON, schema version 1:

| Field | Content |
|---|---|
| `schema_version` | `1` |
| `provenance.started_at`, `finished_at` | RFC 3339 UTC |
| `provenance.host` | `hardware` (CPU and machine model), `cpu_cores`, `memory_bytes`, `os` (`macOS 26.6.2 (25G83)`), `load_average` (1, 5, 15 minutes at the start) |
| `provenance.xcode` | `Xcode 27.0 (27A266a)`, or null |
| `provenance.commit` | `sha` and `dirty` of the checkout the harness ran from, or null |
| `provenance.harness` | `version`, `iterations`, `warmup`, `retries` |
| `provenance.targets[]` | `id` (UDID or adb serial), `class` (`simulator`, `android-emulator`, `android-device`), `model`, `os`, `api_level` |
| `provenance.tools[]` | `name`, `role` (`under-test` or `baseline`), `status` (`available` or `skipped`), `version`, `note` (why it was skipped) |
| `measurements[]` | one per series and tool, below |
| `handoffs[]` | each hand-off of a target from one tool to the next, in order: `target`, `from` and `to` (the tool about to drive it, null for the release once the run is over), `elapsed_us`, `released` (what the tool's release did, null when it held nothing), `input_method` (on Android, the input method a tool left selected and the one set back, else null), `error` (why the release or the reset failed, else null) |
| `gates` | `passed` and `rows[]` with `gate`, `statistic`, `limit_us`, `tool`, `target`, `measured_us`, `status` (`pass`, `fail`, `not-measured`); null until judged |

Each measurement:

| Field | Content |
|---|---|
| `tool`, `target`, `target_class`, `fixture`, `screen` | what was measured where; `fixture` and `screen` may be null |
| `primitive` or `flow` | the subject: a primitive id or a flow name |
| `clock` | `wall` (read by the adapter around the tool's process or request) or `tool` (reported by the tool) |
| `iterations`, `warmup` | measured and warm-up iteration counts |
| `stats` | `n`, `p50_us`, `p95_us`, `max_us` over the successful measured iterations, or null |
| `reliability` | `iterations`, `failures`, `flakes`, `failure_rate_pct`, `flake_rate_pct` |
| `nodes` | `min` and `max` node count returned, or null |
| `error` | why the series could not run at all, or null |
| `not_measured` | why the tool could not measure the subject, or null; such a series has no samples |
| `end_state` | for a flow with a golden, the end state the tool's check read (the golden, when the flow was timed), or null |
| `samples[]` | per iteration: `iteration`, `warmup`, `outcome` (`pass`, `flake`, `fail`), `attempts`, `us`, `nodes`, `detail`, `errors` |

Provenance covers what the spec asks for: host hardware and OS from `sysctl` and `sw_vers` on macOS (`/proc` and `uname` on Linux), Xcode from `xcodebuild -version`, the commit from `git`, tool versions from each adapter's detection, and each target's model, OS and API level. An adapter may fill the target fields; the harness fills the rest from `adb -s <serial> shell getprop` or `xcrun simctl list devices --json`. `Provenance::environment_differences` lists the fields in which two runs' environments differ, ignoring times and load.

The markdown has a provenance table with targets and tools, a measurements table (tool, target, fixture, screen, subject, clock, n, p50, p95, max, Fail %, Flake %, nodes) followed by the series that could not run and the series not measured, each with its reason, a comparison table with one column per tool that measured something and one row per target, fixture, screen and subject, holding p50 / p95 (`no samples` or `not measured` where there is none), a line naming skipped tools and why, a table of the hand-offs, and the gate table.

## Gates

Defaults, from the spec as amended:

| Target class | Primitive | Fixture and screen | Limit |
|---|---|---|---|
| simulator | cli-start | | 20 ms |
| simulator | cold-attach | | 2000 ms |
| simulator | warm-snapshot | swiftui, big | 150 ms |
| simulator | warm-snapshot | expo, big | 250 ms |
| simulator | tap-dispatch | | 20 ms |
| android-emulator | cli-start | | 20 ms |
| android-emulator | cold-attach | | 1500 ms |
| android-emulator | warm-snapshot | any fixture, big | 100 ms |
| android-emulator | tap-to-frame-change | | 50 ms |
| android-device | cli-start | | 20 ms |
| android-device | cold-attach | | 1500 ms |

Every default reads p95. The spec states p95 for the snapshot, dispatch and tap-to-frame gates and no statistic for CLI start and cold attach, so those use p95 too; a gate file can choose `p50`, `p95` or `max` per gate. "Under" is strict: a value equal to its limit fails.

- Gates judge only the tool under test, never a baseline.
- Only gates whose target class has a target in the run are listed.
- A gate with no matching measurement, or whose measurement the tool could not make, is listed as not measured and does not fail the run; a matching series in which no iteration succeeded fails.
- A gate file replaces the defaults: `{"gates": [{"class": "android-emulator", "primitive": "tap-to-frame-change", "limit_ms": 160}]}`, with optional `fixture`, `screen` and `statistic` (default `p95`). Limits must be positive. This is how a gate is restated per GPU mode, as design.md foresees for the emulator's tap-to-frame gate.

## Adapter interface

`adapter::Adapter` is what every tool implements, and what the CLI adapter of task 9.4 will implement for `mobile-agent`:

```rust
pub trait Adapter {
    fn name(&self) -> &str;
    fn role(&self) -> Role;                    // UnderTest or Baseline
    fn detect(&mut self) -> Detection;         // Available { version } or Absent { note }
    fn supports(&self, target: &Target, scenario: &Scenario, subject: &Subject) -> bool;
    fn prepare(&mut self, target: &Target, scenario: &Scenario, subject: &Subject)
        -> Result<(), AdapterError> { Ok(()) }
    fn measure(&mut self, target: &Target, scenario: &Scenario, primitive: Primitive)
        -> Result<Sample, AdapterError>;       // default: unsupported
    fn run_flow(&mut self, target: &Target, flow: &Flow)
        -> Result<Sample, AdapterError>;       // default: unsupported
    fn end_state(&mut self, target: &Target, flow: &Flow)
        -> Result<EndState, AdapterError>;     // default: unsupported
    fn check_flow(&mut self, target: &Target, flow: &Flow)
        -> Result<EndState, AdapterError> {    // default: run_flow, then end_state
        self.run_flow(target, flow)?;
        self.end_state(target, flow)
    }
    fn release(&mut self, target: &Target)
        -> Result<Option<String>, AdapterError> { Ok(None) }
}
```

- The harness never times a call. `measure` returns a `Sample` holding the latency of exactly the primitive's span, the clock it read (`Wall` or `Tool`), the node count for snapshots and an optional detail such as a text entry method. Work outside the span, such as stopping the daemon before a cold attach or relaunching the app before a first snapshot, happens inside the call but outside the span.
- `end_state` reads the screen a flow left through the tool and summarizes it in the terms of the flow's golden; `check_flow` is one untimed run of the flow and that reading, which the harness compares with the golden before it times the flow (see "Golden end states").
- `detect` runs once per run, before any series; an `Absent` tool is skipped with its note. `supports` makes the harness skip combinations a tool cannot measure instead of recording failures. `prepare` runs once per series, before its warm-up; `Scenario` names the fixture and screen.
- An `AdapterError` is a failed attempt; the harness retries it within the iteration. `AdapterError::not_measured(reason)` is no failure: the tool cannot measure the subject at all, and the series is reported as not measured.
- `release` lets go of a target so that another tool can drive it, and says what it did (see "Hand-offs between tools").

## The CLI adapter

`cli::CliAdapter` drives `mobile-agent`, the tool under test, as an agent drives it: one process per command in `--json` mode, each reply in agent-browser's envelope (`{success, data, error, code?, warning?}`, `docs/research/protocol.md`), and a reply with `success: false` a failed attempt carrying its code and the first line of its message. Every command names the bench's session and the target, then the command, its flags, `--` and its operands, so an operand that starts with a hyphen stays an operand:

```sh
mobile-agent --json --session mobile-agent-bench --device emulator-5554 click --timeout 10000 -- id=row-1
```

`cli::CliConfig` names the executable (a path, or a name looked up on `PATH`), changes to the environment of every command (the tests point it at a private socket directory with the daemon's fake targets), a working directory, a work directory for screenshots and a deadline per command, 120 s by default. Detection runs `mobile-agent --version`; an executable that is not there is skipped with a note. `CliAdapter::devices` lists the targets `mobile-agent devices` reports, with their class, model and OS, which `run --target` chooses from.

Each primitive, in its `plan::scenario`:

| Primitive | Before the span | Span | Clock | Nodes |
|---|---|---|---|---|
| `cli-start` | | one `mobile-agent --version` process, its output discarded, as hyperfine times a command | wall | |
| `cold-attach` | `shutdown`, repeated until it finds no daemon | `open <app>`, which starts a daemon, attaches the target and brings the app up; the sample notes how long the open's settle took | wall | |
| `first-snapshot` | `open --relaunch --env <big> <app>`, where `<big>` is the Big screen's launch option | `snapshot -i`, repeated until one lists `cell-500`, from the first one's start | wall | refs |
| `warm-snapshot` | once per series: the app relaunched on `big`, `cell-500` visible, a settle, one snapshot | one `snapshot -i`, which must list `cell-500` | wall | refs |
| `tap-dispatch` | once per series: the app relaunched on `list`, `row-1` visible, a settle | a `click` on `row-1`, which opens its detail: the reply's `dispatchMs`; then a `click` on `detail-back` and a settle bring the quiet list back | tool | |
| `tap-to-frame-change` | as for `tap-dispatch` | the same click's `frameChangeMs` | tool | |
| `tap-to-tree-delta` | as for `tap-dispatch` | the same click's `treeDeltaMs` | tool | |
| `text-entry` | once per series: the app relaunched on `login`, a settle | `fill` of `TEXT_ENTRY` into the empty `login-username`, noting the fill's method; then a wait until the string shows, a check that the field holds exactly it, the field emptied and a settle | wall | |
| `screenshot` | once per series: the app relaunched on `big` | one `screenshot` into the work directory, noting its size | wall | |
| `settle` | a `click` on `row-1` | `wait --settle`: its reply's `elapsedMs`, noting `endedBy`; then the list brought back | tool | |

A relaunch on a screen passes the launch option the fixtures of the target's platform read (`fixtures/CONTRACT.md`, Launch options, and `fixture::Screen::launch_option`): `--env FIXTURE_INITIAL_TAB=<screen>` on a Simulator, and on Android `--env initial_tab=<screen>`, which the daemon passes as the launch intent's string extra `initial_tab`.

Spans read from replies are milliseconds to the microsecond, on the daemon's clock (`crates/cli/README.md`, "Timing spans"). A click that watched the tap (its reply has `watchedMs`) without a `frameChangeMs` or `treeDeltaMs` is a failed attempt: the tap changed nothing that way. A span the target cannot produce is not measured, with the reason: tap to frame change on an `android-device` target, which has no frame stream (design D7), and both tap spans from a `mobile-agent` whose click does not watch the tap (before task 8.9).

A flow runs its steps as commands, timed by wall clock from the first command's start to the last one's end:

| Step | Commands |
|---|---|
| `launch` | `open --relaunch <app>` |
| `tap` | `click --timeout 10000 <selector>` |
| `fill` | `fill --timeout 10000 <selector> <text>` |
| `scroll-to` | until the element is visible: `wait --settle`, then `wait --timeout 0 <selector>`, a check that does not wait, and on `wait_timeout` `scroll <direction> 600`; at most 50 scrolls, whose number the sample notes |
| `assert-visible` | `wait --timeout 10000 <selector>` |
| `assert-text` | `wait --timeout 10000 --text=<text>`, then `get text id=<id>`, which must be exactly the text |

A selector is `id=<identifier>` or `text=<text>`; the CLI matches a text as part of an element's name or value, so a text selector is exact only while no other element's text contains it. The settle before each scroll check keeps a check from racing a list still moving from the last scroll, and a step of 600 points, less than a phone screen's content, keeps an element from passing through the view between two checks. A flow that records its node count takes one `snapshot -i` after its timing and counts the refs.

## Flow description

`flow::Flow` describes a flow once for every tool: a name, a fixture, a list of steps, whether to record the final screen's node count outside its timing, and the golden end state that screen must show:

| Step | JSON | Meaning |
|---|---|---|
| `Launch` | `"launch"` | stop the app if it runs, then launch it; it opens on the Login tab |
| `Tap(selector)` | `{"tap": {"id": "login-submit"}}` | tap the element |
| `Fill { target, text }` | `{"fill": {"target": {"id": "login-username"}, "text": "agent"}}` | type into an empty field |
| `ScrollTo { target, direction }` | `{"scroll-to": {"target": {"id": "row-200"}, "direction": "down"}}` | scroll until the element is visible |
| `AssertVisible(selector)` | `{"assert-visible": {"id": "detail-200"}}` | wait until the element is visible |
| `AssertText { id, text }` | `{"assert-text": {"id": "login-status", "text": "Welcome, agent"}}` | wait until the element shows the text |

A selector is `{"id": …}`, the contract identifier, or `{"text": …}`, the exact visible text. Flows reach screens by tapping their tabs rather than through the fixtures' launch option, because Maestro passes launch arguments to iOS apps as process arguments, not environment variables.

A golden (`end_state::EndState`) states what the final screen shows when the flow did its work, in three kinds of terms: the text an identified element shows (`texts`, `null` for no such element), whether an element has an identifier (`present`), and how many distinct identifiers `<prefix>-<n>` elements have (`counts`). As JSON it sits under the flow's `golden` key; a flow without one leaves the key out.

The spec's three flows, `flow::reference(fixture)` in report order, are the same on every fixture:

| Flow | Steps | Golden |
|---|---|---|
| `login` (`flow::login`) | launch; fill `login-username` with `agent`; fill `login-password` with `hunter2`; tap `login-submit`; wait until `login-status` shows `Welcome, agent` | `{"texts": {"login-status": "Welcome, agent"}}` |
| `list-scroll` (`flow::list_scroll`) | launch; tap `tab-list`; scroll down until `row-200` is visible; tap it; wait until `detail-200` shows `Detail 200` | `{"present": {"detail-200": true}}` |
| `big-screen` (`flow::big_screen`) | launch; tap `tab-big`; wait until `cell-500` is visible; record the node count | `{"counts": {"cell": 500}}` |

A run may hold the same flow for several fixtures. A flow series names the flow and a fixture the run has it for, and no screen; two flows with one name on one fixture are refused before anything runs.

## Golden end states

Timing starts only once the tools agree on what a flow does. Before any series is timed, every flow with a golden runs once, untimed, through each tool that will time it (`Adapter::check_flow`), and the tool reads the screen the flow left (`Adapter::end_state`). The reading is summarized in the golden's terms (`EndState::observe`): the text of the first element with each named identifier (value before name or label, as each tool reports it; the empty text when it shows none; null when no element has the identifier), whether an element has each identifier the golden requires, and how many distinct identifiers `<prefix>-<n>` the screen has. A tool whose summary differs from the golden, after `--retries` further tries, does not time the flow: its series has the error `the end state differs from the golden:` followed by each difference (such as `login-status shows "Invalid credentials", not "Welcome, agent"`), counts every measured iteration as failed, and is listed among the series that could not run. The summary read is kept as the measurement's `end_state`. A flow without a golden is timed unchecked.

Each tool reads its screen its own way:

| Tool | Reading | Identifier | Text |
|---|---|---|---|
| the fake | its flow series' scripted `end_state`, else the flow's golden; a check uses none of the scripted attempts | | |
| `mobile-agent` | a full `snapshot`, its listing parsed line by line (`cli::listing`) | `id=` | the value after `: `, else the name, as `get text` reads it |
| Maestro | `maestro --device <id> hierarchy`: a tree of nodes with `attributes` and `children` | `resource-id`, without the `<package>:id/` prefix of native Android views | `text`, else `accessibilityText`, where Maestro puts an iOS label |
| maestro-runner | `maestro-runner --platform <ios\|android> --device <id> hierarchy`: its tree normalized across drivers, nodes with `id`, `text` and `children` | `id` | `text` |
| agent-device | `agent-device snapshot --json --platform <p> --udid\|--serial <id> --session mobile-agent-bench`: `data.nodes`, a flat list linked by `parentIndex` | `identifier` | `value`, else `label`; a node marked `inheritsLabel` or `inheritsIdentifier`, whose value agent-device left out as equal to its nearest ancestor's, takes the ancestor's |

A baseline's check is one run of the flow's file or batch and one read of the screen. The shapes come from the tools' sources: Maestro's `PrintHierarchyCommand`, `TreeNode` and its drivers' attribute mapping (github.com/mobile-dev-inc/maestro), maestro-runner's `hierarchy` command (`pkg/cli/hierarchy.go` in github.com/devicelab-dev/maestro-runner), and the installed agent-device 0.21.14's `CaptureSnapshotResult` type, its JSON printer and its snapshot serialization. agent-device prints its JSON pretty-printed, so replies are read as the first JSON object that starts a line, with any progress lines before it and anything after it ignored; the batch reply is read the same way.

## Baselines

Each baseline adapter finds its executable on `PATH`, following symbolic links, and reads its version from `--version`; a tool that is not found is skipped with the note `<tool> not found on PATH`, and the tools that ran are still measured and compared. Baselines run flows only, timed by wall clock around the tool's process, since none of them exposes the primitives' spans; the flow file is written to `<out>/flows/` and rewritten before every run.

| Tool | Flow as | Command |
|---|---|---|
| Maestro | a Maestro flow file: `appId`, then `launchApp`, `tapOn`, `inputText` (after a `tapOn` of its field), `scrollUntilVisible` (with a 90 s `timeout`), `assertVisible` | `maestro --device <id> test <file>` |
| maestro-runner | the same file, which it runs unchanged | `maestro-runner --platform <ios\|android> --device <id> test <file>` |
| agent-device | one batch of `open` (with `relaunch`), `press`, `fill`, `scroll` (with `until`), `wait` and `is` steps with their structured inputs | `agent-device batch --steps-file <file> --on-error stop --json --platform <p> --udid <udid>` or `--serial <serial>` `--session mobile-agent-bench` |

Maestro matches `id` and `text` as regular expressions, so selector values are escaped. Its `scrollUntilVisible` gives up after 20 s by default; on the Android emulator Maestro 2.10.0 swipes about 9 rows of a fixture's list every 2.2 to 2.5 s, so the default reaches row 97 of 200, and the flow's scroll has 90 s (`maestro::SCROLL_TIMEOUT_MS`), about 52 s of which reach row 200, within the flow's 120 s deadline. Assertions wait, as Maestro's do: agent-device checks a visible element with `wait` on its selector, and a text with `wait` for the text followed by `is text` on the element. A Maestro or maestro-runner flow fails on a non-zero exit, reported with the last lines the tool printed; an agent-device flow fails on `success: false` in its JSON reply, reported with the failing step and error, and its `totalDurationMs` is kept as the sample's detail.

A flow that records its node count, the 500-node flow, reads the screen once more after each timed run, outside the timing, with the same command as the golden check ("Golden end states"), and records how many nodes the tool's output held: every node of Maestro's and maestro-runner's trees, every entry of agent-device's `nodes`. A read that fails fails the iteration. `mobile-agent` records the refs of a `snapshot -i` in the same place, so the report shows the node count each tool actually returned.

The command shapes come from the tools' documentation: the Maestro CLI reference and command pages (`maestro-cli/maestro-cli-commands-and-options.md`, `api-reference/commands-available/*.md` in github.com/mobile-dev-inc/maestro-docs), the maestro-runner README (github.com/devicelab-dev/maestro-runner), and agent-device's batching, selector and command pages (`website/docs/docs/*.md` in github.com/callstack/agent-device) together with `agent-device help batch` and `help open` of version 0.21.14. maestro-runner was not installed where these adapters were written: its `--device` flag comes from its documentation, and the form of its `--version` output is unverified.

## Hand-offs between tools

An Android device has one UI automation owner at a time, and tools keep the role between their commands: `mobile-agent`'s daemon keeps its agent until the daemon exits (`close` only ends the session's claim), and agent-device keeps a snapshot helper, an instrumentation, until its session is closed. A tool that finds the role held fails, as Maestro's driver did on the emulator with `UiAutomationService ... already registered!`. Tools also leave device state behind: agent-device selects its test input method for text entry, and Maestro carries one of its own. So the harness hands a target from one tool to the next (`handoff`):

- It notes which tool drives each target. Before another tool drives it, and once every series has run, the tool that drove it last lets go of it (`Adapter::release`). Hand-offs happen between series, never inside one, so no measured iteration includes one; the first iteration after a hand-off pays the next tool's own start, such as `mobile-agent`'s daemon and agent, which warm-up iterations absorb.
- `mobile-agent` reads `status`, stops its daemon with `shutdown` until none answers, then waits, up to 10 s, until the daemon's socket is gone: the daemon answers `shutdown` and stops taking connections before it detaches its targets and stops their agents, and removes its socket after.
- agent-device closes its session: `agent-device close --json --platform <p> --udid|--serial <id> --session mobile-agent-bench`, which stops the snapshot helper and selects the input method it found again (never `close --shutdown`, which would shut the target down). A session that is not there held nothing.
- Maestro and maestro-runner hold nothing: Maestro's driver instrumentation ends with the `maestro test` process that started it.
- On an Android target the harness then selects the input method the run found again, read at the start with `adb -s <serial> shell settings get secure default_input_method`, when a tool left another selected (`adb -s <serial> shell ime set <it>`).

Each hand-off is recorded in `handoffs` with its duration, what the release did, the input method set back and any failure; a failed release does not stop the run, and the next tool meets the target as it was left. Before a run, the target should be as a run leaves it: the stock input method, no automation running, the launcher in front.

## Fake adapter

`fake::FakeAdapter` stands in for the tool under test in the tests and in `run --fake`. A script declares the tool, its targets, the flows its flow series name, and per series the durations to return, one per attempt in order and repeating, the attempts that fail, the node count and the clock:

```json
{
  "tool": "mobile-agent",
  "version": "0.1.0",
  "targets": [{"id": "SIM-1", "class": "simulator"}],
  "series": [
    {"target": "SIM-1", "primitive": "cli-start", "ms": [4.1, 4.3, 3.9]},
    {"target": "SIM-1", "fixture": "swiftui", "screen": "big", "primitive": "warm-snapshot",
     "ms": [126.4, 130.4], "nodes": 546, "fail": [7], "clock": "wall"}
  ],
  "flows": []
}
```

Its targets and series make up the run; the baselines run its flow series for real unless `--no-baselines` is given.

## Scenarios and tests

| Spec requirement, scenario | Tests |
|---|---|
| Measured primitives, "Primitive report" | `measurement::every_listed_primitive_appears_with_its_percentiles_and_sample_count`; `cli_adapter::every_listed_primitive_appears_through_the_cli_adapter_on_a_fake_emulator` for `mobile-agent` itself, and through `run --target` in the `run::*` runs; `primitive::tests::every_listed_primitive_is_in_all_and_only_tap_dispatch_is_added`; the `stats::tests` |
| Measured primitives: through `mobile-agent`'s CLI | `cli_adapter::a_tap_on_an_android_device_has_no_frame_change_and_is_listed_as_not_measured`; `android::a_screen_opens_through_the_launch_option_its_platform_s_fixtures_read`; `fixture::tests::each_platform_opens_a_screen_through_the_launch_option_its_fixtures_read`; `cli_adapter::click_replies_that_carry_frame_and_tree_spans_fill_them_in`; `cli_adapter::a_cli_whose_click_does_not_watch_the_tap_leaves_both_spans_not_measured`; `cli_adapter::a_tap_whose_effects_never_came_fails_its_spans`; `cli_adapter::a_failed_command_is_an_error_naming_the_command_its_code_and_its_message`; `cli_adapter::cold_attach_stops_the_daemon_then_times_the_open_that_starts_another`; `cli_adapter::a_first_snapshot_follows_a_relaunch_on_the_big_screen_and_counts_its_refs`; `cli_adapter::text_entry_fills_the_fixed_string_into_the_username_field`; `cli_adapter::a_text_entry_that_did_not_land_fails`; `cli_adapter::an_absent_mobile_agent_is_skipped_with_a_note`; `cli_adapter::the_cli_adapter_measures_each_primitive_in_the_scenarios_it_knows`; `cli_adapter::a_release_stops_the_daemon_and_returns_once_it_has_let_go_of_its_targets`; the `cli::tests`, `cli::listing::tests` and `plan::tests`; `process::tests::a_timed_run_gives_the_exit_status_and_covers_the_run` |
| Latency gates, "Gate missed" | `gates::a_missed_gate_fails_the_run_and_the_table_names_the_gate_limit_and_value`; `gates::a_run_that_misses_a_gate_exits_non_zero_and_prints_the_table` |
| Latency gates, "All gates met" | `gates::when_every_gate_is_met_the_run_exits_zero_and_the_table_shows_each_passed`; `gates::a_run_that_meets_every_gate_exits_zero_through_the_binary` |
| Latency gates: the defaults, configurable per target class | `gates::the_defaults_are_the_spec_gates`; `gates::gates_are_configurable_per_target_class`; `gates::the_gates_command_judges_an_existing_report`; `gates::only_the_tool_under_test_is_gated`; `gates::a_gate_without_a_measurement_is_reported_but_does_not_fail`; `cli_adapter::a_tap_on_an_android_device_has_no_frame_change_and_is_listed_as_not_measured` for a span the tool cannot measure; `gates::a_primitive_that_never_succeeded_fails_its_gate`; `gates::a_value_equal_to_the_limit_misses_the_gate`; the `gates::tests` |
| Baseline tools, "One baseline missing" | `baselines::an_absent_tool_is_noted_as_skipped_and_the_tools_that_ran_are_still_compared`; `run::a_run_on_a_fake_simulator_runs_swiftui_and_expo_through_the_cli_beside_the_bench`; `baselines::every_baseline_is_skipped_when_none_is_installed`; `baselines::a_run_through_the_binary_includes_the_installed_baselines`; `baselines::the_baselines_command_lists_each_tool_and_notes_absent_ones` |
| Baseline tools: the same flows through each tool's CLI | `run::a_run_measures_mobile_agent_on_a_fake_emulator_and_the_baselines_beside_it`; `run::a_run_hands_the_emulator_from_one_tool_to_the_next_between_series` and the `handoff::tests` for the hand-off between tools; `baseline::maestro::tests::the_list_flow_gives_maestro_s_scroll_the_time_to_reach_row_200`; `golden::every_adapter_reports_the_golden_end_state_of_each_flow_on_android`; `golden::every_adapter_reports_the_golden_end_state_of_each_flow_on_ios`; the `baseline::maestro::tests` and `baseline::agent_device::tests`; `baselines::a_flow_the_tool_fails_is_a_failed_iteration_with_its_output`; `baselines::agent_device_failures_are_read_from_its_json_reply`; `baselines::a_series_the_run_cannot_carry_out_is_refused_before_anything_runs`; the `baseline::tests` |
| Reference fixtures, "Large screen fixture" | `golden::the_large_screen_records_the_node_count_each_tool_returned` for the fake tool under test and the three baselines; `cli_adapter::every_listed_primitive_appears_through_the_cli_adapter_on_a_fake_emulator` and `cli_adapter::the_reference_flows_run_through_the_cli_adapter_on_a_fake_emulator` for `mobile-agent`'s refs; `measurement::the_large_screen_records_the_node_count_the_tool_returned`; the readers' `baseline::maestro::tests` and `baseline::agent_device::tests` |
| Reference fixtures: `run --target` on each platform's fixtures | `run::a_run_measures_mobile_agent_on_a_fake_emulator_and_the_baselines_beside_it`; `run::a_run_on_a_fake_simulator_runs_swiftui_and_expo_through_the_cli_beside_the_bench`; `run::a_target_mobile_agent_does_not_list_is_refused_naming_the_connected_ones`; `run::a_fixture_the_target_s_platform_lacks_is_refused`; `run::a_cli_that_is_not_there_is_refused`; `run::run_takes_targets_or_a_fake_script_but_not_both`; `plan::tests::each_named_target_runs_its_platform_s_fixtures_by_default`; `plan::tests::chosen_fixtures_run_on_every_named_target`; `plan::tests::a_target_nobody_connected_is_refused_naming_the_connected_ones`; `plan::tests::a_fixture_a_target_s_platform_lacks_is_refused`; `cli::tests::devices_give_each_target_s_id_class_model_and_os`; `fixture::tests::app_ids_follow_the_fixture_readmes` |
| Reference fixtures: golden end states, checked before timing | `golden::every_adapter_reports_the_golden_end_state_of_each_flow_on_android`; `golden::every_adapter_reports_the_golden_end_state_of_each_flow_on_ios`; `golden::a_run_times_each_flow_only_after_every_tool_read_its_golden`; `golden::a_tool_whose_end_state_differs_from_the_golden_is_not_timed`; `golden::the_tool_under_test_is_held_to_the_golden_too`; `golden::a_flow_without_a_golden_is_timed_unchecked`; the `end_state::tests` |
| Reference fixtures: flows described once | `cli_adapter::the_reference_flows_run_through_the_cli_adapter_on_a_fake_emulator`; `cli_adapter::the_reference_flows_run_through_the_cli_adapter_on_a_fake_simulator`; `flow::tests::a_flow_reads_from_its_documented_json_shape`; `flows::the_login_flow_signs_in_and_ends_on_the_welcome_text`; `flows::the_list_scroll_flow_scrolls_to_row_200_and_opens_its_detail`; `flows::the_500_node_flow_opens_the_big_tab_and_records_its_node_count`; `flows::the_reference_flows_come_in_report_order_and_differ_only_in_their_fixture`; `flows::a_flow_and_its_golden_read_and_write_their_documented_json`; `flows::maestro_runs_each_reference_flow_as_one_flow_file`; `flows::agent_device_runs_each_reference_flow_as_one_batch`; `flows::a_run_measures_the_same_flow_on_several_fixtures`; `flows::a_run_refuses_two_flows_with_one_name_on_one_fixture`; `flows::a_flow_series_must_name_a_fixture_the_flow_has` |
| Reproducible runs, "Report provenance" | `measurement::provenance_is_recorded_in_both_outputs`; `measurement::provenance_tells_whether_two_environments_matched`; `measurement::warm_up_iterations_are_excluded_from_statistics`; the `provenance::tests` |
| Reliability metrics, "Intermittent failure" | `reliability::two_failed_iterations_of_fifty_taps_are_a_four_percent_failure_rate`; `reliability::a_failure_that_passes_on_retry_is_a_flake_not_a_failure`; `reliability::an_iteration_that_fails_every_retry_is_a_failure`; `reliability::warm_up_failures_are_not_counted`; `reliability::a_series_that_could_not_be_prepared_failed_every_iteration`; `reliability::the_rates_reach_both_outputs_through_the_binary` |
| Output formats, "Two outputs" | `run::a_run_measures_mobile_agent_on_a_fake_emulator_and_the_baselines_beside_it`; `run::a_run_on_a_fake_simulator_runs_swiftui_and_expo_through_the_cli_beside_the_bench`; `measurement::the_json_report_and_the_markdown_summary_agree_on_every_number`; `measurement::the_comparison_table_spans_tools_and_targets`; `measurement::the_report_round_trips_through_json` |
