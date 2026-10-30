//! The microbenchmark of the server's call path (design D3 of `profiling`,
//! spike S15's hot spot 4), run by `mobile-agent-bench micro`: a
//! `tools/call` of `snapshot`, interactive and the whole listing rather than
//! a delta, in the session holding a fake emulator at the fixture's Big tab,
//! its 500 cells and five tabs, through `mobile-agent-mcp` over its standard
//! input and output, from the request line written to the response line
//! read; and beside it the line the server sends the daemon for that call,
//! straight on the daemon's socket on a connection of its own, from the
//! connect to the reply line read, the floor the call stands on, so that
//! the server's share is the difference of the two medians, as spikes S13
//! and S15 paired them.
//!
//! The server runs as a client runs it, a process of its own over pipes:
//! the crate is a binary, a handler's request context comes only from
//! rmcp's serve loop, and the path timed holds the server's runtime, its
//! stdio transport and its hand-offs to the blocking pool beside the call
//! itself. The work is in the server's and the daemon's processes, so no
//! allocations are counted.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, ChildStdin, ChildStdout};
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use serde_json::{Value, json};

#[path = "../tests/support/mod.rs"]
mod support;

/// The cells of the fixture's Big tab (`fixtures/CONTRACT.md`), and what an
/// interactive listing of it holds, each with a ref: the cells and the five
/// tab buttons.
const BIG_CELLS: usize = 500;
const BIG_ELEMENTS: usize = BIG_CELLS + 5;

/// The session the daemon's requests without one go out in, which the
/// calls name, as spike S15's client named it.
const SESSION: &str = "default";

/// The line the server sends the daemon for the call, the request
/// `Tool::call` makes of its arguments, with an id as the CLI makes them.
const SOCKET_LINE: &str =
    r#"{"action":"snapshot","delta":false,"id":"r123456","interactive":true,"session":"default"}"#;

/// How long a reply may take before the benchmark gives up on it.
const ANSWER: Duration = Duration::from_secs(30);

/// The server over its pipes, opened in the 2025 revisions' era with
/// `initialize`, as spike S15's client opened it.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Server {
    fn start(env: &support::TestEnv) -> Server {
        let mut child = env.server_command().spawn().expect("the server starts");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("a stdout pipe"));
        let mut server = Server {
            child,
            stdin,
            stdout,
            next: 1,
        };
        let id = server.next_id();
        let opened = server.exchange(&request(
            id,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "mcp-bench", "version": "0.1.0"},
            }),
        ));
        let opened: Value = serde_json::from_str(&opened).expect("the answer is JSON");
        assert!(opened.get("result").is_some(), "initialize: {opened}");
        server.write(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string());
        server
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }

    /// Writes one message line.
    fn write(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("the server's input is open");
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .expect("the server reads");
    }

    /// Writes `line` and reads the next line the server writes, the answer
    /// to a request when no notification comes between, as none comes to a
    /// call without a progress token.
    fn exchange(&mut self, line: &str) -> String {
        self.write(line);
        let mut answer = String::new();
        let read = self
            .stdout
            .read_line(&mut answer)
            .expect("the server writes");
        assert!(read > 0, "the server closed its output");
        answer
    }

    /// Closes the server's input, so that it releases the session's target
    /// and exits.
    fn close(mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + ANSWER;
        while self
            .child
            .try_wait()
            .expect("the server can be polled")
            .is_none()
        {
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.wait();
    }
}

