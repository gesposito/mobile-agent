//! Task 3.2 of `ci-benchmarks`: the benchmarks run in CI only on a schedule
//! or a manual dispatch, never on a push or a pull request (design D1), with
//! the dispatch's inputs, one job per suite and runner kind, the harness
//! jobs only when the suite asks for them and bounded by 90 minutes, and the
//! reports kept as artifacts for 90 days. Task 1.1 of `pass-4-fixes`: the
//! device-backed tests run the same way, on the runners of the benchmarks'
//! harness jobs (design D1), and a dispatch can narrow a run to one job, one
//! package and a test filter (task 3.4). Task 3.6 of `ci-benchmarks`: a
//! change to documentation alone skips CI's Linux and macOS jobs, whose
//! checks keep reporting, and the device tests' emulator teardown passes
//! once the runner's step has stopped the emulator. The workflow files are read with an
//! outline of YAML's block mappings, enough for their keys: indentation opens
//! a key's children, a list item holds a value, and a block scalar's lines,
//! more indented than its key, are skipped; a job's steps are read as text.

use std::fs;
use std::path::Path;

/// One key of a block mapping, its value on the same line, if any, and its
/// children: keys more indented beneath it, or list items.
#[derive(Debug, Default)]
struct Node {
    key: String,
    value: Option<String>,
    children: Vec<Node>,
    items: Vec<String>,
}

impl Node {
    /// The child `key`.
    fn get(&self, key: &str) -> &Node {
        self.children
            .iter()
            .find(|child| child.key == key)
            .unwrap_or_else(|| panic!("no key {key:?} under {:?}", self.key))
    }

    /// The node at `path`, keys separated by `/`.
    fn at(&self, path: &str) -> &Node {
        path.split('/').fold(self, |node, key| node.get(key))
    }

    /// The children's keys, in order.
    fn keys(&self) -> Vec<&str> {
        self.children
            .iter()
            .map(|child| child.key.as_str())
            .collect()
    }

    /// The value, unquoted.
    fn text(&self) -> &str {
        unquoted(
            self.value
                .as_deref()
                .unwrap_or_else(|| panic!("{:?} has no value", self.key)),
        )
    }
}

/// `text` without the double quotes around it, if it has them.
fn unquoted(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(text)
}

/// The outline of the YAML file at `path`, relative to the repository root.
fn outline(path: &str) -> Node {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    let mut root = Node::default();
    // The path of open keys, each with its indentation.
    let mut open: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut scalar: Option<usize> = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let content = line.trim();
        if let Some(at) = scalar {
            if content.is_empty() || indent > at {
                continue;
            }
            scalar = None;
        }
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        while open.last().is_some_and(|(at, _)| *at >= indent) {
            open.pop();
        }
        let parent = open
            .last()
            .map(|(_, path)| path.clone())
            .unwrap_or_default();
        let node = parent
            .iter()
            .fold(&mut root, |node, &i| &mut node.children[i]);
        if let Some(item) = content.strip_prefix("- ") {
            node.items.push(unquoted(item).to_owned());
            continue;
        }
        let Some((key, value)) = content.split_once(':') else {
            panic!("{path}: cannot outline {line:?}");
        };
        let value = value.trim();
        if value == "|" || value == ">" {
            scalar = Some(indent);
        }
        node.children.push(Node {
            key: unquoted(key.trim()).to_owned(),
            value: (!value.is_empty()).then(|| value.to_owned()),
            ..Node::default()
        });
        let mut path = parent;
        path.push(node.children.len() - 1);
        open.push((indent, path));
    }
    root
}

