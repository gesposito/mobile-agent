//! Microbenchmarks of the daemon's JSON wire and of a snapshot through its
//! socket (design D14), run by `mobile-agent-bench micro`: a warm
//! interactive snapshot of the fake fixture's Big tab, its 500 cells and five
//! tabs, through the socket of a daemon on a fake target, one request per
//! iteration, the host's whole path less the CLI process; an interactive
//! snapshot of the fixture's Anim tab, whose spinner keeps the frames
//! changing, through the socket of a daemon on a fake Simulator, whose
//! windows send no change events, so that each request finds the frames
//! changed since the latest read of the tree (design D3 of `profiling`);
//! `snapshot -i --delta` of the Big tab with nothing changed since the
//! delta before it, through the same socket;
//! `wire::parse` of the line the CLI sends for `snapshot -i`, into the
//! typed request the daemon dispatches; and the reply to it on the Big tab,
//! built from the mirror's listing as the snapshot handler builds it and
//! written into the reply line by `reply_line`, on a mirror in this process
//! over a fake emulator (design D3 of `profiling`). The wire benchmarks
//! report their allocations per iteration beside the time; the socket
//! benchmarks' work is mostly in the daemon's process, which the counting
//! allocator does not see.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use daemon::driver::{AttachOptions, Driver, Launch};
use daemon::fake::FakeDriver;
use daemon::listing::{self, Answer};
use daemon::wire::{self, Command, Request};
use mirror::{DEFAULT_MIN_REREAD_INTERVAL, Mirror, Snapshot, SnapshotOptions};
use serde_json::{Value, json};

#[path = "../../protocol/benches/support/alloc.rs"]
mod alloc;
#[path = "../tests/support/mod.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

/// The line the CLI sends for `snapshot -i --delta` in the default session,
/// its fields in the order serde_json writes a map's.
const DELTA_SNAPSHOT: &str =
    r#"{"action":"snapshot","delta":true,"id":"r123456","interactive":true,"session":"default"}"#;

/// The `readMs` of the reply `reply_line` writes. The daemon's reply carries
/// the worker's time for the request there, the one field that changes from
/// run to run, and with its number of digits the bytes the line allocates,
/// which `micro` compares exactly (design D16).
const READ_MS: f64 = 1.0;

fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("daemon");

    // One daemon for the socket benchmark, on a fake Android emulator, whose
    // windows send change events, so a warm snapshot reads nothing from it.
    // Dropping `Daemon` kills it, so a panic leaves no daemon behind.
    let daemon = support::Daemon::builder().fake("android").start();
    support::open_big(&daemon);
    // The daemon's reply line to the CLI's request, `reply_line`'s input.
    let raw = support::raw_request(&daemon.socket(), support::INTERACTIVE_SNAPSHOT);
    let reply: Value = serde_json::from_str(&raw).expect("the reply is JSON");
    support::check_big(&reply);

    let request: Value =
        serde_json::from_str(support::INTERACTIVE_SNAPSHOT).expect("the request is JSON");
    assert_eq!(
        request.to_string(),
        support::INTERACTIVE_SNAPSHOT,
        "the socket benchmark sends the line the parse benchmark parses"
    );
    // Each request goes on a connection of its own, as the CLI's do, and its
    // timed span runs from writing the request out to the reply parsed into
    // a `Value`, the CLI's side of the socket. Each reply is checked, and
    // dropped, outside its span, so a failed request cannot pass for a fast
    // one.
    group.bench_function(
        BenchmarkId::new("socket", "snapshot-interactive-big"),
        |b| {
            b.iter_custom(|iters| {
                let mut timed = Duration::ZERO;
                for _ in 0..iters {
                    let request = request.clone();
                    let started = Instant::now();
                    let reply = daemon.request(request);
                    timed += started.elapsed();
                    support::check_big(&reply);
                }
                timed
            });
        },
    );

    // The same listing as a delta, `snapshot -i --delta`, with nothing
    // changed since the delta before it, which the request before the
    // benchmark takes as the session's first, the whole listing: each reply
    // says `unchanged`, as a model's turn that lists a screen it already
    // has gets it (spike S15: 1.08 ms of CPU on a Simulator, more than the
    // whole listing). The span is the listing's above.
    let delta: Value = serde_json::from_str(DELTA_SNAPSHOT).expect("the delta request is JSON");
    let first = daemon.request(delta.clone());
    assert_eq!(first["data"]["snapshot"]["kind"], "full", "{first}");
    check_unchanged(&daemon.request(delta.clone()));
    group.bench_function(
        BenchmarkId::new("socket", "snapshot-interactive-delta-unchanged-big"),
        |b| {
            b.iter_custom(|iters| {
                let mut timed = Duration::ZERO;
                for _ in 0..iters {
                    let request = delta.clone();
                    let started = Instant::now();
                    let reply = daemon.request(request);
                    timed += started.elapsed();
                    check_unchanged(&reply);
                }
                timed
            });
        },
    );
    // Stopped before the next benchmarks, so its process stays out of them.
    stop(daemon);

    // A daemon on a fake Simulator on the fixture's Anim tab, whose spinner
    // keeps the frames changing while the windows send no change events, so
    // that the mirror re-reads them when frames change, no sooner than its
    // minimum interval after the read before. Each request goes half that
    // interval after the reply before it, outside its span, about what a
    // request that comes at a random time between the re-reads a
    // Simulator's frames call for has left of the interval (spike S15: 106
    // ms of a 119 ms listing waited for the re-read). The fake makes its
    // frames as it is called, so the request finds frames newer than the
    // latest read, which the previous request made; the request and its
    // span are the Big tab's above. Each iteration lasts at least the pause,
    // so each sample holds one iteration or a few (flat sampling), the
    // benchmark taking 10 to 20 s.
    let pause = DEFAULT_MIN_REREAD_INTERVAL / 2;
    let animating = support::Daemon::builder().fake("ios").start();
    support::open_anim(&animating);
    group.sampling_mode(SamplingMode::Flat);
    group.bench_function(
        BenchmarkId::new("socket", "snapshot-interactive-animating"),
        |b| {
            b.iter_custom(|iters| {
                let mut timed = Duration::ZERO;
                for _ in 0..iters {
                    std::thread::sleep(pause);
                    let request = request.clone();
                    let started = Instant::now();
                    let reply = animating.request(request);
                    timed += started.elapsed();
                    support::check_anim(&reply);
                }
                timed
            });
        },
    );
    group.sampling_mode(SamplingMode::Auto);
    stop(animating);

    // Each `iter` below drops its routine's output inside the timed region,
    // as spike S8 timed decode: the request's maps and strings, the line.
    let line = support::INTERACTIVE_SNAPSHOT;
    let (parsed, counts) = alloc::counted(|| wire::parse(line));
    let mut expected = Request::new(Command::Snapshot(::wire::Snapshot {
        interactive: true,
        ..Default::default()
    }));
    expected.id = json!("r123456");
    assert_eq!(parsed, Ok(expected));
    alloc::report("daemon/wire/parse-snapshot-request", counts);
    group.bench_with_input(
        BenchmarkId::new("wire", "parse-snapshot-request"),
        line,
        |b, line| b.iter(|| wire::parse(black_box(line))),
    );

    // The listing's reply to that request built from the mirror's snapshot
    // as the snapshot handler builds it, the refs map, the text and the
    // rest of its fields, and written into the reply line, on a mirror in
    // this process over a fake emulator's Big tab, so that the allocations
    // the counting allocator sees are the reply's. The snapshot is the
    // warm listing the socket benchmark's daemon makes; its own making is
    // the mirror's benchmarks' (`mirror/snapshot/interactive-again-grid`).
    let (target, snapshot) = big_listing();
    let id = reply["id"].clone();
    let answer = Answer {
        read_ms: READ_MS,
        served_from_replica: true,
        target: &target,
    };
    let build = || wire::reply_line(&id, &Ok(listing::reply(&snapshot, &answer)));
    let (written, counts) = alloc::counted(build);
    let parsed: Value = serde_json::from_str(&written).expect("the line is JSON");
    support::check_big(&parsed);
    assert_eq!(parsed["id"], id, "{written}");
    assert_eq!(parsed["error"], Value::Null, "{written}");
    assert_eq!(parsed["data"]["readMs"], json!(READ_MS), "{written}");
    assert_eq!(parsed["data"]["target"], json!(target), "{written}");
    // The daemon's reply lists the same screen, the refs' numbers aside:
    // the daemon's listings before the Big tab issued refs of their own.
    assert_eq!(
        unnumbered(&parsed["data"]["snapshot"]),
        unnumbered(&reply["data"]["snapshot"]),
        "the mirror here lists what the daemon's lists"
    );
    alloc::report("daemon/wire/reply-snapshot-big", counts);
    group.bench_function(BenchmarkId::new("wire", "reply-snapshot-big"), |b| {
        b.iter(|| {
            wire::reply_line(
                black_box(&id),
                &Ok(listing::reply(black_box(&snapshot), black_box(&answer))),
            )
        });
    });

    group.finish();
}

