//! Task 3.1 of `open-sourcing`: the workflows as Astral's practices and
//! zizmor's audits want them (design D4). Every workflow starts without
//! permissions and each job asks for the least it needs, every action is
//! pinned to a full commit SHA with its version beside it, no checkout keeps
//! its credentials, and the tools come from lockfiles or from releases
//! pinned with their SHA-256; CI lints and audits the workflows. Task 3.6 of
//! `ci-benchmarks`: a checkout fetches the head commit alone unless its job
//! reads history, and no step waits a fixed time. The files
//! are read as text, line by line: a job is the block under `jobs:` that
//! starts at two spaces of indentation, a step the block that starts with
//! `- ` at six.

use std::fs;
use std::path::Path;

use serde_json::Value;

const WORKFLOWS: [&str; 3] = [
    ".github/workflows/ci.yml",
    ".github/workflows/benchmarks.yml",
    ".github/workflows/device-tests.yml",
];

/// The text of `path`, relative to the repository root.
fn read(path: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// The jobs of a workflow: each one's name and its lines.
fn jobs(text: &str) -> Vec<(String, Vec<&str>)> {
    let mut jobs: Vec<(String, Vec<&str>)> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            inside = line == "jobs:";
            continue;
        }
        if !inside {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 2 && line.trim_end().ends_with(':') && !line.trim_start().starts_with('#') {
            jobs.push((line.trim().trim_end_matches(':').to_owned(), Vec::new()));
        } else if let Some((_, lines)) = jobs.last_mut() {
            lines.push(line);
        }
    }
    jobs
}

/// The steps of a job: the lines of each, from its `- ` on.
fn steps<'a>(lines: &[&'a str]) -> Vec<Vec<&'a str>> {
    let mut steps: Vec<Vec<&'a str>> = Vec::new();
    for &line in lines {
        if line.starts_with("      - ") {
            steps.push(vec![line]);
        } else if let Some(step) = steps.last_mut() {
            step.push(line);
        }
    }
    steps
}

/// Whether `value` is `owner/repo[/path]@<40 hex digits> # v<version>`.
fn pinned(value: &str) -> bool {
    let Some((action, rest)) = value.split_once('@') else {
        return false;
    };
    let Some((sha, comment)) = rest.split_once(" # v") else {
        return false;
    };
    action.split('/').count() >= 2
        && sha.len() == 40
        && sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && !comment.is_empty()
        && comment
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

#[test]
fn every_workflow_starts_without_permissions_and_each_job_asks_for_the_least_it_needs() {
    for path in WORKFLOWS {
        let text = read(path);
        let top: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("permissions:"))
            .collect();
        assert_eq!(top, ["permissions: {}"], "{path}");
        let jobs = jobs(&text);
        assert!(!jobs.is_empty(), "{path} has jobs");
        for (job, lines) in &jobs {
            let at = lines
                .iter()
                .position(|line| *line == "    permissions:")
                .unwrap_or_else(|| panic!("{path}: job {job} declares its permissions"));
            let granted: Vec<&str> = lines[at + 1..]
                .iter()
                .take_while(|line| {
                    line.starts_with("      ") && !line.trim_start().starts_with('-')
                })
                .filter(|line| !line.trim_start().starts_with('#'))
                // The scope without the comment that explains it.
                .map(|line| line.split(" #").next().unwrap_or(line).trim())
                .collect();
            assert!(
                granted.contains(&"contents: read"),
                "{path}: {job} {granted:?}"
            );
            for permission in &granted {
                assert!(
                    permission.ends_with(": read"),
                    "{path}: {job} grants {permission}"
                );
            }
            // A job that reads this repository's workflow runs and their
            // artifacts through the API needs the actions scope, and only it.
            let uses_api = lines.iter().any(|line| {
                line.contains("/actions/workflows/") || line.contains("previous-artifact.sh")
            });
            assert_eq!(
                granted.contains(&"actions: read"),
                uses_api,
                "{path}: {job} {granted:?}"
            );
        }
    }
}