#[test]
fn the_benchmarks_workflow_runs_on_a_schedule_or_a_dispatch_and_never_on_a_push_or_a_pull_request()
{
    let workflow = outline(".github/workflows/benchmarks.yml");
    assert_eq!(workflow.get("on").keys(), ["schedule", "workflow_dispatch"]);
    // Weekly, on Sundays, off the start of the hour.
    let cron = &workflow.at("on/schedule").items;
    assert_eq!(cron.len(), 1, "{cron:?}");
    assert_eq!(cron[0], "cron: \"17 2 * * 0\"");

    let inputs = workflow.at("on/workflow_dispatch/inputs");
    assert_eq!(inputs.keys(), ["suite", "base", "head", "iterations"]);
    let suite = inputs.get("suite");
    assert_eq!(suite.get("type").text(), "choice");
    assert_eq!(suite.get("options").items, ["micro", "harness", "both"]);
    assert_eq!(suite.get("default").text(), "micro");
    for optional in ["base", "head", "iterations"] {
        assert_eq!(inputs.get(optional).get("required").text(), "false");
    }

    // One job resolves the refs; then one per suite and runner kind.
    let jobs = workflow.get("jobs");
    assert_eq!(
        jobs.keys(),
        [
            "resolve",
            "micro-linux",
            "micro-macos",
            "harness-simulator",
            "harness-emulator"
        ]
    );
    // Each runs unless the suite is the other one.
    for (job, runner, unless) in [
        ("micro-linux", "ubuntu-latest", "harness"),
        ("micro-macos", "macos-latest", "harness"),
        ("harness-simulator", "xcode-27", "micro"),
        ("harness-emulator", "ubuntu-latest", "micro"),
    ] {
        let job = jobs.get(job);
        assert_eq!(job.get("runs-on").text(), runner, "{}", job.key);
        assert_eq!(job.get("needs").text(), "resolve", "{}", job.key);
        assert_eq!(
            job.get("if").text(),
            format!("needs.resolve.outputs.suite != '{unless}'"),
            "{}",
            job.key
        );
        // Each keeps its reports for 90 days under its own name.
        let upload = job
            .get("steps")
            .items
            .iter()
            .filter(|item| item.starts_with("uses: actions/upload-artifact@"))
            .count();
        assert_eq!(upload, 1, "{}", job.key);
    }
    for job in ["harness-simulator", "harness-emulator"] {
        assert_eq!(jobs.get(job).get("timeout-minutes").text(), "90", "{job}");
    }
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/benchmarks.yml"),
    )
    .unwrap();
    assert_eq!(text.matches("retention-days: 90").count(), 4);
    for name in [
        "micro-linux",
        "micro-macos",
        "harness-simulator",
        "harness-emulator",
    ] {
        assert!(text.contains(&format!("name: {name}\n")), "{name}");
    }
    // The harness emulator boots on the measured AVD's device, whose display,
    // 1080x2400 at 420 dpi, the gates were measured on; without a profile
    // avdmanager makes a 320x640 display at 160 dpi.
    assert!(
        text.contains("profile: pixel_8\n"),
        "the harness emulator's AVD profile"
    );
}

#[test]
fn the_validation_workflow_runs_no_benchmark_and_says_where_they_are() {
    let ci = outline(".github/workflows/ci.yml");
    assert_eq!(
        ci.get("on").keys(),
        ["push", "pull_request", "workflow_dispatch"]
    );
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/ci.yml"),
    )
    .unwrap();
    assert!(
        text.contains("benchmarks.yml"),
        "ci.yml names benchmarks.yml"
    );
    assert!(
        !text.contains("mobile-agent-bench"),
        "ci.yml runs no benchmark"
    );
    assert!(!text.contains("just micro"), "ci.yml runs no benchmark");
}

/// The text of the workflow at `path`, relative to the repository root.
fn read(path: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// The steps of the job `job` in a workflow's `text`, each as its lines from
/// its `- ` at six spaces of indentation on.
fn steps(text: &str, job: &str) -> Vec<String> {
    let header = format!("  {job}:");
    let mut steps: Vec<String> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        if indent <= 2 && !line.trim().is_empty() && !line.trim_start().starts_with('#') {
            inside = line == header;
            continue;
        }
        if !inside {
            continue;
        }
        if line.starts_with("      - ") {
            steps.push(format!("{line}\n"));
        } else if let Some(step) = steps.last_mut() {
            step.push_str(line);
            step.push('\n');
        }
    }
    steps
}

/// The one step of `steps` holding every text of `wanted`.
#[track_caller]
fn step_with<'a>(steps: &'a [String], wanted: &[&str]) -> &'a str {
    let found: Vec<&String> = steps
        .iter()
        .filter(|step| wanted.iter().all(|text| step.contains(text)))
        .collect();
    assert_eq!(found.len(), 1, "one step holds {wanted:?}: {found:#?}");
    found[0]
}

