# protocol

The wire protocol between the host daemon and every device agent (design D4,
spec `device-protocol`): the message schema, framing, the handshake and
capability negotiation, per-stream event sequences and liveness. It holds no
device code. The Android agent's Kotlin codec in
`agent-android/src/dev/mobileagent/agent/codec` writes the same bytes; golden
frames checked in both directions keep the two in step.

## Layout

```
src/
  message.rs     envelope: hello, welcome, request, response, ping, pong; methods, parameters, results
  snapshot.rs    snapshot payload: string table, windows and their metadata, nodes, bounds, flags, scroll semantics
  event.rs       events and their streams
  error.rs       structured errors and the code table
  capability.rs  the capability set
  frame.rs       length-prefixed framing, oversized-frame handling
  session/       HostSession (daemon end) and AgentSession (agent end), without I/O
  sequence.rs    per-stream sequence tracking
  liveness.rs    ping and silence timing
  cbor.rs        shared decoding helpers
golden/          frames this crate writes for the golden samples
golden/kotlin/   frames the Kotlin codec writes for the same samples
tests/           one file per task: schema, framing, handshake, events, liveness, golden
```

## Frames

A frame is a 32-bit big-endian payload length followed by exactly one CBOR
item (RFC 8949) of that length, with no bytes after the item. The limit
applies to the payload length. Before the handshake each side uses its own
maximum; afterwards both use the negotiated one, the smaller of the two.

- The sender refuses a message over the limit and writes none of it
  (`write_frame`). An agent answers such a response with a `frame_too_large`
  error naming the limit and the request instead.
- The receiver reports a frame whose header exceeds the limit (`Oversized`),
  with the kind, request id and method read from its first 32 bytes, and
  drops its payload as it arrives: nothing of it is delivered and the next
  frame decodes normally. The host turns it into a `frame_too_large` error for
  the pending request; the agent answers an oversized request the same way.
  Bytes that only feed such a discard do not count as signs of life, so a
  corrupt length cannot keep a dead peer alive.

## CBOR layout rules

Both codecs follow these rules, so the same message is the same bytes on both
sides.

1. **Preferred serialization.** Every integer and length head takes its
   shortest form.
2. **Definite lengths only.** Arrays, maps, text and byte strings always carry
   their length. Neither encoder writes indefinite-length items, and every
   golden frame is checked for them. The Kotlin decoder rejects them; the Rust
   decoder, like minicbor's derived decoders, accepts them.
3. **Maps keyed by field index.** Messages (except events), parameters,
   results, errors, snapshots, windows, nodes and scroll semantics are maps
   whose keys are unsigned integers, written in ascending order, each once.
4. **Absent optional fields are omitted**, never written as null. Decoders
   read a null as absent. A node's child list is omitted when empty and a
   window's modal flag when false; every other list is always present,
   possibly empty.
5. **Positional arrays have no optional elements.** Events, bounds, scroll
   ranges and scroll positions are arrays with a fixed layout, so their
   length never shrinks. Receivers skip elements past those they know, so new
   elements may be appended.
6. **Unknown keys are skipped**, and a repeated key's last value wins.
7. **Types.** Unsigned fields are major type 0, signed ones (coordinates,
   bounds, window layer, scroll range and position) major type 0 or 1,
   booleans are simple values, text is UTF-8 (decoders reject
   invalid UTF-8), image data is a byte string, flags are `u32` bitmasks and
   enums small integers. No floats and no tags.
8. **Parameters and results are always maps.** A request carries its
   parameters even when empty (`{}`); a successful response carries its
   result, `{}` for methods with none.
9. **Nesting.** Nodes nest at most `MAX_NODE_DEPTH` (256) levels below a
   window root; decoders reject deeper trees. The Rust decoder keeps its stack
   of open nodes on the heap: the derived recursive decoder took about 20 KiB
   of stack per level in a debug build and overflowed a 2 MiB thread below
   128 levels.

Receivers never allocate more than the input can hold: array lengths are
capped by the bytes left before anything is reserved.

## Messages

The kind is key 0 of a map or element 0 of an event array. `?` marks an
optional key.

| Kind | Value | Layout |
|---|---:|---|
| hello | 0 | `{0: 0, 1: versions [u16], 2: capabilities u32, 3: max_frame u32, 4: agent text, ?5: token text}` |
| welcome | 1 | `{0: 1, 1: versions [u16], ?2: version u16, 3: max_frame u32, ?4: error}` |
| request | 2 | `{0: 2, 1: id u32, 2: method u16, ?3: deadline_ms u32, 4: params}` |
| response | 3 | `{0: 3, 1: id u32, 2: method u16, 3: result}` or `{0: 3, 1: id u32, 2: method u16, 4: error}` |
| event | 4 | `[4, stream u8, seq u64, time_us u64, ...]` |
| ping | 5 | `{0: 5, 1: nonce u64}` |
| pong | 6 | `{0: 6, 1: nonce u64}` |

