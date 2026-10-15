# Spike S5: wire format

Task 2.8 of `mobile-automation-foundation`. It benchmarks wire formats for the device link of design D4 on the "S5 benchmark plan" of `docs/research/protocol.md`, and replaces the research note's *model* numbers with measurements. The spike code is `spikes/s5-wire-format/`, kept privately since the repository was made public (`docs/spikes/README.md`); every table in Measurements is generated from its `results/` by `cargo run --release -- report`.

**In brief.** No candidate passes the plan's "3x better than JSON" criterion, and the only reason is bytes on the 500-node snapshot: CBOR is 2.49x smaller than JSON with readable keys, MessagePack 2.53x, protobuf 2.65x, where the research note's model predicted 2.35x to 2.60x. All three pass on 10,000 event frames (3.37x to 3.49x), are 7.6x to 21.7x faster than the faster platform JSON on the device-to-host path, and add 5.2 to 6.3 KB to the R8 dex against a 100 KB limit. The recommendation is CBOR, with minicbor on the host and a hand-rolled Kotlin codec on the device, chosen on time, dex size, evolution and debuggability while the criterion stays as written.

## Setup

**Host.** Apple M1 Max (8 performance and 2 efficiency cores), 64 GiB, macOS 26.6.2 (25G83). The host was shared with other agents building fixtures, booting a Simulator and running spikes S1 and S7 on the same emulator (see Measurement hygiene).

**Toolchains.** Rust 1.95.0 with serde 1.0.229, serde_json 1.0.151, minicbor 2.3.0 and minicbor-derive 0.19.5, ciborium 0.2.2, rmp-serde 1.3.1 on rmp 0.8.15, prost 0.14.4 and prost-derive 0.14.4; release profile with fat LTO and one codegen unit. Kotlin 2.4.20 (`kotlinc -jvm-target 17 -no-jdk -no-reflect -Werror` against `android-36/android.jar`), Temurin JDK 21.0.12, build-tools 37.0.0 with D8 and R8 9.2.4-dev, `--release --min-api 26` as in `agent-android/build.sh`. Every crate came through `sfw cargo add`; nothing came from Maven.

**Device.** `emulator-5554`: AVD `pixel_api36`, `sdk_gphone64_arm64`, Android 16 (API 36, build BE2A.250530.026.F3), arm64-v8a with 4 vCPUs, emulator 37.1.11 on the M1's hardware virtualization, ART 2.1.0 with the JIT on (`dalvik.vm.usejit=true`). The harness is a single `classes.dex` in a jar under `/data/local/tmp`, started by `app_process` as the shell user, so it runs without AOT compilation or a profile, as the agent will. No physical Android device was attached; that column is not measured.

**Code.** Branch `spike/s5-wire-format`. The measurements ran on the Rust codecs and harness of commit 5564f1b and the Kotlin codecs, device harness and `bench.sh` of commit 7bbf22e; the commit that adds this report adds the table generator (`s5 report`), the hygiene summary (`s5 hygiene`), `examples/prealloc.rs` and `results/`.

**Datasets.** No agent can capture the Compose fixture's 500-node screen yet (spike S1), so `src/generator.rs` generates the datasets with the research model's distributions from seed 20260926 (`data/manifest.json`); `cargo test` checks that the committed files match the generator byte for byte.

- `T500`: one snapshot response, 500 nodes, depth 7, a 16-entry string table (14 view classes, 2 resource-id prefixes), resource ids on 54.8% of nodes, text on 50%, content descriptions on 12.4%, bounds as four i32 (8 nodes with negative values), 8 flags in a u32 bitmask, a window id per node, 8,069 bytes of node strings including 2-, 3- and 4-byte UTF-8 and strings JSON must escape.
- `E10K`: 10,000 event frames, 8,975 on the tree stream and 1,025 on the window stream, contiguous sequences per stream, paths of 2 to 10 u16 (mean 6.07), microsecond timestamps above 2^32.
- `R1K`: 1,000 requests: 595 taps, 198 type requests with text, 207 snapshot requests with optional bounds; 691 carry a deadline.