/// The fake emulator's id and the warm interactive listing of its Big tab,
/// as the daemon's worker lists it for the socket benchmark: a mirror over
/// the fake driver's attachment, the fixture launched on its Big tab, listed
/// once to issue the refs and again.
fn big_listing() -> (String, Snapshot) {
    let driver = FakeDriver::parse("android").expect("a fake emulator");
    let info = driver.discover().expect("the fake's targets").remove(0);
    let mut attachment = driver
        .attach(&info, &AttachOptions::default())
        .expect("the fake attaches");
    let big = Launch {
        relaunch: false,
        env: vec![("initial_tab".into(), "big".into())],
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    attachment
        .apps
        .launch(support::APP, &big, deadline)
        .expect("the fixture launches");
    let mut mirror = Mirror::new(attachment.backend, attachment.config);
    let options = SnapshotOptions::interactive();
    loop {
        mirror
            .run_until(Instant::now() + Duration::from_millis(10))
            .expect("the mirror reads the fake");
        let first = mirror.snapshot(&options).expect("a listing");
        if first.refs().len() == support::BIG_ELEMENTS {
            break;
        }
        assert!(Instant::now() < deadline, "no Big tab:\n{}", first.text);
    }
    let snapshot = mirror.snapshot(&options).expect("the warm listing");
    (info.id, snapshot)
}

/// The listing `text` with its refs' numbers left out, `ref=e12` as `ref=e`.
fn unnumbered(text: &Value) -> String {
    let mut rest = text.as_str().unwrap_or_default();
    let mut out = String::with_capacity(rest.len());
    while let Some(at) = rest.find("ref=e") {
        let (kept, after) = rest.split_at(at + "ref=e".len());
        out.push_str(kept);
        rest = after.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

/// Checks that `reply` is a delta snapshot of the Big tab that found the
/// listing unchanged since the delta before it, served from the replica.
#[track_caller]
fn check_unchanged(reply: &Value) {
    assert_eq!(reply["success"], true, "{reply}");
    let data = &reply["data"];
    assert_eq!(data["snapshot"]["kind"], "unchanged", "{reply}");
    assert_eq!(
        data["servedFromReplica"], true,
        "an unchanged delta reads nothing from the target: {reply}"
    );
}

/// Stops `daemon` with `shutdown` and waits for it to exit.
fn stop(mut daemon: support::Daemon) {
    let stopped = daemon.request(json!({"action": "shutdown"}));
    assert_eq!(stopped["data"]["stopped"], true, "{stopped}");
    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("the daemon exits after shutdown");
    assert!(status.success(), "{status}");
}

criterion_group!(daemon, benches);
criterion_main!(daemon);