An error is `{0: code text, 1: message text, 2: retryable bool}`. The response
echoes the method so its result decodes without the request at hand. A
decoder reports an unknown kind as such, and the sessions skip it.

### Methods

| Method | Value | Needs | Parameters | Result |
|---|---:|---|---|---|
| snapshot | 1 | tree | `{?0: max_depth u32, ?1: max_nodes u32, ?2: idle_ms u32}` | snapshot |
| screenshot | 2 | screenshot | `{?0: format u8, ?1: quality u8, ?2: max_dimension u32}` | `{0: format u8, 1: width u32, 2: height u32, 3: rotation u8, 4: data bytes}` |
| tap | 3 | input | `{0: x i32, 1: y i32, ?2: duration_ms u32}` (a long press with a duration) | `{0: accepted bool}` |
| swipe | 4 | input | `{0: x1 i32, 1: y1 i32, 2: x2 i32, 3: y2 i32, 4: duration_ms u32}` | `{0: accepted bool}` |
| key | 5 | input | `{0: code u32, ?1: meta u32}` | `{0: accepted bool}` |
| text_set | 6 | text_set | `{0: text, ?1: node u32, ?2: mode u8}` | `{0: method u8}` |
| frames_start | 7 | frames | `{?0: max_fps u32, ?1: max_dimension u32}` | `{}` |
| frames_stop | 8 | frames | `{}` | `{}` |
| resync | 9 | events | `{0: stream u8}` | `{0: stream u8, 1: seq u64}` |
| shutdown | 10 | nothing | `{}` | `{}` |

An agent answers a method it does not know with `unknown_method`; the request
still decodes, its parameters skipped, so it can be answered.

### Snapshot

| Item | Layout |
|---|---|
| snapshot | `{0: strings [text], 1: windows [window], 2: node_count u32, 3: truncated u32, 4: seq u64}` |
| window | `{0: id u32, 1: kind u8, 2: layer i32, ?3: package u32, ?4: title text, 5: bounds, 6: flags u32, ?7: root node, ?8: emits_events bool, ?9: owner u32, ?10: modal bool}` |
| node | `{0: id u32, 1: class u32, ?2: res_pkg u32, ?3: res_id text, ?4: text, ?5: desc text, 6: bounds, 7: flags u32, ?8: children [node], ?9: scroll}` |
| bounds | `[left i32, top i32, right i32, bottom i32]` |
| scroll | `{0: axis u8, 1: flags u32, ?2: range, ?3: position}` |
| range | `[unit u8, min i32, max i32]` |
| position | `[first i32, last i32]` |

`class`, `res_pkg`, `package` and `owner` index the per-snapshot string table.
`truncated` says which request bound cut the walk short (bit 0 depth, 1 nodes,
2 windows). `seq` is the last tree-stream event the agent had sent when it read
the tree. Window kinds are `AccessibilityWindowInfo.TYPE_*` values. Node flag
bits: clickable, long_clickable, focusable, focused, enabled, checkable,
checked, scrollable, selected, editable, password, visible,
accessibility_focused (bits 0 to 12). Window flag bits: active, focused,
accessibility_focused, picture_in_picture.

Keys 8 to 10 of a window are what the agent knows about the window beyond
the platform's own window states (task 3.6):

- `emits_events` says whether the window's toolkit sends change events for
  its content. `false` marks a window whose changes are seen only by reading
  it again: Jetpack Compose sends none to a UI automation client (spike S1),
  so a consumer re-reads such a window after each action and on a poll.
  Absent when the agent cannot tell.
- `owner` names the system process that owns the window, such as
  `com.android.systemui` for the status bar or the permission controller for
  a permission prompt. Absent when the application under automation owns it.
- `modal` marks a window, such as a dialog, that keeps the user from the
  windows below it until it is dismissed. Omitted when false.

Key 9 of a node is present on nodes that scroll. `axis` is 0 unknown, 1
vertical, 2 horizontal. Flag bits: back (0), toward the start, up or left, and
forward (1), toward the end, down or right, each set when the node can scroll
that way from where it is. `range` is the span of the content in `unit`, both
ends inclusive: 0 items, a collection's items along the axis numbered from 0
(`[0, 0, 199]` for 200 rows), or 1 percent of the scrollable range, 0 to 100,
for a position the platform reports as a value between bounds. `position`,
present only with a range, is the part of it inside the viewport, both ends
inclusive: `[0, 15]` while the first 16 rows are in view, `[37, 37]` at 37
percent. A platform that reports neither leaves them out and the flags say
what it knows: Compose reports only the rows inside a list's viewport, so a
consumer tells what lies beyond them from these semantics rather than from
nodes. Values of `axis`, `unit` and flag bits this version does not define
survive a round trip.

