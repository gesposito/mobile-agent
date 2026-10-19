# mcp

`mobile-agent-mcp`, the Model Context Protocol server of spec `mcp-server`
(design D1, D2): a stdio server an agent client starts, which gives a model
what the `mobile-agent` CLI gives a shell, the same refs, replies, codes and
sessions, over the daemon in `crates/daemon`. It is a crate of its own, so
that the SDK, `rmcp`, and its runtime are its dependencies alone and the
CLI keeps its size and start time; it builds on the CLI's library for what
the two surfaces share: `client.rs` (discovery, the daemon's start, the
version check, retries, the reply wait), `output.rs` (a reply's text),
`request.rs` (the lines the release sends), `doctor.rs` (the host's checks)
and `config.rs` (the settings file). Its tools' input schemas are the
request structs of `crates/wire`, with the `schema` feature on.

```sh
cargo build --release -p mcp -p daemon    # mobile-agent-mcp and mobile-agent-daemon, side by side
```

## Layout

```
src/
  main.rs       the mobile-agent-mcp binary: the daemon's endpoint, the runtime, serving stdio, the
                ways out and the release on each
  server.rs     rmcp's handler: the server's info and instructions, tools/list and tools/call, each
                call one request on the daemon's socket
  tools.rs      the tool table: names, actions, descriptions, annotations, the changes to the
                request structs' schemas, and a call's arguments back into a request line
  results.rs    a reply as a tool's result: the CLI's text, a failure's code, a screenshot's image
  sessions.rs   the minted session, the targets each session's replies named, their release, and
                each session's last screenshot
tests/
  support/      a daemon on fake targets per test on a private socket directory, the server over
                its stdio, rmcp's client (initialize) and a JSON-RPC exchange in the 2026-07-28 era
  tools.rs      the tools in both eras with their schemas, and the answers to snapshot, click,
                fill, get, devices and status, failures among them
  sessions.rs   the minted session, a call's session, and the release on each way out
  screenshots.rs  image content, ifChanged and path
```

## Running

The client starts the binary, which takes no arguments, and speaks MCP on
its standard input and output. The server writes its own lines to standard
error alone: one naming its session and the daemon's socket at start, and
any failure to release a target at exit. It serves both protocol eras with
no code per era: rmcp's serve loop takes the lifecycle from the client's
first request other than `server/discover`, `initialize` for clients of
the 2025 revisions, any request with the version, the client's info and its
capabilities in its `_meta` for the 2026-07-28 revision. The answer to
either opening names the server, its version and a paragraph of
instructions.

| Variable | Default |
|---|---|
| `MOBILE_AGENT_SOCKET_DIR` | the CLI's: `$XDG_RUNTIME_DIR/mobile-agent`, else `~/.mobile-agent`, else `mobile-agent` in the temporary directory |
| `MOBILE_AGENT_DAEMON_BIN` | `mobile-agent-daemon` beside this executable, else on `PATH` |
| `MOBILE_AGENT_IDLE_TIMEOUT_MS` | for a daemon the server starts, the settings file's `idleTimeout`, else the daemon's 1 h |
| `MOBILE_AGENT_EMULATOR_CONTROLLER` | for a daemon the server starts, the settings file's `emulatorController`, else `auto` |
| `MOBILE_AGENT_CONFIG` | the settings file, as the CLI reads it: this file alone when given, else `~/.mobile-agent/config.json` overlaid by `./mobile-agent.json`; a file that cannot be read is said on standard error and the server goes on without it |

The first call starts the daemon when none of this build serves, as the
CLI's first command does, with the server's environment, so
`MOBILE_AGENT_DRIVERS` and the fake targets' variables reach it. A relative
`path` a call names is taken from the server's working directory, as the
CLI takes one from its own.

## Tools

One tool per daemon action an agent drives, sorted by name, under
agent-browser's stems where the concepts match and the daemon's own
elsewhere, without a prefix, since Claude Code prefixes the server's name
(design D4). agent-browser's names below carry its `agent_browser_` prefix
(`docs/research/agent-surfaces.md`, Mirroring agent-browser). `set_timeout`,
`shutdown` and `cancel` are no tools: a waiting tool takes its `timeout`,
the daemon serves the shell's sessions too and exits when idle, and a
cancel belongs to the server.

| Tool | Daemon action | agent-browser's tool | Read-only |
|---|---|---|---|
| `click` | `click` | `click`; `tap` in its mobile profile | no |
| `close` | `close` | `close` | no, destructive |
| `devices` | `devices` | `device`, its list of Simulators, in its mobile profile | yes |
| `diff_snapshot` | `diff_snapshot` | `diff_snapshot` | yes |
| `doctor` | the CLI's checks of this host, `doctor` among them | `doctor` | yes |
| `fill` | `fill` | `fill` | no |
| `get` | `get` | `get_text`, `get_value`, `get_box` | yes |
| `open` | `open` | `open` | no |
| `press` | `press` | `press` | no |
| `screenshot` | `screenshot` with `inline` | `screenshot` | yes |
| `scroll` | `scroll` | `scroll` | no |
| `snapshot` | `snapshot` | `snapshot` | yes |
| `status` | `status` | none | yes |
| `swipe` | `swipe` | `swipe`, in its mobile profile | no |
| `type` | `type` | `type` | no |
| `wait` | `wait` | `wait_ms`, `wait_for_selector`, `wait_for_text`; `settle` and `change` have none | yes |

The annotations: `readOnlyHint` on the read-only tools, `destructiveHint`
on `close`, and `destructiveHint: false` on the others, since a client
takes a tool that is not read-only as destructive unless it says otherwise.

A tool's input schema is its action's request struct's, generated at run
time as rmcp generates a tool's (JSON Schema 2020-12, a flat object, every
parameter typed, described by the field's doc comment and showing its
default), with the table's changes in `src/tools.rs`:

- the element argument, `target` to the daemon, is `selector`, as
  agent-browser names it (a ref `@e3`, an id `#login`, or `text=`), in the
  properties, in `required` and in the descriptions that name it;
- `snapshot`'s `interactive` and `delta` default to true;
- every tool takes the envelope's `session` and `device`, and `open` its
  `force`;
- `screenshot` does not take `inline`, which the server sends set, takes
  `ifChanged`, which the server reads and does not send, and describes
  `path` for the tool.

Every other parameter is the struct's under the daemon's name
(`interactive`, `maxDepth`, `delta`, `text`, `key`, `direction`,
`amount`, `until`, `timeout`, `frames` and the rest), so a parameter a
struct gains reaches its tool, which `tests/tools.rs` holds. A call's
arguments go back through the same changes into the request line, each
value as the call gave it and a null as one not given, and only the
parameters the tool lists go out, so the daemon refuses a malformed value
with its own code and message (`"amount" must be a number, not "a lot"`).

## Results

A result's text is the reply as the CLI prints it without `--json`
(design D5), written by the CLI's `output.rs`, one formatter for both
surfaces:

- a snapshot is its listing, without the refs map that repeats it; a delta
  is the whole listing first, then `unchanged (revision N)` or the changes
  as `snapshot --delta` prints them;
- an action is the CLI's line: `✓ Done (dispatch 0.3 ms, no change in
  202 ms)` for a click, `✓ Opened <app> on <target>` for `open`,
  `✓ Released <target>; <app> keeps running` for `close` with `keep`,
  `✓ Settled after 111 ms: the tree changed last` or `✓ The tree changed
  after 300 ms` and the delta for a wait;
- `get`, `devices`, `status` and `diff_snapshot` are the CLI's lines;
- `doctor` is the CLI's report, one line per check and the Xcode
  compatibility table;
- a warning follows on its own line, marked `⚠` as the CLI marks it.

A failure is an execution error (`isError`), the CLI's `✗` line with the
daemon's code after the mark, whose message says what to do next:
`✗ stale_ref: e2 is stale: ... Take a new snapshot`. A doctor check that
fails makes the report an execution error ending with `✗ doctor_failed:
checks failed: ...`. A call whose blocking work fails in the server is a
protocol error.

Each call is one request on the daemon's socket through the CLI's client,
a connection of its own with the CLI's retries, awaited on the runtime's
blocking pool: the server runs tokio's current-thread runtime, and the
calls of a client wait side by side.

## Sessions and exit

The server mints its session, `mcp-<pid>`, and every call goes out in it
unless the call's `session` names another, for a client that resumes a
transcript with another session's refs (design D9). The server keeps, for
every session a call went out in, the targets its replies named: `open`'s,
a snapshot's and a diff's hold their target, and `close`'s releases it.

It ends when its input ends, at SIGTERM or SIGINT, whose handlers are in
place before the first request, and when the client leaves before a
request that chooses a lifecycle, which rmcp ends with an initialization
error (a client that only discovered the server), and on each way out it
releases the targets its sessions hold: `close` with `keep` and the
target's id, which releases the claim and leaves the app running, for each
target the replies named, then for each target `status` lists for those
sessions, since a command whose reply names no target, such as a click on a
selector, claims one too. The next session attaches to the target without
`force`, and `status` still names the app. The release waits at most 5 s
for the daemon; a close still queued behind a command in flight then goes
ahead in the daemon all the same. A daemon that no longer serves holds no
claims, and the server starts none to release them. The server exits 0,
and 1 when it cannot start its runtime or its signal handlers, or when the
client's opening failed otherwise than by the client leaving.

## Screenshots

The screenshot tool asks the daemon for the image in its reply (`inline`),
at the request struct's defaults, quality 80 and a long edge of 1000 px,
and answers with image content: the daemon's base64 and its `mimeType`
(design D6). With `path`, the daemon writes the file too and the answer is
the path alone, for a client that takes no images. The server hashes each
screenshot's image, as its base64 text, with the standard library's default
hasher, by its session and the `device` the call named, since a
screenshot's reply names no target; with `ifChanged`, an image whose hash
is that session's last screenshot's there gives the line `unchanged since
the last screenshot` instead.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Tools over the daemon | Snapshot through the server; Failure carries the code | `tools::the_tools_list_in_both_eras_as_flat_objects_with_selector_session_and_device`, `tools::every_parameter_of_each_action_reaches_its_tool`, `tools::a_snapshot_answers_with_the_listing_and_no_refs_map_as_a_delta_by_default`, `tools::a_click_answers_with_the_cli_line_and_keeps_its_warning`, `tools::fill_and_get_answer_with_the_lines_the_cli_prints`, `tools::devices_and_status_answer_with_the_lines_the_cli_prints`, `tools::a_failure_is_an_execution_error_naming_its_code_and_what_to_do`; in `src/tools.rs`, `the_table_is_sorted_by_name_with_one_tool_per_action` and `a_call_goes_out_as_the_daemon_s_line_through_the_table`; in `src/results.rs`, `a_success_is_the_cli_s_lines_and_its_warning` and `a_failure_names_its_code_after_the_cli_s_mark` |
| Sessions and claims | Server exits | `sessions::a_server_mints_its_session_and_a_call_may_name_another`, `sessions::a_server_whose_input_ends_releases_its_claim_and_leaves_the_app_running`, `sessions::a_server_signalled_to_stop_releases_its_claim_and_leaves_the_app_running`, `sessions::a_server_releases_a_claim_no_reply_named`, `sessions::a_server_that_was_only_discovered_exits_when_its_input_ends`; in `src/sessions.rs`, `a_reply_that_names_its_target_holds_it_and_close_releases_it` |
| Screenshots inline | Unchanged screen | `screenshots::a_screenshot_is_the_daemon_s_image_within_the_long_edge_bound`, `screenshots::if_changed_answers_a_still_screen_with_a_line_and_a_changed_one_with_the_image`, `screenshots::a_screenshot_with_a_path_answers_with_the_path_alone`; in `src/sessions.rs`, `a_screenshot_is_the_last_one_s_when_its_image_is_on_the_same_session_and_device` |