**Schema.** Every codec encodes the same logical fields (`src/model.rs`, `kotlin/src/main/.../Model.kt`): integer enums, the bitmask, the string table and the interned prefixes. J is that schema as compact JSON with readable keys (`id`, `class`, `resPkg`, `resId`, `text`, `desc`, `bounds`, `flags`, `window`, `children`; events `kind`, `stream`, `seq`, `type`, `window`, `path`, `timeUs`), with absent fields left out. One frame is a 4-byte big-endian payload length plus the payload; `T500` is one frame, `E10K` 10,000 and `R1K` 1,000.

| Format | Rust | Kotlin on ART | Layout |
|---|---|---|---|
| JSON (J) | serde_json (`json`) | org.json (`json-orgjson`); `android.util.JsonWriter`/`JsonReader` (`json-jsonwriter`) | readable keys |
| CBOR | minicbor derive (`cbor`); ciborium through hand-written serde impls (`cbor-ciborium`) | hand-rolled, 391 lines | maps keyed by field index for the snapshot, nodes, requests and parameters; positional arrays for events |
| MessagePack | rmp-serde through hand-written serde impls (`msgpack`) | hand-rolled, 422 lines | the CBOR layout |
| protobuf | prost derive on hand-written structs, no `protoc` (`protobuf`) | hand-rolled wire format, 400 lines | proto3 field numbers, packed repeated scalars, zigzag bounds |
| context only | ciborium serde derive (`cbor-ciborium-named`); rmp-serde `to_vec` (`msgpack-array`) and `to_vec_named` (`msgpack-named`); Appium-style JSON (`appium-json`, bytes only) | | text keys; every struct an array; text keys; UiAutomator2 attributes as strings |

The Kotlin CBOR, MessagePack and protobuf encoders write exactly the bytes of minicbor, rmp-serde and prost (checked by `cargo test`), so each Rust decoder reads the same input whichever side wrote it.

**Method.**

- Rust (`s5 bench`, `s5 kotlin-decode`): 0.5 s warm-up, then samples until at least 100 and 1 s (at most 10,000). Encoding writes all frames of a dataset into a reused `Vec`. Decoding builds owned values, dropped outside the timed region; where the codec can, a second decoder builds values whose strings borrow from the input (serde_json copies only strings with escapes).
- Kotlin (`placeholder.s5.Main` through `app_process`): one fresh process per codec, dataset and round, three rounds. The first encode and the first decode of the process are timed on their own; then a 2 s warm-up, and for encoding into a reused buffer, decoding its own frames and decoding the Rust encoder's frames, samples until at least 200 and 2 s (at most 5,000), with `System.gc()` before each loop. Every run checks its decoded values against the dataset outside the timed loops. Tables give the median over rounds of each statistic.
- Percentiles are nearest-rank on both sides.
- Critical path: warm median Kotlin encode plus warm median Rust decode of the frames that Kotlin encoder wrote on the device, pulled with `adb pull` and committed in `golden/kotlin/`, for `T500` and `E10K`. For `R1K`, the host-to-device direction, Rust encode plus Kotlin decode of the Rust frames. On the device J is whichever of org.json and JsonWriter/JsonReader is faster: by encode for `T500` and `E10K`, by decode for `R1K`.
- Dex: `kotlin/build.sh` builds the harness with platform JSON only and the harness with platform JSON plus one codec, each through `R8 --release` with `kotlin-stdlib.jar` and a keep rule for `Main.main`, and with the stdlib's compile-only `annotations-13.0.jar` from the Kotlin distribution on `--classpath`, without which R8 stops on missing `org.jetbrains.annotations` classes; the delta is the difference of the `classes.dex` sizes, with `dexdump -f` class and method-reference counts. `d8 --release` of the harness's own classes, stdlib on the classpath only, is the unshrunk reference.

**Measurement hygiene.** Before every timing run `bench.sh` checked `uptime` and `pgrep -fl "gradle|xcodebuild|kotlinc"` and waited until no such process used CPU and the 1-minute load average was below 6; device runs also waited until no other `app_process` ran on the emulator, with no time limit. The final runs all started quiet: the 45 device runs (23:03 to 23:41) at a host 1-minute load of 3.91 to 5.64, median 4.63, with the emulator at 0.14 to 1.91; the Rust bench (22:01) at 5.10 and the Rust decode of the Kotlin frames (23:42) at 4.98. Before starting, device runs waited out 46 checks of 30 s: 43 for host load, from other agents' Simulator boots and builds, and 3 for another agent's `app_process` on the emulator. No run measured anyway. At 23:13:19 the shared adb server crashed, caused by another spike; the one device run in flight ended before it recorded anything and was repeated, and the golden frames had been pulled at 23:03, before the crash. An earlier pass of all device runs, made with the first version of the Kotlin CBOR decoder (see Observations), is superseded and not reported. `results/hygiene.tsv` lists every check.

