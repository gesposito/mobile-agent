//! Microbenchmarks of the daemon's JSON wire and of a snapshot through its
//! socket (design D14), run by `mobile-agent-bench micro`: a warm
//! interactive snapshot of the fake fixture's Big tab, its 500 cells and five
//! tabs, through the socket of a daemon on a fake target, one request per
//! iteration, the host's whole path less the CLI process; `Request::parse`
//! of the line the CLI sends for it; and `reply_line` of the reply the daemon
//! sends back. The wire benchmarks report their allocations per iteration
//! beside the time; the socket benchmark's work is mostly in the daemon's
//! process, which the counting allocator does not see.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use daemon::wire::{self, Failure, Request, Success};
use serde_json::{Map, Value, json};

#[path = "../../protocol/benches/support/alloc.rs"]
mod alloc;
#[path = "../tests/support/mod.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

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
    let mut daemon = support::Daemon::builder().fake("android").start();
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
    // Stopped before the wire benchmarks, so its process stays out of them.
    let stopped = daemon.request(json!({"action": "shutdown"}));
    assert_eq!(stopped["data"]["stopped"], true, "{stopped}");
    let status = daemon
        .wait_exit(Duration::from_secs(5))
        .expect("the daemon exits after shutdown");
    assert!(status.success(), "{status}");
    drop(daemon);

    // Each `iter` below drops its routine's output inside the timed region,
    // as spike S8 timed decode: the request's maps and strings, the line.
    let line = support::INTERACTIVE_SNAPSHOT;
    let (parsed, counts) = alloc::counted(|| Request::parse(line));
    let expected = Request {
        id: json!("r123456"),
        action: "snapshot".to_owned(),
        session: "default".to_owned(),
        params: Map::from_iter([("interactive".to_owned(), json!(true))]),
    };
    assert_eq!(parsed, Ok(expected));
    alloc::report("daemon/wire/parse-snapshot-request", counts);
    group.bench_with_input(
        BenchmarkId::new("wire", "parse-snapshot-request"),
        line,
        |b, line| b.iter(|| Request::parse(black_box(line))),
    );

    // `reply_line` of the daemon's data is the daemon's line, byte for byte,
    // so the benchmark writes what the daemon writes.
    let id = reply["id"].clone();
    let mut data = reply["data"].clone();
    let rewritten = wire::reply_line(&id, &Ok(Success::new(data.clone())));
    assert!(
        rewritten == raw,
        "reply_line does not give the daemon's line"
    );
    let read_ms = data["readMs"].to_string();
    assert!(data["readMs"].is_f64(), "readMs is {read_ms}");
    data["readMs"] = json!(READ_MS);
    let result: Result<Success, Failure> = Ok(Success::new(data.clone()));
    let (written, counts) = alloc::counted(|| wire::reply_line(&id, &result));
    assert_eq!(
        written.len(),
        raw.len() - read_ms.len() + json!(READ_MS).to_string().len(),
        "the daemon's line with readMs {READ_MS}"
    );
    let parsed: Value = serde_json::from_str(&written).expect("the line is JSON");
    assert!(
        parsed == json!({"id": id, "success": true, "data": data, "error": null}),
        "the line parses back to another reply"
    );
    alloc::report("daemon/wire/reply-snapshot-big", counts);
    group.bench_with_input(
        BenchmarkId::new("wire", "reply-snapshot-big"),
        &(id, result),
        |b, (id, result)| b.iter(|| wire::reply_line(black_box(id), black_box(result))),
    );

    group.finish();
}

criterion_group!(daemon, benches);
criterion_main!(daemon);
