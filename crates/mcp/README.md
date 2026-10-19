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
  main.rs       the mobile-agent-mcp binary: the command line, the daemon's endpoint, the runtime,
                serving stdio, the ways out and the release on each
  server.rs     rmcp's handler: the server's info and card, tools/list and tools/call, each call
                one request on the daemon's socket, selected against the progress tick, the
                request's cancellation token and the bound
  tools.rs      the tool table: names, actions, descriptions, annotations, the profile, the changes
                to the request structs' schemas, the reply structs' schemas, and a call's arguments
                back into a request line within the bound
  results.rs    a reply as a tool's result: the CLI's text, a failure's code, a screenshot's image,
                the bound's error and the structured content
  sessions.rs   the minted session, the targets each session's replies named, their release, a
                session's cancel, and each session's last screenshot
  instructions.rs  the instructions card and the help tool's text
tests/
  support/      a daemon on fake targets per test on a private socket directory, the server over
                its stdio with its flags, rmcp's client (initialize) and a JSON-RPC exchange in the
                2026-07-28 era, requests sent without waiting, notifications and their arrival
  tools.rs      the tools in both eras with their schemas, and the answers to snapshot, click,
                fill, get, devices and status, failures among them
  sessions.rs   the minted session, a call's session, and the release on each way out
  screenshots.rs  image content, ifChanged and path
  waits.rs      the flags, the bound and its cap, progress, a client's cancellation, the backstop
  profiles.rs   the read-only profile, the card's sizes, help
  structured.rs  outputSchema and structuredContent with the flag, neither without