**Rerunning.**

```sh
cd spikes/s5-wire-format                     # in the private copy
cargo test --release                            # datasets, Rust round trips, Kotlin golden frames
ANDROID_SERIAL=emulator-5554 kotlin/test.sh     # golden test on the device; refreshes golden/kotlin/
ANDROID_SERIAL=emulator-5554 ./bench.sh all     # env, dex, rust, crit, kotlin -> results/
cargo run --release -- report                   # the tables below
cargo run --release --example prealloc          # minicbor Vec preallocation
```

**Not measured.**

- A physical device: none was attached. Task 10.3 repeats the Kotlin timings on one.
- Kotlin library codecs (`kotlinx-serialization-cbor`, `msgpack-core`, `protobuf-javalite`, `flatbuffers-java`): they need Maven downloads, which Socket Firewall does not wrap.
- FlatBuffers, on both sides: no `flatc` is obtainable through Socket Firewall. The crates that vendor it, `flatc` 0.2.2+23.5.26 and `flatc-fork` 0.6.0+25.12.19, compile its C++ through the `cmake` crate, which needs a `cmake` binary that this host lacks and cargo cannot install; `flatbuffers-build` 0.2.4's `vendored` feature downloads the sources with reqwest inside its build script, outside Socket Firewall; `planus` compiles FlatBuffers schemas in pure Rust but generates code for its own runtime rather than the `flatbuffers` crate the plan names. On the device, `flatbuffers-java` needs Maven and a hand-rolled FlatBuffers builder was out of scope, as the research note expected.

## Measurements

### Bytes on the wire

4-byte frame headers included; J/C is how many times larger J is.

| Codec | Layout | T500 | E10K | R1K | J/C T500 | J/C E10K | J/C R1K |
|---|---|---:|---:|---:|---:|---:|---:|
| `json` | serde_json, readable keys (J) | 57,027 | 1,018,143 | 81,420 | 1.00x | 1.00x | 1.00x |
| `json-orgjson` | Kotlin org.json, readable keys | 57,029 | 1,018,143 | 81,420 | 1.00x | 1.00x | 1.00x |
| `json-jsonwriter` | Kotlin JsonWriter, readable keys | 57,027 | 1,018,143 | 81,420 | 1.00x | 1.00x | 1.00x |
| `cbor` | minicbor and Kotlin: integer-key maps, event arrays | 22,863 | 296,966 | 28,609 | 2.49x | 3.43x | 2.85x |
| `cbor-ciborium-named` | ciborium serde derive: text keys | 40,388 | 696,966 | 59,336 | 1.41x | 1.46x | 1.37x |
| `msgpack` | rmp-serde and Kotlin: integer-key maps, event arrays | 22,576 | 291,881 | 28,301 | 2.53x | 3.49x | 2.88x |
| `msgpack-array` | rmp-serde `to_vec`: every struct an array | 20,541 | 291,881 | 27,077 | 2.78x | 3.49x | 3.01x |
| `msgpack-named` | rmp-serde `to_vec_named`: text keys | 40,101 | 691,881 | 59,028 | 1.42x | 1.47x | 1.38x |
| `protobuf` | prost and Kotlin: field numbers | 21,516 | 301,763 | 23,623 | 2.65x | 3.37x | 3.45x |
| `appium-json` | UiAutomator2 attributes as strings (context only) | 203,680 | | | 0.28x | | |

### Rust on the host (µs, median / p95)

Host load: `22:01  up 21 days, 19:56, 1 user, load averages: 5.10 14.21 25.02` before, `22:02  up 21 days, 19:57, 1 user, load averages: 4.58 12.10 23.30` after.