#[test]
fn the_device_tests_run_on_fresh_devices_on_a_schedule_or_a_dispatch_with_their_gates_set() {
    let workflow = outline(".github/workflows/device-tests.yml");
    assert_eq!(workflow.get("on").keys(), ["schedule", "workflow_dispatch"]);
    // Weekly, on Wednesdays, off the start of the hour and days from the
    // benchmarks' Sunday run.
    assert_eq!(workflow.at("on/schedule").items, ["cron: \"23 3 * * 3\""]);
    // A second run waits for the first.
    assert_eq!(
        workflow.at("concurrency/cancel-in-progress").text(),
        "false"
    );
    // A shared runner is not the host the tests' millisecond bounds were set
    // for: both jobs record those measurements without judging them.
    assert_eq!(workflow.at("env/MOBILE_AGENT_HOST_GATES").text(), "off");

    // One job per device, on the runner of the benchmarks' harness job that
    // provisions the same device. The Simulator's tests boot devices of their
    // own, one per test binary that needs a first boot, each taking minutes
    // on the runner, so its job is bounded at 150 minutes, the emulator's at
    // 90.
    let benchmarks = outline(".github/workflows/benchmarks.yml");
    let jobs = workflow.get("jobs");
    assert_eq!(jobs.keys(), ["simulator", "emulator"]);
    for (job, harness, bound) in [
        ("simulator", "harness-simulator", "150"),
        ("emulator", "harness-emulator", "90"),
    ] {
        let harness = benchmarks.at(&format!("jobs/{harness}"));
        assert_eq!(
            jobs.at(&format!("{job}/runs-on")).text(),
            harness.get("runs-on").text(),
            "{job}"
        );
        assert_eq!(
            jobs.at(&format!("{job}/timeout-minutes")).text(),
            bound,
            "{job}"
        );
    }

    let text = read(".github/workflows/device-tests.yml");
    let benchmarks_text = read(".github/workflows/benchmarks.yml");
    for job in ["simulator", "emulator"] {
        let steps = steps(&text, job);
        // The tests the dispatch asks for, every test of the workspace on a
        // schedule, past a failure.
        step_with(
            &steps,
            &["cargo test $TEST_SCOPE --no-fail-fast -- $TEST_FILTER"],
        );
        // The counts per crate in the summary, whatever happened.
        step_with(
            &steps,
            &[
                "if: always()",
                "scripts/test-summary.sh",
                "$GITHUB_STEP_SUMMARY",
            ],
        );
    }

    // The Simulator: created as the benchmarks' is, its UDID the gate, both
    // fixtures installed, and deleted whatever happened.
    let simulator = steps(&text, "simulator");
    let created = step_with(&simulator, &["xcrun simctl create"]);
    assert!(created.contains("\"iPhone 17\""), "{created}");
    assert!(
        created.contains("MOBILE_AGENT_SIM_UDID=") && created.contains("$GITHUB_ENV"),
        "{created}"
    );
    for fixture in ["$MOBILE_AGENT_SWIFTUI_APP", "$MOBILE_AGENT_EXPO_APP"] {
        assert!(
            created.contains(&format!(
                "xcrun simctl install \"$MOBILE_AGENT_SIM_UDID\" \"{fixture}\""
            )),
            "{created}"
        );
    }
    step_with(
        &simulator,
        &[
            "if: always()",
            "xcrun simctl delete \"$MOBILE_AGENT_SIM_UDID\"",
        ],
    );

    // The emulator: booted by the same pinned runner as the benchmarks', with
    // its controller on, on a Pixel 8, its serial the gate, both fixtures
    // installed, and shut down whatever happened.
    let emulator = steps(&text, "emulator");
    let runner = benchmarks_text
        .lines()
        .find(|line| line.contains("uses: reactivecircus/android-emulator-runner@"))
        .expect("benchmarks.yml boots its emulator through the runner")
        .trim();
    let booted = step_with(&emulator, &[runner]);
    assert!(booted.contains("-grpc 8554 -grpc-use-token"), "{booted}");
    // The measured AVD's display, 1080x2400 at 420 dpi, which the Android
    // driver's tests assume.
    assert!(booted.contains("profile: pixel_8\n"), "{booted}");
    for apk in ["$COMPOSE_APK", "$EXPO_APK"] {
        assert!(
            booted.contains(&format!("adb -s emulator-5554 install -r \"{apk}\"")),
            "{booted}"
        );
    }
    assert_eq!(
        jobs.at("emulator/env/MOBILE_AGENT_ANDROID_SERIAL").text(),
        "emulator-5554"
    );
    // The runner's step stops the emulator when its script ends, and adb can
    // still list emulator-5554 once the console `emu kill` talks to has
    // closed: the teardown kills only an emulator whose console on port 5554
    // answers, and a kill that fails because the console closed in between
    // passes.
    let teardown = step_with(&emulator, &["if: always()", "emu kill"]);
    assert!(
        teardown.contains("(exec 3<> /dev/tcp/127.0.0.1/5554)"),
        "{teardown}"
    );
    assert!(teardown.contains("if ! console; then"), "{teardown}");
    assert!(
        teardown.contains(
            "elif ! \"$ANDROID_HOME/platform-tools/adb\" -s emulator-5554 emu kill && console; then"
        ),
        "{teardown}"
    );
    assert!(teardown.contains("exit 1\n"), "{teardown}");
    assert!(!teardown.contains(" devices "), "{teardown}");
}

