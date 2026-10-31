//! Task 3.2 of `ci-benchmarks`: the benchmarks run in CI only on a schedule
//! or a manual dispatch, never on a push or a pull request (design D1), with
//! the dispatch's inputs, one job per suite and runner kind, the harness
//! jobs only when the suite asks for them and bounded by 90 minutes, and the
//! reports kept as artifacts for 90 days. Task 1.1 of `pass-4-fixes`: the
//! device-backed tests run the same way, on the runners of the benchmarks'
//! harness jobs (design D1). The workflow files are read with an outline of
//! YAML's block mappings, enough for their keys: indentation opens a key's
//! children, a list item holds a value, and a block scalar's lines, more
//! indented than its key, are skipped; a job's steps are read as text.

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

    // One job per device, on the runner of the benchmarks' harness job that
    // provisions the same device, bounded as that job is.
    let benchmarks = outline(".github/workflows/benchmarks.yml");
    let jobs = workflow.get("jobs");
    assert_eq!(jobs.keys(), ["simulator", "emulator"]);
    for (job, harness) in [
        ("simulator", "harness-simulator"),
        ("emulator", "harness-emulator"),
    ] {
        let harness = benchmarks.at(&format!("jobs/{harness}"));
        assert_eq!(
            jobs.at(&format!("{job}/runs-on")).text(),
            harness.get("runs-on").text(),
            "{job}"
        );
        assert_eq!(
            jobs.at(&format!("{job}/timeout-minutes")).text(),
            "90",
            "{job}"
        );
    }

    let text = read(".github/workflows/device-tests.yml");
    let benchmarks_text = read(".github/workflows/benchmarks.yml");
    for job in ["simulator", "emulator"] {
        let steps = steps(&text, job);
        // The workspace's tests, all of them, past a failure.
        step_with(&steps, &["cargo test --workspace --no-fail-fast"]);
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
    step_with(&emulator, &["if: always()", "emu kill"]);
}