| Codec | Dataset | Encode | Decode, owned | Decode, borrowed |
|---|---|---:|---:|---:|
| `json` | T500 | 68.2 / 70.8 | 150 / 158 | 139 / 158 |
| `json` | E10K | 1,264 / 1,325 | 2,679 / 2,760 | 2,676 / 2,826 |
| `json` | R1K | 98.1 / 104 | 200 / 207 | 203 / 211 |
| `cbor` | T500 | 26.9 / 28.2 | 59.1 / 61.7 | 49.4 / 51.8 |
| `cbor` | E10K | 284 / 302 | 915 / 950 | 927 / 944 |
| `cbor` | R1K | 39.7 / 40.4 | 74.2 / 77.4 | 70.7 / 71.8 |
| `cbor-ciborium` | T500 | 52.5 / 53.3 | 165 / 168 | n/a |
| `cbor-ciborium` | E10K | 736 / 764 | 2,012 / 2,165 | n/a |
| `cbor-ciborium` | R1K | 73.3 / 76.0 | 235 / 257 | n/a |
| `cbor-ciborium-named` | T500 | 58.2 / 60.8 | 184 / 191 | n/a |
| `cbor-ciborium-named` | E10K | 1,036 / 1,086 | 3,013 / 3,194 | n/a |
| `cbor-ciborium-named` | R1K | 81.2 / 85.4 | 299 / 322 | n/a |
| `msgpack` | T500 | 34.1 / 35.5 | 61.6 / 64.1 | 53.5 / 56.1 |
| `msgpack` | E10K | 378 / 413 | 597 / 622 | 606 / 641 |
| `msgpack` | R1K | 47.6 / 50.8 | 77.2 / 79.8 | 73.4 / 76.2 |
| `msgpack-array` | T500 | 25.6 / 28.1 | 61.7 / 64.2 | n/a |
| `msgpack-array` | E10K | 363 / 398 | 557 / 601 | n/a |
| `msgpack-array` | R1K | 33.0 / 34.0 | 70.0 / 72.8 | n/a |
| `msgpack-named` | T500 | 29.8 / 31.0 | 79.9 / 88.8 | n/a |
| `msgpack-named` | E10K | 625 / 687 | 1,317 / 1,369 | n/a |
| `msgpack-named` | R1K | 37.0 / 39.5 | 111 / 115 | n/a |
| `protobuf` | T500 | 53.4 / 56.0 | 56.5 / 60.9 | n/a |
| `protobuf` | E10K | 488 / 511 | 1,093 / 1,132 | n/a |
| `protobuf` | R1K | 28.3 / 29.4 | 49.7 / 54.6 | n/a |

### Rust decode of the frames the Kotlin encoders wrote (µs, median / p95)

Host load: `23:42  up 21 days, 21:36, 1 user, load averages: 4.98 27.03 71.36` before, `23:43  up 21 days, 21:37, 1 user, load averages: 6.83 24.69 68.47` after.

| Kotlin encoder | Rust decoder | T500 | E10K | R1K |
|---|---|---:|---:|---:|
| `json-orgjson` | `json` | 150 / 160 | 2,656 / 2,776 | 206 / 235 |
| `json-jsonwriter` | `json` | 147 / 156 | 2,761 / 2,936 | 206 / 232 |
| `cbor` | `cbor` | 59.5 / 65.2 | 901 / 940 | 73.1 / 78.6 |
| `cbor` | `cbor-ciborium` | 166 / 171 | 2,059 / 2,150 | 240 / 268 |
| `msgpack` | `msgpack` | 64.2 / 67.6 | 617 / 660 | 78.1 / 81.3 |
| `protobuf` | `protobuf` | 58.5 / 68.0 | 1,103 / 1,157 | 50.3 / 53.3 |

### Kotlin on the emulator (µs)

Median over 3 rounds of one `app_process` each. First call: the first encode and first decode in a fresh process (range over rounds in brackets). Warm: median and p95 after a 2 s warm-up, over at least 200 samples and 2 s of sampling, or 5,000 samples when those come first.