#[test]
fn every_action_is_pinned_to_a_full_commit_sha_with_its_version_beside_it() {
    let mut count = 0;
    for path in WORKFLOWS {
        for line in read(path).lines() {
            let line = line.trim_start().trim_start_matches("- ");
            if let Some(value) = line.strip_prefix("uses: ") {
                count += 1;
                assert!(pinned(value.trim()), "{path}: {value}");
            }
        }
    }
    assert!(count >= 19, "{count} actions");
}

#[test]
fn no_checkout_keeps_its_credentials() {
    for path in WORKFLOWS {
        let text = read(path);
        for (job, lines) in jobs(&text) {
            for step in steps(&lines) {
                if step
                    .iter()
                    .any(|line| line.contains("uses: actions/checkout@"))
                {
                    assert!(
                        step.iter()
                            .any(|line| line.trim() == "persist-credentials: false"),
                        "{path}: a checkout of {job} keeps its credentials"
                    );
                }
            }
        }
    }
}

#[test]
fn tools_come_from_lockfiles_or_from_releases_pinned_with_their_sha_256() {
    for path in WORKFLOWS {
        let text = read(path);
        assert!(
            !text.contains("/releases/latest/"),
            "{path} downloads a latest release"
        );
        assert!(
            !text.contains("bun-version: latest"),
            "{path} installs the latest Bun"
        );
        for (job, lines) in jobs(&text) {
            for step in steps(&lines) {
                let body = step.join("\n");
                if body.contains("curl ") {
                    assert!(
                        body.contains("sha256sum -c") || body.contains("shasum -a 256 -c"),
                        "{path}: {job} downloads without checking the SHA-256:\n{body}"
                    );
                }
                for line in &step {
                    let command = line.trim();
                    if command.starts_with("cargo install ") {
                        assert!(command.contains("--locked"), "{path}: {command}");
                        assert!(
                            command.split_whitespace().any(|word| word.contains('@')),
                            "{path}: {command} names no version"
                        );
                    }
                    assert!(
                        !command.starts_with("npm install"),
                        "{path}: {command} installs outside a lockfile"
                    );
                }
            }
        }
    }
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("npm ci --prefix .github/tools"),
        "ci.yml installs OpenSpec from .github/tools"
    );
    let lock: Value = serde_json::from_str(&read(".github/tools/package-lock.json")).unwrap();
    let openspec = &lock["packages"]["node_modules/@fission-ai/openspec"];
    assert_eq!(openspec["version"], "1.13.1", "{openspec}");
    assert!(
        openspec["integrity"]
            .as_str()
            .is_some_and(|i| i.starts_with("sha512-")),
        "{openspec}"
    );
    let manifest: Value = serde_json::from_str(&read(".github/tools/package.json")).unwrap();
    assert_eq!(manifest["dependencies"]["@fission-ai/openspec"], "1.13.1");
}

#[test]
fn ci_lints_and_audits_the_workflows() {
    let ci = read(".github/workflows/ci.yml");
    let audit = jobs(&ci)
        .into_iter()
        .find(|(_, lines)| {
            lines
                .iter()
                .any(|line| line.trim() == "run: zizmor --no-progress .github")
        })
        .expect("a job runs zizmor over .github");
    assert!(
        audit.1.iter().any(|line| line.trim() == "run: actionlint"),
        "the audit job runs actionlint"
    );
    // zizmor's online audits, impostor-commit among them, need a token.
    assert!(
        audit
            .1
            .iter()
            .any(|line| line.trim() == "GH_TOKEN: ${{ github.token }}"),
        "zizmor runs with the job's token"
    );
}