### Events and streams

| Stream | Value | Elements after `time_us` | Sequence |
|---|---:|---|---|
| tree | 0 | `change u8, window u32, path [u32]` | lossless |
| window | 1 | `change u8, window u32` | lossless |
| frame | 2 | `width u32, height u32, rotation u8` | lossy |
| lifecycle | 3 | `change u8, message text` | lossless |

Every stream numbers its events from 1 on each connection. On a lossless
stream each event is one more than the last; any other number marks the
stream out of sync, the host drops its events and asks the agent to `resync`
it, and the stream is in sync again from the `seq` the answer names. A lossy
stream only increases; the numbers it skips are counted as dropped frames. An
agent that drops an event from a full queue skips its number
(`AgentSession::skip_event`) so the host sees the gap. Unknown streams are
tracked as lossless.

### Capabilities

A `u32` bitmask: tree (bit 0), input (1), screenshot (2), frames (3), events
(4), text_set (5). Unknown bits survive a round trip. The host refuses a
request whose method needs a capability the agent did not advertise, naming
it, and sends nothing; the agent refuses such a request too.

### Errors

| Code | Retryable | Meaning |
|---|:-:|---|
| `unknown_method` | no | the agent does not implement the method |
| `invalid_params` | no | parameters missing or malformed |
| `capability_missing` | no | the agent did not advertise what the method needs |
| `deadline_exceeded` | yes | the deadline passed before the work finished |
| `busy` | yes | the agent is still finishing an abandoned request |
| `window_changed` | yes | the foreground window changed during acquisition |
| `frame_too_large` | no | the message exceeds the negotiated frame size |
| `version_mismatch` | no | no common protocol version |
| `token_mismatch` | no | the hello carries another per-launch token |
| `frame_limit_too_small` | no | the negotiated frame size is under 4 KiB |
| `not_ready` | yes | the handshake has not completed |
| `disconnected` | no | the connection is gone; the message says to reconnect |
| `protocol_error` | no | a malformed or unexpected message |
| `internal` | no | an unexpected failure in the agent |

The flag travels with every error; `Error::new` takes it from this table.

## Sessions

`HostSession` and `AgentSession` are state machines without I/O. The caller
feeds bytes to `receive`, writes what `poll_transmit` returns, calls
`handle_timeout` at `poll_timeout`, and reads `poll_event`. Every
time-dependent method takes `now`; the sessions never read the clock, so
tests drive time themselves.

1. The agent speaks first: its hello, ready as soon as the session exists,
   doubles as proof that something listens behind an `adb forward`.
2. The host selects the highest version both speak and the smaller maximum
   frame size, checks the per-launch token if it has one, and answers with a
   welcome. On failure the welcome carries the error (no common version lists
   both version sets and asks to redeploy the agent) and the host closes.
3. Requests carry host-assigned ids, unique among those in flight; responses
   may arrive in any order.
4. A request may carry a relative deadline. The agent answers it with
   `deadline_exceeded` when the deadline passes and drops the late answer; the
   host reports the error itself when nothing arrives within the grace period
   after the deadline. The connection stays usable.
5. After a second without hearing from the other side a session pings; after
   three it closes as `Silent`, failing pending and later requests with
   `disconnected` and a reconnect hint. The end of the stream closes it at
   once.

## Defaults

| Constant | Value |
|---|---|
| `PROTOCOL_VERSION`, `SUPPORTED_VERSIONS` | 1, `[1]` |
| `frame::DEFAULT_MAX_FRAME` | 16 MiB (16,777,216 bytes), room for a full-resolution PNG screenshot, as agent-device's bridge allows |
| `frame::MIN_MAX_FRAME` | 4 KiB, so hellos, errors and pings always fit |
| `session::PING_INTERVAL` | 1 s |
| `session::PEER_TIMEOUT` | 3 s, within the spec's 5 s bound for a killed agent |
| `session::DEADLINE_GRACE` | 500 ms |
| `snapshot::MAX_NODE_DEPTH` | 256 |
| oversized-frame peek | 32 bytes |

## Golden frames and the Kotlin codec

`tests/common/samples.rs` defines 37 samples: one per message kind, parameter
and result type, with integers at every head-length boundary, u32 and u64
values past 2^31 and 2^63, multi-byte UTF-8, a byte string and a 25-element
array, plus window metadata in every state
(`response_snapshot_window_meta`) and scroll semantics in every shape
(`response_snapshot_scroll`). `golden/<name>.bin` is the frame this crate writes for each.
`agent-android/codec-test/Samples.kt` builds the same values in Kotlin.