| Codec | Dataset | First encode | First decode | Encode median / p95 | Decode median / p95 | Decode of Rust frames |
|---|---|---:|---:|---:|---:|---:|
| `json-orgjson` | T500 | 2,425 (2,373–3,250) | 1,903 (1,183–2,395) | 1,815 / 2,795 | 1,240 / 2,017 | 1,261 / 2,031 |
| `json-jsonwriter` | T500 | 5,939 (5,708–6,184) | 1,443 (1,365–1,492) | 843 / 1,303 | 853 / 1,539 | 845 / 1,522 |
| `cbor` | T500 | 752 (750–1,168) | 734 (706–735) | 44.6 / 50.8 | 84.5 / 140 | 84.5 / 166 |
| `msgpack` | T500 | 1,122 (1,075–1,130) | 759 (713–769) | 45.7 / 48.2 | 88.4 / 151 | 88.0 / 169 |
| `protobuf` | T500 | 1,303 (1,223–1,471) | 1,407 (1,282–1,713) | 71.9 / 76.7 | 107 / 188 | 107 / 231 |
| `json-orgjson` | E10K | 47,019 (46,729–49,893) | 32,635 (32,264–33,085) | 42,311 / 57,052 | 30,042 / 32,374 | 30,650 / 33,535 |
| `json-jsonwriter` | E10K | 38,209 (37,863–57,037) | 36,778 (35,351–42,144) | 21,175 / 25,202 | 34,369 / 37,176 | 35,211 / 37,792 |
| `cbor` | E10K | 9,074 (7,046–9,491) | 10,628 (9,294–11,061) | 605 / 670 | 1,033 / 1,913 | 1,039 / 2,002 |
| `msgpack` | E10K | 6,639 (6,258–6,710) | 8,232 (8,208–8,673) | 485 / 540 | 921 / 1,768 | 935 / 1,816 |
| `protobuf` | E10K | 10,107 (9,694–11,047) | 6,884 (6,742–7,009) | 671 / 765 | 1,301 / 2,147 | 1,307 / 2,134 |
| `json-orgjson` | R1K | 4,500 (4,289–4,752) | 2,407 (2,237–3,107) | 3,028 / 3,601 | 2,080 / 2,712 | 2,187 / 2,736 |
| `json-jsonwriter` | R1K | 9,003 (8,638–9,099) | 3,632 (3,359–4,017) | 1,284 / 1,811 | 2,652 / 3,389 | 2,807 / 3,393 |
| `cbor` | R1K | 1,328 (1,236–1,552) | 1,371 (1,130–1,745) | 65.3 / 82.0 | 119 / 217 | 119 / 253 |
| `msgpack` | R1K | 1,259 (1,232–1,845) | 1,042 (995–2,212) | 64.1 / 77.8 | 115 / 205 | 113 / 248 |
| `protobuf` | R1K | 1,631 (1,423–1,890) | 839 (690–1,832) | 70.8 / 86.9 | 102 / 184 | 103 / 225 |

### Critical path (µs, warm medians)

Device to host (T500, E10K): Kotlin encode plus Rust decode of the Kotlin frames. Host to device (R1K): Rust encode plus Kotlin decode of the Rust frames. J uses the faster platform JSON encoder per dataset.

| Format | T500 encode + decode | T500 total | E10K encode + decode | E10K total | R1K encode + decode | R1K total |
|---|---:|---:|---:|---:|---:|---:|
| J (T500 `json-jsonwriter`, E10K `json-jsonwriter`, R1K `json-orgjson`) | 843 + 147 | 990 | 21,175 + 2,761 | 23,935 | 98.1 + 2,187 | 2,285 |
| CBOR | 44.6 + 59.5 | 104 | 605 + 901 | 1,506 | 39.7 + 119 | 159 |
| MessagePack | 45.7 + 64.2 | 110 | 485 + 617 | 1,102 | 47.6 + 113 | 160 |
| protobuf | 71.9 + 58.5 | 130 | 671 + 1,103 | 1,774 | 28.3 + 103 | 132 |

### Dex size

The harness with one codec minus the harness with platform JSON only, both through R8 with the stdlib; d8 is the harness's own classes, unshrunk.

| Variant | R8 `classes.dex` | Delta | Classes | Method refs | Delta | d8 unshrunk | Delta | Method refs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| json | 42,640 |  | 40 | 368 |  | 46,328 |  | 259 |
| cbor | 47,848 | +5,208 | 41 | 387 | +19 | 54,340 | +8,012 | 293 |
| msgpack | 48,912 | +6,272 | 41 | 390 | +22 | 55,372 | +9,044 | 295 |
| protobuf | 48,020 | +5,380 | 40 | 384 | +16 | 54,252 | +7,924 | 289 |

### Correctness