```

## Running

The client starts the binary and speaks MCP on its standard input and
output. The command line:

| Flag | Default | What it sets |
|---|---|---|
| `--call-timeout <SECONDS>` | 55, at least 3 | the bound of a call (Waits below), under Codex's 60 s |
| `--tools <PROFILE>` | `all` | the tools served, `read-only` or `all` (Profiles below) |
| `--structured` | off | an `outputSchema` on each tool and a `structuredContent` on each result (Structured results below) |

The server writes its own lines to standard error alone: one at start
naming its session, the daemon's socket, the bound and the profile; one
when it sends the daemon a cancel, past the bound or on the client's
cancellation; and any failure to release a target at exit, or to cancel a
call. It serves both protocol eras with
no code per era: rmcp's serve loop takes the lifecycle from the client's
first request other than `server/discover`, `initialize` for clients of
the 2025 revisions, any request with the version, the client's info and its
capabilities in its `_meta` for the 2026-07-28 revision. The answer to
either opening names the server, its version and the instructions card.

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
| `help` | none: the guide, answered by the server | none | yes |
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

## Waits

Every call is bounded by `--call-timeout`, 55 s by default (design D7),
with one mechanism for every tool, in `src/server.rs`'s loop, which
selects the blocking call against a tick, the request's cancellation token
and the bound:

- A request that takes `timeout` (`click`, `fill`, `type`, `get`,
  `scroll`, `open` and `wait`) and whose call names none gets the bound
  less a margin of 2 s as its `timeout`, so the daemon's own wait ends
  first, with its `wait_timeout` and the screen's summary; a `wait` with
  `settle` keeps its own maximum of 3 s. A `timeout` or `ms` above the
  bound less the margin is capped to it, and the result's warning line
  says so: `⚠ timeout 60000 ms was capped to 53000 ms, the server's bound
  of 55 s less its margin of 2 s`. An `open` whose app keeps animating
  settles until the bound rather than 3 s, since its `timeout` bounds its
  settle; pass `timeout` to shorten it.
- While a call runs, every 5 s, a request that carries a progress token in
  its `_meta` gets `notifications/progress` through the request's peer:
  `progress` is the seconds elapsed, with no total, and the message names
  the tool, the seconds and what the arguments say the call waits for,
  `wait: 5 s elapsed, waiting for the text "Welcome"`: `until`'s element,
  the element, the text, the app, a change since the last snapshot, the
  screen settling, the milliseconds, else the daemon's reply. The
  notifications keep Claude Code's idle timer alive and show in its task
  entry.
- A client's `notifications/cancelled` fires the request's cancellation
  token: the handler says so on standard error, sends the daemon the
  session's `cancel` on a connection of its own (`src/sessions.rs`, as
  `close` goes at exit) and lets the call go, so the command in flight
  fails with `cancelled` and the target is free for the next call; rmcp
  drops the abandoned call's response, and the daemon's reply to it is
  dropped with it.
- As a backstop, a call the daemon has not answered 5 s past the bound
  gets the same `cancel`, said on standard error; the daemon's reply
  within 5 s more is the call's answer (a tap held in its device call,
  which no cancel interrupts, completes), else the call is an execution
  error: `✗ daemon_timeout: click reached the server's bound of 55 s and
  was cancelled`.

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

## Profiles

`--tools read-only` serves the tools whose annotation is read-only alone,
`devices`, `status`, `doctor`, `snapshot`, `diff_snapshot`, `get`, `wait`,
`screenshot` and `help` (design D10), which also passes Codex's write
approvals without prompts: `tools/list` shows them alone, and a call to
another tool is refused with invalid params naming the profile and what it
serves. `--tools all`, the default, serves every tool.

## The instructions card and `help`

The answer to `initialize` and to `server/discover` carries the
instructions card (`src/instructions.rs`), built at start for the profile
and the bound: a first paragraph of 369 characters that stands alone
within the 512 Codex keeps, what the server drives, that refs come from
`snapshot`, that replies verify and a failure names its code and what to
do next, and that `help` holds the guide; then selectors and refs, with
where a plain text such as a status line is read, since it has no ref and
is in neither the interactive listing nor a change wait's delta taken in
it (`get`, `wait` with `text`, or `interactive: false`), the waits
(`change`, `settle`, `text`), sessions and `close` with `keep`,
screenshots with `ifChanged` and that one right after a click can show
the transition the tap set off, so `settle` comes first, and the tools
served, which says so when the profile is read-only: 2,594 characters in
all, 2,635 when read-only, under the 4,096 Claude Code keeps. The two
sentences on the plain text and the transition are what the acceptance's
models worked around (`reports/12.1-mcp-acceptance.md`, findings 1 and
4).

`help`, a read-only tool with no arguments of its own, answers with the
CLI's agent guide (`mobile-agent help agent`, the CLI library's
`AGENT_GUIDE`) after a table of 32 lines that maps each command the guide
uses to its tool and arguments (`snapshot -i` is `snapshot`, whose
`interactive` and `delta` are the defaults; `click @e3` is `click` with
`selector`; `wait --change` is `wait` with `change`; `close --keep` is
`close` with `keep`) and names the three the server has no tool for, `set
timeout`, `cancel` and `batch`, with what stands in for them, so a model
reads one guide for both surfaces.

## Structured results

With `--structured`, each tool whose answer is a daemon reply carries an
`outputSchema`, the schema of its reply-data struct in `crates/wire`
(`wire::reply::Snapshot` for `snapshot`, and so on, the root's title and
description left out as rmcp strips them), and each successful result a
`structuredContent`, the reply's data less what the content carries
already: the refs map, at the top and in a delta, which the listing
repeats, and a screenshot's image, which the image block holds, each
optional in the schema. `doctor` and `help` have no reply struct, so no
schema, and carry the report's JSON and `{"help": text}`. A failure
carries no structured content, so its text, with the code, reaches a
client that shows structured content alone, and a warning stays on the
text. Without the flag, neither is sent (design D5): Codex shows a model
only the structured content when a result has it, dropping images and
text.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Tools over the daemon | Snapshot through the server; Failure carries the code | `tools::the_tools_list_in_both_eras_as_flat_objects_with_selector_session_and_device`, `tools::every_parameter_of_each_action_reaches_its_tool`, `tools::a_snapshot_answers_with_the_listing_and_no_refs_map_as_a_delta_by_default`, `tools::a_click_answers_with_the_cli_line_and_keeps_its_warning`, `tools::fill_and_get_answer_with_the_lines_the_cli_prints`, `tools::devices_and_status_answer_with_the_lines_the_cli_prints`, `tools::a_failure_is_an_execution_error_naming_its_code_and_what_to_do`; the structured results (design D5): `structured::with_structured_each_typed_tool_carries_its_reply_s_schema_and_each_result_the_reply_s_data`, `structured::with_structured_a_screenshot_s_image_a_failure_and_the_untyped_tools_keep_their_content`, `structured::without_structured_no_tool_carries_a_schema_and_no_result_structured_content`; in `src/tools.rs`, `the_table_is_sorted_by_name_with_one_tool_per_action`, `a_call_goes_out_as_the_daemon_s_line_through_the_table` and `a_definition_carries_its_reply_s_schema_when_structured`; in `src/results.rs`, `a_success_is_the_cli_s_lines_and_its_warning`, `a_failure_names_its_code_after_the_cli_s_mark` and `structured_content_leaves_the_refs_and_the_image_to_the_content` |
| Sessions and claims | Server exits | `sessions::a_server_mints_its_session_and_a_call_may_name_another`, `sessions::a_server_whose_input_ends_releases_its_claim_and_leaves_the_app_running`, `sessions::a_server_signalled_to_stop_releases_its_claim_and_leaves_the_app_running`, `sessions::a_server_releases_a_claim_no_reply_named`, `sessions::a_server_that_was_only_discovered_exits_when_its_input_ends`; in `src/sessions.rs`, `a_reply_that_names_its_target_holds_it_and_close_releases_it` |
| Screenshots inline | Unchanged screen | `screenshots::a_screenshot_is_the_daemon_s_image_within_the_long_edge_bound`, `screenshots::if_changed_answers_a_still_screen_with_a_line_and_a_changed_one_with_the_image`, `screenshots::a_screenshot_with_a_path_answers_with_the_path_alone`; in `src/sessions.rs`, `a_screenshot_is_the_last_one_s_when_its_image_is_on_the_same_session_and_device` |
| Long waits bounded and cancellable | Wait past the bound | `waits::the_server_takes_its_bound_its_profile_and_structured_with_their_defaults`, `waits::a_wait_past_the_bound_sends_progress_every_5_s_and_ends_with_the_daemon_s_timeout`, `waits::a_timeout_above_the_bound_is_capped_and_said_and_no_progress_goes_without_a_token`, `waits::a_cancelled_call_ends_the_daemon_s_wait_so_the_next_call_runs_at_once`, `waits::a_call_the_daemon_has_not_answered_past_the_bound_is_cancelled_by_the_server`; in `src/tools.rs`, `a_call_waits_within_the_bound_less_the_margin` and `a_call_says_what_it_waits_for`; in `src/main.rs`, `the_command_line_defaults_to_a_55_s_bound_all_tools_and_plain_results` |
| Profiles and instructions | Read-only profile | `profiles::the_read_only_profile_lists_the_read_only_tools_alone_and_refuses_the_rest`, `profiles::the_instructions_card_s_first_paragraph_stands_alone_and_the_whole_fits_the_clients`, `profiles::help_answers_with_the_agent_guide_after_a_table_mapping_its_commands_to_the_tools`; in `src/instructions.rs`, `the_card_s_first_paragraph_stands_alone_and_the_whole_fits_the_clients` and `help_is_the_table_then_the_guide` |