#[test]
fn a_device_tests_dispatch_picks_its_runners_its_package_and_its_tests() {
    let workflow = outline(".github/workflows/device-tests.yml");
    let inputs = workflow.at("on/workflow_dispatch/inputs");
    assert_eq!(inputs.keys(), ["tests", "crate", "runner"]);
    for optional in ["tests", "crate"] {
        let input = inputs.get(optional);
        assert_eq!(input.get("type").text(), "string", "{optional}");
        assert_eq!(input.get("required").text(), "false", "{optional}");
    }
    let runner = inputs.get("runner");
    assert_eq!(runner.get("type").text(), "choice");
    assert_eq!(
        runner.get("options").items,
        ["simulator", "emulator", "both"]
    );
    assert_eq!(runner.get("default").text(), "both");

    // Each job runs unless the dispatch asks for the other one alone; a
    // scheduled run has no inputs and runs both.
    let jobs = workflow.get("jobs");
    for (job, other) in [("simulator", "emulator"), ("emulator", "simulator")] {
        assert_eq!(
            jobs.at(&format!("{job}/if")).text(),
            format!("inputs.runner != '{other}'"),
            "{job}"
        );
    }

    // The package and the filter reach the shell through the environment,
    // checked before they become its words: the package as the scope, the
    // whole workspace without one, and the filter after `--`, every test
    // without one.
    let text = read(".github/workflows/device-tests.yml");
    for job in ["simulator", "emulator"] {
        let steps = steps(&text, job);
        let resolve = step_with(&steps, &["TEST_SCOPE=", "TEST_FILTER="]);
        for wanted in [
            "CRATE: ${{ inputs.crate }}",
            "TESTS: ${{ inputs.tests }}",
            "::error::",
            "$GITHUB_ENV",
            "TEST_SCOPE=--workspace",
            "TEST_SCOPE=-p $CRATE",
        ] {
            assert!(resolve.contains(wanted), "{job}: {wanted} in {resolve}");
        }
        // The build of the test binaries, whose messages name each binary's
        // package for the summary, covers the same scope.
        step_with(&steps, &["cargo test $TEST_SCOPE --no-run"]);
        assert!(
            !steps.iter().any(|step| step.contains("--workspace --no")),
            "{job}: no step tests the whole workspace whatever the dispatch asked"
        );
    }
}