The golden cross-checks were written before any codec and failed first: `cargo test --release --test golden` 0 of 75 and `GoldenTest` on the emulator 0 of 30 (commit 77a29be). With the codecs they pass, 75 of 75 and 30 of 30: every Kotlin codec's frames decode in Rust to the logical datasets (owned, and borrowed where the Rust codec can), every Rust frame decodes in Kotlin, and the Kotlin CBOR, MessagePack and protobuf frames are byte-identical to minicbor's (and ciborium's), rmp-serde's and prost's. Every timing run on either side also checks its decoded values.

### Observations

- **Bytes.** The three binary formats are within 6% of each other on the snapshot and 3.4% on events, and which one is smallest depends on the message: protobuf on the snapshot (21,516 B, 5.9% under CBOR) and on requests (23,623 B, 17% under CBOR, because absent parameters and zero-valued fields cost nothing), MessagePack on events (291,881 B, 1.7% under CBOR). With text keys, the serde default of ciborium and rmp-serde, CBOR and MessagePack are only 1.41x to 1.47x smaller than J. Appium-style JSON is 3.6x the size of J and 8.9x that of CBOR.
- **Why the snapshot misses 3x.** J already carries the string table, the interned prefixes, integer enums and the flag bitmask, so what it spends beyond CBOR is keys, quotes and decimal digits: about 68 bytes per node. Strings are 8,491 bytes, 37% of the CBOR snapshot and 15% of J. One node: `{"id":26,"class":7,"resPkg":2,"resId":"title","text":"Flight","bounds":[48,-7,860,53],"flags":16,"window":154}` is 110 bytes, and its CBOR map, `{0: 26, 1: 7, 2: 2, 3: "title", 4: "Flight", 6: [48, -7, 860, 53], 7: 16, 8: 154}` in diagnostic notation, is 38 bytes (2.9x). Even positional MessagePack, which drops every key, reaches only 2.78x. Getting to 3x takes schema changes, such as interning resource-id names and repeated text or delta-coding bounds, not another format. Events have no strings and J spends a key on every field, so every binary format clears 3x there.
- **Rust on the host.** minicbor encodes the snapshot in 26.9 µs and decodes it in 59.1 µs (49.4 µs borrowing strings), against 68.2 and 150 µs for serde_json. On byte-identical input, ciborium is 1.8 to 2.6x slower than minicbor to encode and 2.2 to 3.2x slower to decode, and it cannot borrow. minicbor's derived `Vec` decode starts empty and grows, ignoring the length in the array header, so it decodes events in 915 µs against 597 µs for rmp-serde; `examples/prealloc.rs` decodes the event path with a `decode_with` that preallocates and measures 601 µs against 901 µs derived.
- **Kotlin on the emulator, warm.** The hand-rolled encoders are 12 to 44x faster than the faster platform JSON (snapshot: 44.6 µs for CBOR, 843 µs for JsonWriter; events: 605 µs against 21.2 ms), and the decoders 8 to 33x (snapshot: 84.5 µs against 853 µs for JsonReader; events: 1,033 µs against 30.0 ms for org.json). org.json is the slowest encoder, at 1.8 ms for the snapshot.
- **Kotlin, first call.** A fresh process pays for the interpreter and the JIT once: the first CBOR snapshot encode takes 752 µs, 17x its warm median, and the first 10,000 events 9.1 ms. Even cold, CBOR encodes the snapshot faster than warm JsonWriter.
- **CBOR against MessagePack on ART.** They are level on the snapshot and on requests. MessagePack encodes events 20% faster (485 against 605 µs) and decodes them 11% faster. The likely cause is the formats' one-byte integer ranges: MessagePack stores 0 to 127 in one byte and CBOR only 0 to 23, so child indices and ids from 24 to 127 cost CBOR a second byte and a less predictable branch. The encoder's code layout is not the cause: an A/B run with a one-byte fast path in the CBOR encoder left its event encode time unchanged. A first pass of the Kotlin CBOR decoder, which kept the major type in a field, decoded the snapshot in 97.6 µs; checking one-byte heads first, as the MessagePack decoder does, brought it to 84.5 µs with the same bytes. The tables report that version, and every device run was repeated with it.
- **Critical path.** CBOR is 9.5x faster than J on the snapshot (104 against 990 µs) and 15.9x on events (1,506 against 23,935 µs); MessagePack 9.0x and 21.7x, protobuf 7.6x and 13.5x. In the host-to-device direction, requests, CBOR is 14.4x faster (159 against 2,285 µs). Of MessagePack's 404 µs lead on events, 284 µs is minicbor's `Vec` growth and 120 µs the Kotlin encoder; with the preallocating decode the CBOR events path is about 1,206 µs.
- **Dex.** Each codec adds 5.2 to 6.3 KB to the R8 dex, or 16 to 22 method references; unshrunk, 7.9 to 9.0 KB. Neither the codec nor the format is what the agent's 300 KB target (task 4.10) depends on.

### Pass criterion

The plan's definition: for `T500` and `E10K` and each candidate C, J is the same logical messages as compact JSON with readable keys, serde_json on the host and the faster platform JSON on the device. C passes only if, on both datasets, bytes(J)/bytes(C) ≥ 3 with frame headers included, time(J)/time(C) ≥ 3 where time is the warm median Kotlin encode plus the warm median Rust decode, and the R8 dex delta is under 100 KB. Only candidates with a Kotlin codec can be timed this way, so the context-only rows are not candidates.

| Candidate | Dataset | bytes(J)/bytes(C) | time(J)/time(C) | R8 dex delta | Bytes ≥ 3 | Time ≥ 3 | Dex < 100 KB |
|---|---|---:|---:|---:|:-:|:-:|:-:|
| CBOR | T500 | 2.49x | 9.5x | 5,208 B | **no** | yes | yes |
| CBOR | E10K | 3.43x | 15.9x | 5,208 B | yes | yes | yes |
| MessagePack | T500 | 2.53x | 9.0x | 6,272 B | **no** | yes | yes |
| MessagePack | E10K | 3.49x | 21.7x | 6,272 B | yes | yes | yes |
| protobuf | T500 | 2.65x | 7.6x | 5,380 B | **no** | yes | yes |
| protobuf | E10K | 3.37x | 13.5x | 5,380 B | yes | yes | yes |

- CBOR fails: bytes on `T500`.
- MessagePack fails: bytes on `T500`.
- protobuf fails: bytes on `T500`.

## Verdict

**Go for CBOR, as D4 proposed, with the measured ratios recorded.** No candidate passes the plan's criterion, because on the 500-node snapshot no format is 3x smaller than JSON with readable keys: CBOR is 2.49x, MessagePack 2.53x, protobuf 2.65x, and even keyless positional MessagePack is 2.78x. The research note predicted this (2.35x to 2.60x for CBOR). As the plan prescribes, D4 records the ratios and decides on time, dex size, evolution and debuggability, and the baseline stays as written until something passes.

Why CBOR:

- **Bytes do not decide it.** The binary formats are within 6% of each other on the snapshot and 3.4% on events; protobuf is 17% smaller on requests, the smallest dataset. The snapshot's remaining gap to 3x is a schema question.
- **Time.** Every candidate beats J by 7.6x or more on the device-to-host path. CBOR is level with MessagePack on snapshots and requests; MessagePack's 20% lead in encoding events on ART is 12 ns per event.
- **Dex.** CBOR adds the least, 5.2 KB.
- **Spec.** The device-protocol spec requires a self-describing payload. CBOR and MessagePack are; protobuf is not and would need a spec change. protobuf also drops the zero values of non-optional fields, so the wire cannot tell zero from absent, and its encoder needs a sizes-first pass over the tree on the device.
- **Evolution.** minicbor derives the integer-keyed layout from field indices, skips unknown keys and decodes absent optional fields as `None`. Through serde, MessagePack (rmp-serde) and ciborium need the 400 lines of hand-written impls in `src/codecs/l1.rs`, because serde's derive keys fields by name and ciborium accepts only text or byte keys for fields.
- **Debuggability.** RFC 8949 defines a diagnostic notation for CBOR, which minicbor prints (`minicbor::display`, behind the `half` feature). MessagePack has no standard text form.
- **Headroom.** If a later message needs a list whose length is not known when it starts, CBOR has indefinite-length arrays; MessagePack and protobuf need the count or size first.

What it means for the tasks S5 gates:

- **Group 3, protocol crate.** Use minicbor with derive: integer keys by field index for maps, arrays for events, definite lengths on the wire so golden fixtures are byte-stable. Decode arrays with preallocation, a `decode_with` or a small wrapper type, since the derived `Vec` decode costs a third on events. The Kotlin decoder here rejects indefinite lengths; decide whether the wire ever uses them before task 3.5 cuts fixtures.
- **Task 3.5.** This spike's arrangement carries over: the Kotlin encoder writes byte-identical frames, so golden fixtures compare byte for byte in both directions. The Kotlin CBOR codec for this message set is 391 lines with its decoder; check one-byte heads first.
- **Task 4.10.** The codec adds about 5 KB, and the harness with platform JSON shrinks to 42.6 KB with R8, so the 300 KB target is about keeping R8 on the stdlib. R8 needs `lib/annotations-13.0.jar` of the Kotlin distribution on `--classpath` (see `kotlin/build.sh`).
- **S1 and S6.** Encoding the 500-node tree costs 45 µs on the device and decoding it 60 µs on the host. Compare that with S1's tree-read latency, and convert the byte ratios into milliseconds with S6's transfer rate, before optimizing further.
- **Not measured.** A physical device (task 10.3 repeats the Kotlin timings on one), the Kotlin library codecs, and FlatBuffers.

### Proposed replacement for design.md D4

To paste over D4 in `openspec/changes/archive/2026-10-07-mobile-automation-foundation/design.md`:

> ### D4. Wire format: CBOR, measured by spike S5
> Frames are a 32-bit big-endian payload length followed by one CBOR item (RFC 8949) in preferred serialization with definite lengths. Requests, responses, snapshots and tree nodes are maps keyed by small integers, the field index, so absent optional fields cost nothing and receivers skip keys they do not know; events are positional arrays, because at thousands of frames the keys are the overhead. Message kinds: request with id, method and params; response with id and either result or error; event with stream, sequence and payload. Snapshots carry a per-snapshot string table for class names and resource-id prefixes; enums are integers and node flags a bitmask. The host uses minicbor with derive (no_std, borrowed strings, index-based evolution rules) and preallocates arrays from their length; the Kotlin agent hand-rolls the encoder and decoder for the fixed message set, about 400 lines with no dependency. Version and capability negotiation are the first exchange on every connection.
>
> Spike S5 (`docs/spikes/s5-wire-format.md`) measured CBOR, MessagePack and protobuf against compact JSON with readable keys on the same logical fields: a 500-node snapshot, 10,000 event frames and 1,000 requests, with Kotlin on an API 36 emulator and Rust on the host. JSON is larger than CBOR by 2.49x on the snapshot, 3.43x on events and 2.85x on requests (MessagePack 2.53x, 3.49x and 2.88x; protobuf 2.65x, 3.37x and 3.45x). On the device-to-host path, Kotlin encode plus Rust decode, CBOR is 9.5x faster than the faster platform JSON on the snapshot and 15.9x on events (MessagePack 9.0x and 21.7x, protobuf 7.6x and 13.5x). Each codec adds 5.2 to 6.3 KB to the R8 dex. No candidate meets the pass criterion (at least three times smaller and faster than JSON on both the snapshot and the events, under 100 KB of dex), only because no format is 3x smaller on the snapshot: the string table, interned prefixes and integer enums already shrink its JSON, and strings are 37% of its CBOR. The criterion stays as stated, and CBOR is chosen on time, dex, evolution and debuggability. It is within 6% of the smallest candidate on the snapshot and 2% on events (protobuf is 17% smaller on requests), and adds the least dex; it is self-describing, as the device-protocol spec requires and protobuf is not; minicbor derives the integer-keyed layout that serde formats need hand-written impls for, and decodes 2.2 to 3.2x faster than ciborium; frames print in RFC 8949 diagnostic notation; and indefinite-length arrays remain available for lists of unknown length. Alternatives: MessagePack (as small, and 20% faster at encoding events on ART, but no integer-keyed derive in rmp-serde, no standard text form, and counts before elements); protobuf (not self-describing, zero values of non-optional fields dropped, a sizes-first pass on the device; its hand-rolled wire codec adds only 5.4 KB, so the dex is no longer the objection); FlatBuffers (not measured, since `flatc` has no install path through Socket Firewall); JSON over HTTP (per-node cost and no event channel). The Kotlin library codecs and a physical device were not measured; the former need Maven, which Socket Firewall does not wrap, and task 10.3 repeats the Kotlin timings on a device.
