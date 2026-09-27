# bench

The benchmark harness of design decision D10 and the `benchmark-harness` spec. It measures the spec's primitives through adapters, one per tool, writes a JSON report and a markdown summary from the same data, judges the latency gates, and runs the same flows through Maestro, maestro-runner and agent-device for comparison.

The binary is `mobile-agent-bench`; `just bench <arguments>` runs it in release mode. Until the CLI adapter exists (task 9.4), the tool under test is the scripted fake adapter described below.

```sh
just bench run --fake run.json --out reports/my-run   # measure, write both reports, judge the gates
just bench gates reports/my-run/report.json           # judge an existing report again
just bench gates --gates gates.json reports/my-run/report.json
just bench default-gates > gates.json                 # the spec's defaults, as a gate file
just bench baselines                                  # which baseline tools are on PATH
```

`run` takes `--iterations` (default 50), `--warmup` (default 3), `--retries` (default 1), `--gates <file>` and `--no-baselines`; without `--out` it writes to `target/bench/<UTC start time>/`. It prints where the reports went and the gate table. Exit status: 0 when every gate passed, 1 when any failed, 2 when the harness itself could not run.

## What is measured

A run is a list of series. A series is one subject, a primitive or a flow, on one target, in a scenario (a fixture app and one of its screens), and every tool that supports it measures it: first the warm-up iterations, then the measured ones. A failed attempt is retried within its iteration up to `--retries` times.

| Primitive | Spec wording | Planned scenario |
|---|---|---|
| `cli-start` | CLI process start | none |
| `cold-attach` | cold attach | none |
| `first-snapshot` | first snapshot after application launch | a fixture's `big` screen |
| `warm-snapshot` | warm snapshot | a fixture's `big` screen, the spec's reference screen |
| `tap-dispatch` | tap dispatch, the Simulator gate's span | a fixture screen |
| `tap-to-frame-change` | tap-to-frame-change | a fixture screen |
| `tap-to-tree-delta` | tap-to-tree-delta | a fixture screen |
| `text-entry` | text entry of a fixed string | `login`; the string is `TEXT_ENTRY`, `abcdefghijklmnopqrstuvwxyz0123456789` |
| `screenshot` | screenshot | a fixture screen |
| `settle` | settle detection | a fixture screen |

The spec lists nine primitives; `tap-dispatch` is the tenth because the Simulator gate is on dispatch, as spike S3 recommended. The scenario column is where task 9.4's run plan is to measure each primitive; the harness itself takes any scenario a series names, and the default gates look for warm snapshots on the `big` screen. Fixtures and screens are those of `fixtures/CONTRACT.md`: `swiftui` (iOS), `expo` (both platforms) and `compose` (Android); `login`, `list`, `big`, `anim` and `system`.

Statistics follow the spikes: nearest-rank percentiles, so every reported value is a measured sample, over whole microseconds, with warm-up iterations excluded. Each series reports `n`, p50, p95 and max, the node counts the tool returned, a failure rate and a flake rate. An iteration whose first attempt succeeds passes; one that succeeds only on a retry is a flake, and its latency is the successful attempt's; one in which no attempt succeeds is a failure. Rates are percentages of the measured iterations, rounded half up to a tenth. A series whose preparation failed counts every measured iteration as failed.

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
| `samples[]` | per iteration: `iteration`, `warmup`, `outcome` (`pass`, `flake`, `fail`), `attempts`, `us`, `nodes`, `detail`, `errors` |

Provenance covers what the spec asks for: host hardware and OS from `sysctl` and `sw_vers` on macOS (`/proc` and `uname` on Linux), Xcode from `xcodebuild -version`, the commit from `git`, tool versions from each adapter's detection, and each target's model, OS and API level. An adapter may fill the target fields; the harness fills the rest from `adb -s <serial> shell getprop` or `xcrun simctl list devices --json`. `Provenance::environment_differences` lists the fields in which two runs' environments differ, ignoring times and load.