/// Task 3.6 of `ci-benchmarks`: a change to documentation alone skips CI's
/// Linux and macOS jobs at the job level, so that their checks, which the
/// ruleset `main` requires, report skipped, which passes them, rather than
/// missing, as a workflow-level path filter would leave them. A first job
/// classifies the change; the heavy jobs run unless it answered
/// documentation alone, so that a classifier that failed runs them; Audit
/// runs on every change and holds the documentation's own checks.
#[test]
fn a_change_to_documentation_alone_skips_the_heavy_ci_jobs_and_keeps_their_checks_reporting() {
    let ci = outline(".github/workflows/ci.yml");
    let jobs = ci.get("jobs");
    assert_eq!(
        jobs.keys(),
        ["changes", "audit", "linux", "macos-26", "macos-27"]
    );
    let text = read(".github/workflows/ci.yml");

    // The classifier: quick, reading the contents alone, on a pull request's
    // merge commit and its two parents; a push to main and a dispatch always
    // count as code.
    let changes = jobs.get("changes");
    assert_eq!(changes.get("runs-on").text(), "ubuntu-latest");
    assert_eq!(changes.get("timeout-minutes").text(), "5");
    assert_eq!(changes.at("permissions/contents").text(), "read");
    assert_eq!(
        changes.at("outputs/code").text(),
        "${{ steps.classify.outputs.code }}"
    );
    let classifier = steps(&text, "changes");
    let checkout = step_with(&classifier, &["uses: actions/checkout@"]);
    for wanted in [
        "if: github.event_name == 'pull_request'",
        "fetch-depth: 2\n",
    ] {
        assert!(checkout.contains(wanted), "{wanted} in {checkout}");
    }
    let classify = step_with(&classifier, &["id: classify"]);
    for wanted in [
        "EVENT: ${{ github.event_name }}",
        "scripts/ci-changes.sh HEAD^1 HEAD >> \"$GITHUB_OUTPUT\"",
        "echo \"code=true\" >> \"$GITHUB_OUTPUT\"",
    ] {
        assert!(classify.contains(wanted), "{wanted} in {classify}");
    }

    // The required checks are the jobs' names, one job each: a job's `if` is
    // evaluated before its matrix expands, so a skipped matrix job would
    // report one check under its unexpanded name and leave the legs' checks
    // missing.
    let names: Vec<&str> = ["audit", "linux", "macos-26", "macos-27"]
        .iter()
        .map(|job| jobs.at(&format!("{job}/name")).text())
        .collect();
    assert_eq!(
        names,
        ["Audit", "Linux", "macOS (Xcode 26)", "macOS (Xcode 27)"]
    );
    for job in ["changes", "audit", "linux", "macos-26", "macos-27"] {
        assert!(
            !jobs.get(job).keys().contains(&"strategy"),
            "{job} has no matrix"
        );
    }
    for job in ["linux", "macos-26", "macos-27"] {
        let heavy = jobs.get(job);
        assert_eq!(heavy.get("needs").text(), "changes", "{job}");
        assert_eq!(
            heavy.get("if").text(),
            "${{ !cancelled() && needs.changes.outputs.code != 'false' }}",
            "{job}"
        );
    }

    // The two macOS jobs run the same steps, each on its image with its
    // Xcode.
    let macos = steps(&text, "macos-26");
    assert!(macos.len() > 5, "{macos:#?}");
    // The blank line between the jobs aside.
    let trimmed = |steps: &[String]| -> Vec<String> {
        steps
            .iter()
            .map(|step| step.trim_end().to_owned())
            .collect()
    };
    assert_eq!(trimmed(&macos), trimmed(&steps(&text, "macos-27")));
    for (job, runner, xcode) in [
        ("macos-26", "macos-26", "26"),
        ("macos-27", "xcode-27", "27"),
    ] {
        let leg = jobs.get(job);
        assert_eq!(leg.get("runs-on").text(), runner, "{job}");
        assert_eq!(leg.at("env/XCODE_MAJOR").text(), xcode, "{job}");
        assert_eq!(leg.get("timeout-minutes").text(), "30", "{job}");
    }
    step_with(&macos, &["just validate", "FIXTURES: expo,swiftui"]);

    // Audit runs on every change, and with it the checks a change to the
    // documentation needs: the publish scan and the OpenSpec validation, the
    // commands of the justfile's recipes.
    let audit = jobs.get("audit");
    for key in ["needs", "if"] {
        assert!(!audit.keys().contains(&key), "audit has no {key}");
    }
    let audit_steps = steps(&text, "audit");
    step_with(&audit_steps, &["npm ci --prefix .github/tools"]);
    step_with(&audit_steps, &["run: scripts/check-public.sh"]);
    step_with(&audit_steps, &["run: openspec validate --all --strict"]);
    let justfile = read("justfile");
    assert!(
        justfile.contains("\ncheck-public:\n    scripts/check-public.sh\n"),
        "just check-public runs the script"
    );
    assert!(
        justfile.contains("\nopenspec-validate:\n    openspec validate --all --strict\n"),
        "just openspec-validate runs the validation"
    );
}