```sh
cargo test -p protocol                                   # includes the golden checks
PROTOCOL_UPDATE_GOLDEN=1 cargo test -p protocol --test golden   # rewrite golden/ after a deliberate change
agent-android/codec-test.sh --write-golden               # Kotlin checks on the host JVM and a device; rewrites golden/kotlin/
agent-android/codec-test.sh --host-only                  # the host JVM only
```

`codec-test.sh` compiles the codec and its test with the flags of
`agent-android/build.sh`, adds the dexed classes to the same jar, runs it on
the host JVM and then once through `app_process` on the connected device. It
checks that every Kotlin sample encodes to the Rust frame byte for byte, that
every Rust frame decodes and re-encodes to itself, and that the Kotlin
decoder enforces the rules above. The Rust tests decode `golden/kotlin/` to
the same messages.

## Interpretations of the spec

- *Capability advertisement* says the agent advertises its capabilities after
  version negotiation. They travel in the same hello as its versions, as the
  research note and D4 put it, and the host acts on them only once it has
  selected a version.
- *Event channel* asks for events on a stream separate from responses. Event
  streams are logical: event frames, told apart from responses by their kind,
  on the same connection.
- `deadline_exceeded` is retryable: the spec defines the flag as whether
  retrying the same request is meaningful, and the agent stays responsive, so
  the same request can finish another time.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Framed self-describing messages | Oversized frame is rejected explicitly | `handshake::an_oversized_response_reaches_the_host_as_an_error_naming_the_limit_and_request`, `handshake::an_oversized_response_frame_is_refused_by_the_host_too`, `handshake::an_oversized_request_is_refused_before_it_is_sent`, `framing::an_oversized_frame_is_refused_naming_the_limit_and_the_request`, `framing::an_oversized_frame_arriving_in_pieces_is_discarded_and_the_stream_continues`, `framing::the_sender_refuses_a_frame_over_the_limit_and_writes_nothing`, `framing::an_oversized_frame_whose_header_cannot_be_read_still_names_the_limit` |
| | Well-formed frame round trip | `framing::well_formed_request_and_reply_round_trip`, `framing::a_frame_is_a_big_endian_length_then_one_cbor_item`, `framing::partial_frames_are_buffered_until_complete`, `framing::several_frames_in_one_read_come_out_in_order`, the round trips in `schema`, `golden::every_golden_frame_decodes_to_its_sample_with_definite_lengths`, `golden::kotlin_frames_decode_to_the_same_messages` |
| Version negotiation on connect | Common version selected | `handshake::both_sides_proceed_on_the_common_version`, `handshake::the_highest_common_version_is_selected` |
| | No common version | `handshake::no_common_version_closes_the_connection_listing_both_sets` |
| Capability advertisement | Command needs an unadvertised capability | `handshake::an_unadvertised_capability_is_refused_naming_it`, `handshake::agent_session_rejects_requests_without_the_advertised_capability`, `schema::capability_set_covers_every_required_capability` |
| Request multiplexing | Overlapping requests | `handshake::overlapping_requests_are_matched_by_id`, `handshake::request_ids_are_unique_and_never_zero` |
| Event channel with sequence numbers | Sequence gap detected | `events::a_sequence_gap_requests_a_resync_from_the_agent`, `events::a_gap_marks_the_stream_out_of_sync`, `events::a_repeated_or_earlier_number_also_marks_the_stream_out_of_sync`, `events::streams_are_tracked_independently`, `events::a_lossy_stream_counts_drops_without_a_resync` |
| Structured errors | Transient versus permanent failure | `schema::transient_errors_are_retryable_and_permanent_ones_are_not`, `schema::response_with_an_error_keeps_code_message_and_retryable_flag`, `handshake::the_agent_answers_an_unknown_method_with_a_permanent_error` |
| Deadlines and cancellation | Snapshot exceeds its deadline | `handshake::a_passed_deadline_is_answered_and_the_connection_stays_usable`, `handshake::the_host_reports_a_deadline_itself_when_the_agent_stays_silent` |
| Liveness detection | Agent process killed | `liveness::a_killed_agent_is_reported_disconnected_within_five_seconds`, `handshake::the_end_of_the_stream_fails_pending_requests_with_a_reconnect_hint`, `liveness::a_ping_goes_out_after_a_second_of_silence`, `liveness::a_hello_that_never_comes_times_out`, `liveness::bytes_of_a_discarded_frame_do_not_count_as_signs_of_life` |
