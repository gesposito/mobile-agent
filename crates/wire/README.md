# wire

The daemon's requests and replies as types (design D3 of the `mcp-server`
change): the envelope of a request line, one struct of parameters per
action under the names the daemon reads, and one struct of reply data per
action under the names its handlers write. The daemon reads its request
lines into them (`crates/daemon/src/wire.rs`), the CLI builds its
requests from them (`crates/cli/src/request.rs`), and the benchmark
harness reads what a snapshot cost the daemon through the snapshot's reply
type (`crates/bench/src/cli/mod.rs`). With the `schema` feature,
every type derives schemars' `JsonSchema`, for an MCP server's tool inputs
and outputs; the CLI and the daemon leave the feature off and link no schema
code.

## Layout

```
src/
  lib.rs        the crate's exports
  request.rs    Request (the envelope), Command (the action and its parameters), one struct per action,
                the parameters' defaults
  param.rs      Param, and how a request's parameters are read: lenient strings and flags, null as
                absent, the defaults and their tests, the schemas' joined descriptions
  reply.rs      Reply (the envelope), one struct of data per action, Details (what a failure carries)
tests/
  lines.json    the request line the CLI sends for each command and option: as it sent them before
                these types existed, and for close --keep, cancel, wait --change and
                wait --change --frames as the types write them
  requests.rs   those lines parse into their actions and serialize back; what a request keeps and ignores
  schemas.rs    with the feature, each request schema is a flat object of typed, described parameters
```

The reply types' test is the daemon's: `crates/daemon/tests/replies.rs`
reads every action's reply on a fake target into its type.

## Requests

A request line is one JSON object: the envelope's `id`, `session`,
`device`, `platform`, `force` and `defaultTimeout`, the `action`, and the
action's parameters beside them at the top level, as agent-browser's
requests carry them. `Request` holds the envelope and flattens `Command`,
an enum tagged by `action` whose variants hold the actions' structs:
`Status`, `Devices`, `Close`, `Cancel`, `SetTimeout`, `Doctor`, `Open`, `Snapshot`,
`DiffSnapshot`, `Get`, `Wait`, `Screenshot`, `Click`, `Fill`, `TypeText`
(the `type` action), `Press`, `Swipe`, `Scroll` and `Shutdown`. A key the
action does not take is ignored, as the daemon has always ignored it.

Every parameter's default is in its type: a request that gives none, as
Codex calls a tool with `{}`, reads as the daemon reads it, and the schema
publishes the default. `Option` stands for a parameter whose absence says
what no default can, such as `maxDepth` (no limit), `timeout` (the
session's default) or `click`'s `target` (a point instead). Serializing a
request leaves out a parameter at its default, which the daemon reads the
same: `swipe up` goes out as `{"action": "swipe", ...}` without its
direction.

The types read parameters as the daemon's handlers always did by name:

- A parameter given null is one not given.
- A string given a value of another type is no string: `scroll` with
  `"direction": 1` scrolls down, and `open` with `"app": 5` has no app.
- A flag is set by `true` alone.
- A number, a point or a list of strings given a value of another type
  keeps that value, as a `Param::Invalid`. The daemon reads a parameter only
  on the paths of a command that use it, and refuses such a value with
  `invalid_argument` only when it reads it, in the order its handler reads
  its parameters and after the target is resolved: `wait` reads `quiet` only
  to settle, and `swipe` its `distance` only without `from` and `to`. A
  request serializes the value back as it came.
- A string a command needs (`open`'s `app`, `get`'s `what` and `target`,
  `fill`'s `target`, `press`'s `key`) is empty when not given, which the
  daemon refuses as one not given; `fill`'s and `type`'s `text` is `None`
  when not given, since an empty text is one to enter.

## Replies

`Reply<T>` is agent-browser's envelope, `id`, `success`, `data`, `error`,
and `code` and `warning` when set, with `T` the action's data, such as
`reply::Snapshot`, or `reply::Details` for a failure. Every reply type
refuses fields it does not name. A reply whose shape depends on the request
has one struct with a field for each shape: `get`'s `text`, `value`
with a switch's or a checkbox's `checked`, or bounds, `wait`'s by what it
waited for, a change wait's delta object in `snapshot`, `screenshot`'s
image with `inline`, base64-encoded beside its `mimeType`, `close`'s
`kept` with `keep`, `open`'s `alert` when a dialog of system UI stands in
front of the app, `scroll`'s with `until` and without it, the latter's
`settled` and `movedOnMs` saying whether the screen came to rest after its
last stroke and how long the content moved on, and a snapshot's listing as
text or as the mirror's delta object (`reply::DeltaSnapshot`). `type`'s
data carries its read-back as `fill`'s does, what the field holds under
`text` beside the text typed under `typed`, and `screenshot`'s the
screen's size in points (`reply::Points`) beside the image's pixels.
`wait`'s `input`, for a settle within a second of the session's last
input, names that input's action, whose first effect the settle waited
for. `click`'s `hit` (`reply::Hit`) names the element its tap landed on by its
role and name, with the ref the session's snapshots listed it under or
null, and is null when no element with a ref lay under the point.
`click`'s, `fill`'s, `type`'s, `get`'s and `scroll`'s `resolved`
(`reply::Resolved`) names a stale ref the request named, under `stale`, and
the ref of its twin, the element the app rebuilt in its place, which the
action took, under `ref`; it is absent when the request named no stale ref.
`cancel`'s data (`reply::Cancel`) says whether a command of the session
was in flight and names its action and target.
`status`'s per-target `driver` report and `doctor`'s `iosRunner` are the
drivers' own objects, kept as JSON.

The handlers build their replies themselves (`crates/daemon/src/handlers`,
design D3); `crates/daemon/tests/replies.rs` holds these types to them. It
reads the reply of every action on a fake target, each shape of the
multi-shape replies, the failures a fake target gives with details
(`target_claimed`, `target_not_found`, `ambiguous_target`,
`element_offscreen`, `stroke_too_short`, `element_not_found` from a scroll,
`text_mismatch`) and those without, a warning on the envelope, a driver's
report, and every delta kind the mirror writes. A field a handler adds fails
the test until its type gains it.

## Schemas

`cargo test -p wire` builds the crate with the feature, through its
dev-dependency on itself, and `tests/schemas.rs` checks each action's
request schema: an object at the root without `anyOf`, `oneOf` or `allOf`,
which Claude's API refuses there, whose properties are the action's
parameters, each with a type and a description, and whose `required` names
the strings a command needs. Descriptions are the fields' doc comments, with
the lines a comment wraps joined. A parameter's schema is its type's
(`Param<T>` publishes `T`'s), and it shows the parameter's default.