The markdown has a provenance table with targets and tools, a measurements table (tool, target, fixture, screen, subject, clock, n, p50, p95, max, Fail %, Flake %, nodes), a comparison table with one column per tool that measured something and one row per target, fixture, screen and subject, holding p50 / p95, a line naming skipped tools and why, and the gate table.

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
- A gate with no matching measurement is listed as not measured and does not fail the run; a matching series in which no iteration succeeded fails.
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
}
```

- The harness never times a call. `measure` returns a `Sample` holding the latency of exactly the primitive's span, the clock it read (`Wall` or `Tool`), the node count for snapshots and an optional detail such as a text entry method. Work outside the span, such as stopping the daemon before a cold attach or relaunching the app before a first snapshot, happens inside the call but outside the span.
- `detect` runs once per run, before any series; an `Absent` tool is skipped with its note. `supports` makes the harness skip combinations a tool cannot measure instead of recording failures. `prepare` runs once per series, before its warm-up; `Scenario` names the fixture and screen.
- An `AdapterError` is a failed attempt; the harness retries it within the iteration.

The CLI adapter will drive `mobile-agent` in `--json` mode, whose envelope mirrors agent-browser's (`docs/research/protocol.md`): `{success, data, error, code?, warning?}` on stdout for one command, and for `batch` an array of `{command, success, result, error, code?, warning?}`, with stop-on-error. `success: false` becomes an `AdapterError` carrying `code` and `error`. The spans it will time:

- wall clock around the process: `cli-start` (`status` against a running daemon, the overhead task 8.3 gates with hyperfine), `cold-attach` (after stopping the daemon and the device-side helper), `first-snapshot` (after relaunching the fixture on its `big` screen, until a snapshot holds `cell-500`), `warm-snapshot`, `text-entry` (`fill` of `TEXT_ENTRY`) and `screenshot`;
- the tool's own clock, from fields of the command's JSON `data`: `tap-dispatch`, `tap-to-frame-change`, `tap-to-tree-delta` and `settle`, spans only the daemon observes.

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

A selector is `{"id": …}`, the contract identifier, or `{"text": …}`, the exact visible text. Flows reach screens by tapping their tabs rather than through `FIXTURE_INITIAL_TAB`, because Maestro passes launch arguments to iOS apps as process arguments, not environment variables.

A golden (`end_state::EndState`) states what the final screen shows when the flow did its work, in three kinds of terms: the text an identified element shows (`texts`, `null` for no such element), whether an element has an identifier (`present`), and how many distinct identifiers `<prefix>-<n>` elements have (`counts`). As JSON it sits under the flow's `golden` key; a flow without one leaves the key out.

The spec's three flows, `flow::reference(fixture)` in report order, are the same on every fixture:

| Flow | Steps | Golden |
|---|---|---|
| `login` (`flow::login`) | launch; fill `login-username` with `agent`; fill `login-password` with `hunter2`; tap `login-submit`; wait until `login-status` shows `Welcome, agent` | `{"texts": {"login-status": "Welcome, agent"}}` |
| `list-scroll` (`flow::list_scroll`) | launch; tap `tab-list`; scroll down until `row-200` is visible; tap it; wait until `detail-200` shows `Detail 200` | `{"present": {"detail-200": true}}` |
| `big-screen` (`flow::big_screen`) | launch; tap `tab-big`; wait until `cell-500` is visible; record the node count | `{"counts": {"cell": 500}}` |

A run may hold the same flow for several fixtures. A flow series names the flow and a fixture the run has it for, and no screen; two flows with one name on one fixture are refused before anything runs.

## Baselines

Each baseline adapter finds its executable on `PATH`, following symbolic links, and reads its version from `--version`; a tool that is not found is skipped with the note `<tool> not found on PATH`, and the tools that ran are still measured and compared. Baselines run flows only, timed by wall clock around the tool's process, since none of them exposes the primitives' spans; the flow file is written to `<out>/flows/` and rewritten before every run.

| Tool | Flow as | Command |
|---|---|---|
| Maestro | a Maestro flow file: `appId`, then `launchApp`, `tapOn`, `inputText` (after a `tapOn` of its field), `scrollUntilVisible`, `assertVisible` | `maestro --device <id> test <file>` |
| maestro-runner | the same file, which it runs unchanged | `maestro-runner --platform <ios\|android> --device <id> test <file>` |
| agent-device | one batch of `open` (with `relaunch`), `press`, `fill`, `scroll` (with `until`), `wait` and `is` steps with their structured inputs | `agent-device batch --steps-file <file> --on-error stop --json --platform <p> --udid <udid>` or `--serial <serial>` `--session mobile-agent-bench` |

Maestro matches `id` and `text` as regular expressions, so selector values are escaped. Assertions wait, as Maestro's do: agent-device checks a visible element with `wait` on its selector, and a text with `wait` for the text followed by `is text` on the element. A Maestro or maestro-runner flow fails on a non-zero exit, reported with the last lines the tool printed; an agent-device flow fails on `success: false` in its JSON reply, reported with the failing step and error, and its `totalDurationMs` is kept as the sample's detail. Recording the node count of a flow's final screen through the baselines comes with 9.4's flows.

The command shapes come from the tools' documentation: the Maestro CLI reference and command pages (`maestro-cli/maestro-cli-commands-and-options.md`, `api-reference/commands-available/*.md` in github.com/mobile-dev-inc/maestro-docs), the maestro-runner README (github.com/devicelab-dev/maestro-runner), and agent-device's batching, selector and command pages (`website/docs/docs/*.md` in github.com/callstack/agent-device) together with `agent-device help batch` and `help open` of version 0.21.14. maestro-runner was not installed where these adapters were written: its `--device` flag comes from its documentation, and the form of its `--version` output is unverified.

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
| Measured primitives, "Primitive report" | `measurement::every_listed_primitive_appears_with_its_percentiles_and_sample_count`; `primitive::tests::every_listed_primitive_is_in_all_and_only_tap_dispatch_is_added`; the `stats::tests` |
| Latency gates, "Gate missed" | `gates::a_missed_gate_fails_the_run_and_the_table_names_the_gate_limit_and_value`; `gates::a_run_that_misses_a_gate_exits_non_zero_and_prints_the_table` |
| Latency gates, "All gates met" | `gates::when_every_gate_is_met_the_run_exits_zero_and_the_table_shows_each_passed`; `gates::a_run_that_meets_every_gate_exits_zero_through_the_binary` |
| Latency gates: the defaults, configurable per target class | `gates::the_defaults_are_the_spec_gates`; `gates::gates_are_configurable_per_target_class`; `gates::the_gates_command_judges_an_existing_report`; `gates::only_the_tool_under_test_is_gated`; `gates::a_gate_without_a_measurement_is_reported_but_does_not_fail`; `gates::a_primitive_that_never_succeeded_fails_its_gate`; `gates::a_value_equal_to_the_limit_misses_the_gate`; the `gates::tests` |
| Baseline tools, "One baseline missing" | `baselines::an_absent_tool_is_noted_as_skipped_and_the_tools_that_ran_are_still_compared`; `baselines::every_baseline_is_skipped_when_none_is_installed`; `baselines::a_run_through_the_binary_includes_the_installed_baselines`; `baselines::the_baselines_command_lists_each_tool_and_notes_absent_ones` |
| Baseline tools: the same flows through each tool's CLI | the `baseline::maestro::tests` and `baseline::agent_device::tests`; `baselines::a_flow_the_tool_fails_is_a_failed_iteration_with_its_output`; `baselines::agent_device_failures_are_read_from_its_json_reply`; `baselines::a_series_the_run_cannot_carry_out_is_refused_before_anything_runs`; the `baseline::tests` |
| Reference fixtures, "Large screen fixture" | `measurement::the_large_screen_records_the_node_count_the_tool_returned` for the tool under test; the baselines' counts and the flows' golden end states are task 9.4 |
| Reference fixtures: flows described once | `flow::tests::a_flow_reads_from_its_documented_json_shape`; `flows::the_login_flow_signs_in_and_ends_on_the_welcome_text`; `flows::the_list_scroll_flow_scrolls_to_row_200_and_opens_its_detail`; `flows::the_500_node_flow_opens_the_big_tab_and_records_its_node_count`; `flows::the_reference_flows_come_in_report_order_and_differ_only_in_their_fixture`; `flows::a_flow_and_its_golden_read_and_write_their_documented_json`; `flows::maestro_runs_each_reference_flow_as_one_flow_file`; `flows::agent_device_runs_each_reference_flow_as_one_batch`; `flows::a_run_measures_the_same_flow_on_several_fixtures`; `flows::a_run_refuses_two_flows_with_one_name_on_one_fixture`; `flows::a_flow_series_must_name_a_fixture_the_flow_has` |
| Reproducible runs, "Report provenance" | `measurement::provenance_is_recorded_in_both_outputs`; `measurement::provenance_tells_whether_two_environments_matched`; `measurement::warm_up_iterations_are_excluded_from_statistics`; the `provenance::tests` |
| Reliability metrics, "Intermittent failure" | `reliability::two_failed_iterations_of_fifty_taps_are_a_four_percent_failure_rate`; `reliability::a_failure_that_passes_on_retry_is_a_flake_not_a_failure`; `reliability::an_iteration_that_fails_every_retry_is_a_failure`; `reliability::warm_up_failures_are_not_counted`; `reliability::a_series_that_could_not_be_prepared_failed_every_iteration`; `reliability::the_rates_reach_both_outputs_through_the_binary` |
| Output formats, "Two outputs" | `measurement::the_json_report_and_the_markdown_summary_agree_on_every_number`; `measurement::the_comparison_table_spans_tools_and_targets`; `measurement::the_report_round_trips_through_json` |