/// A JSON-RPC request line.
fn request(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// The call the benchmark times, as spike S15's client made it.
fn snapshot_call(id: u64) -> String {
    request(
        id,
        "tools/call",
        json!({
            "name": "snapshot",
            "arguments": {"interactive": true, "delta": false, "session": SESSION},
        }),
    )
}

/// Checks that `answer` answers the call `id` with the interactive listing
/// of the Big tab: every cell and tab on a line of its own with its ref.
#[track_caller]
fn check_call(answer: &str, id: u64) {
    let answer: Value = serde_json::from_str(answer).expect("the answer is JSON");
    assert_eq!(answer["id"], id, "the answer to the call: {answer}");
    let listing = support::ok(&answer["result"]);
    check_listing(&listing);
}

/// Checks that `listing` is the Big tab's interactive listing.
#[track_caller]
fn check_listing(listing: &str) {
    assert_eq!(
        listing.lines().count(),
        BIG_ELEMENTS,
        "the cells and the tabs:\n{listing}"
    );
    let cells = listing
        .lines()
        .filter(|line| line.contains("id=cell-") && line.contains("ref=e"))
        .count();
    assert_eq!(cells, BIG_CELLS, "a ref for every cell:\n{listing}");
}

/// Sends [`SOCKET_LINE`] on a connection of its own and reads the reply
/// line, as the server does for the call.
fn socket_request(socket: &std::path::Path) -> String {
    let mut stream = UnixStream::connect(socket).expect("the daemon accepts");
    stream
        .set_read_timeout(Some(ANSWER))
        .expect("a read timeout can be set");
    stream
        .write_all(SOCKET_LINE.as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .expect("the request is written");
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .expect("a reply line arrives");
    reply
}

/// Brings the daemon's only target to the fixture's Big tab in the default
/// session, as `crates/daemon/benches/daemon.rs` brings it there: opens the
/// fixture app, clicks the tab, waits for the screen to settle and takes
/// an interactive snapshot, which issues the screen's refs, so that every
/// listing after it is warm.
fn open_big(env: &support::TestEnv) {
    let opened = env.request(json!({"action": "open", "app": support::APP}));
    assert_eq!(opened["data"]["settled"], true, "{opened}");
    let clicked = env.request(json!({"action": "click", "target": "#tab-big"}));
    assert_eq!(clicked["success"], true, "{clicked}");
    let settled = env.request(json!({"action": "wait", "settle": true}));
    assert_eq!(settled["data"]["settled"], true, "{settled}");
    let first = env.request(json!({"action": "snapshot", "interactive": true}));
    assert_eq!(first["success"], true, "{first}");
}

fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("mcp");

    // A daemon on a fake emulator, whose windows send change events, so a
    // warm listing reads nothing from it. Dropping `TestEnv` stops it, so a
    // panic leaves no daemon behind.
    let env = support::TestEnv::start("android");
    open_big(&env);
    let mut server = Server::start(&env);
    let id = server.next_id();
    check_call(&server.exchange(&snapshot_call(id)), id);

    // Each call's line is written out before its span starts, and its
    // answer checked, and dropped, after the span, so a failed call cannot
    // pass for a fast one.
    group.bench_function(BenchmarkId::new("call", "snapshot-interactive-big"), |b| {
        b.iter_custom(|iters| {
            let mut timed = Duration::ZERO;
            for _ in 0..iters {
                let id = server.next_id();
                let line = snapshot_call(id);
                let started = Instant::now();
                let answer = server.exchange(&line);
                timed += started.elapsed();
                check_call(&answer, id);
            }
            timed
        });
    });

    let socket = env.socket();
    let reply: Value = serde_json::from_str(&socket_request(&socket)).expect("the reply is JSON");
    assert_eq!(reply["success"], true, "{reply}");
    check_listing(reply["data"]["snapshot"].as_str().unwrap_or_default());
    group.bench_function(
        BenchmarkId::new("socket", "snapshot-interactive-big"),
        |b| {
            b.iter_custom(|iters| {
                let mut timed = Duration::ZERO;
                for _ in 0..iters {
                    let started = Instant::now();
                    let reply = socket_request(&socket);
                    timed += started.elapsed();
                    assert!(
                        reply.contains(r#""success":true"#),
                        "the daemon's reply: {reply}"
                    );
                }
                timed
            });
        },
    );

    group.finish();
    server.close();
}

criterion_group!(mcp, benches);
criterion_main!(mcp);