/// Task 3.2: Dependabot proposes new versions of the actions, the crates and
/// the toolchain after 7 days, and CI checks the dependencies' advisories,
/// licenses, bans and sources with cargo-deny, for both Cargo workspaces.
#[test]
fn dependabot_waits_a_week_and_ci_checks_the_dependencies_with_cargo_deny() {
    let dependabot = read(".github/dependabot.yml");
    let mut ecosystems = Vec::new();
    for entry in dependabot.split("\n  - package-ecosystem: ").skip(1) {
        let name = entry.lines().next().unwrap().trim().to_owned();
        assert!(
            entry.contains("    cooldown:\n      default-days: 7\n"),
            "{name} waits 7 days"
        );
        assert!(
            entry.contains("      interval: weekly\n"),
            "{name} runs weekly"
        );
        ecosystems.push(name);
    }
    assert_eq!(ecosystems, ["rust-toolchain", "github-actions", "cargo"]);

    let ci = read(".github/workflows/ci.yml");
    let (_, audit) = jobs(&ci)
        .into_iter()
        .find(|(name, _)| name == "audit")
        .expect("ci.yml has an audit job");
    let runs: Vec<&str> = audit
        .iter()
        .map(|line| line.trim().trim_start_matches("run: "))
        .collect();
    assert!(
        runs.contains(&"cargo deny check advisories licenses bans sources"),
        "{runs:#?}"
    );
    assert!(
        runs.contains(
            &"cargo deny --manifest-path agent-android/host-tests/Cargo.toml check -A license-not-encountered advisories licenses bans sources"
        ),
        "{runs:#?}"
    );
    assert!(read("deny.toml").contains("[licenses.private]\nignore = true\n"));
}

/// Task 3.6 of `ci-benchmarks`: each checkout fetches the head commit alone,
/// checkout's default depth of 1, unless its job reads history: the
/// benchmarks' `resolve`, which resolves any branch, tag or commit, and
/// their micro jobs, which check the base out from the same clone, fetch
/// all of it, and CI's classifier fetches a pull request's merge commit with
/// its two parents. The benchmarks' measuring jobs name their depth.
#[test]
fn each_checkout_fetches_the_head_alone_unless_its_job_reads_history() {
    let depths = [
        (".github/workflows/ci.yml", "changes", "2"),
        (".github/workflows/ci.yml", "audit", "1"),
        (".github/workflows/ci.yml", "linux", "1"),
        (".github/workflows/ci.yml", "macos-26", "1"),
        (".github/workflows/ci.yml", "macos-27", "1"),
        (".github/workflows/benchmarks.yml", "resolve", "0"),
        (".github/workflows/benchmarks.yml", "micro-linux", "0"),
        (".github/workflows/benchmarks.yml", "micro-macos", "0"),
        (".github/workflows/benchmarks.yml", "harness-simulator", "1"),
        (".github/workflows/benchmarks.yml", "harness-emulator", "1"),
        (".github/workflows/device-tests.yml", "simulator", "1"),
        (".github/workflows/device-tests.yml", "emulator", "1"),
    ];
    let mut seen = Vec::new();
    for path in WORKFLOWS {
        let text = read(path);
        for (job, lines) in jobs(&text) {
            for step in steps(&lines) {
                if !step
                    .iter()
                    .any(|line| line.contains("uses: actions/checkout@"))
                {
                    continue;
                }
                let named = step
                    .iter()
                    .find_map(|line| line.trim().strip_prefix("fetch-depth: "));
                let (_, _, wanted) = depths
                    .iter()
                    .find(|(p, j, _)| *p == path && *j == job)
                    .unwrap_or_else(|| {
                        panic!("{path}: {job} checks out with no depth listed here")
                    });
                assert_eq!(named.unwrap_or("1"), *wanted, "{path}: {job}");
                if path.ends_with("benchmarks.yml") {
                    assert!(named.is_some(), "{path}: {job} names its depth");
                }
                seen.push((path, job.clone()));
            }
        }
    }
    assert_eq!(seen.len(), depths.len(), "{seen:?}");
}

/// Task 3.6 of `ci-benchmarks`: no step sleeps a fixed time; a step waits on
/// the condition it needs, a Simulator's boot through `simctl bootstatus
/// -b`, a download through curl's own bounded retries.
#[test]
fn no_step_waits_a_fixed_time() {
    for path in WORKFLOWS {
        for (number, line) in read(path).lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            let fixed = words.windows(2).any(|pair| {
                pair[0] == "sleep" && pair[1].starts_with(|c: char| c.is_ascii_digit())
            });
            assert!(!fixed, "{path}:{}: {line}", number + 1);
        }
    }
}
