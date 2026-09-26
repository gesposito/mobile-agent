# S2: Simulator lifecycle and display surface frames

Spike S2 (task 2.5) checks two assumptions from Rust. The first is D6's: the Simulator lifecycle can be driven from the host through CoreSimulator with objc2. The second is D7's: live frames can come from the display surface, with the IOSurface cached from its change callback, then be downscaled, hashed and JPEG-encoded on a worker thread. Nothing was installed in the Simulator except the app under test. Both hold. The verdict is go for both, with turbojpeg as the encoder.

## Setup

- Host: Apple M1 Max (8 performance and 2 efficiency cores), 64 GB, macOS 26.6.2 (25G83). Other agents' builds, benchmarks and an Android emulator ran intermittently throughout; every measured step records the load averages it started under (below).
- Xcode 27.0 (27A266a). CoreSimulator 1171.7 is in `/Library/Developer/PrivateFrameworks`. It re-exports CoreSimDeviceIO 1171.7 and links ROCKit 47 from the same directory. SimulatorKit 1005.2 was not loaded, because S2 needs nothing from it. Runtime iOS 26.5 (23F77).
- Devices: `mobile-agent-spike`, an iPhone 17 (1206x2622 pixels at 3x) booted headless, carried the frame measurements. `mobile-agent-s2` was created, used, shut down and deleted twice for the lifecycle. DeviceHub.app and Simulator.app were never running.
- App: the Expo fixture (`dev.mobileagent.fixture.expo`), Release build of main at `a443dc0`, which reads the `FIXTURE_INITIAL_TAB` launch option from fixtures/CONTRACT.md.
- Rust 1.95.0. Crates (all added with `sfw cargo add`), from `Cargo.lock`:
  - Runtime: `objc2` 0.6.4 with the `exception` feature, `objc2-foundation` 0.3.2, `objc2-core-foundation` 0.3.2, `objc2-io-surface` 0.3.2, `block2` 0.6.2, `dispatch2` 0.3.1, `libc` 0.2.189.
  - `turbojpeg` 1.5.1 (`turbojpeg-sys` 1.2.0). Its default build compiles the vendored libjpeg-turbo with cmake, which this host lacks, so it links the host's libjpeg-turbo 3.2.0 statically through `TURBOJPEG_SOURCE=explicit` in `.cargo/config.toml`.
  - `jpeg-encoder` 0.7.1, which is scalar on arm64: its `simd` feature is AVX2 only. `zune-jpeg`, the other suggested candidate, is a decoder only.
  - `png` 0.18.1.
  - Accelerate's public `vImageScale_ARGB8888`, as a reference scaler.
- Both profiles set `panic = "unwind"`, which `objc2::exception::catch` needs.
- Code: `spikes/s2-simulator-frames` at `64926b8`. The uncapped stream runs were made before a change to the `--max-fps` pacing, which is not on their code path.
- Rerun: `cargo build --release`, then `./measure.sh frames <udid> <FixtureExpo.app> <out-dir>` and `./measure.sh lifecycle <FixtureExpo.app> <out-dir>`. `cargo test` runs the unit tests and the `list` smoke test against the host's CoreSimulator.
- Loading: `dlopen(RTLD_NOW | RTLD_GLOBAL)` of `CoreSimulator.framework/Versions/A/CoreSimulator`, then of `.../Frameworks/CoreSimDeviceIO.framework/Versions/A/CoreSimDeviceIO`. An `RTLD_NOLOAD` probe shows that CoreSimulator's re-export had already mapped the second.
- `dlsym` names: none. S2 uses no private C function.
- The private surface is Objective-C only:
  - `objc_lookUpClass` finds `SimServiceContext`. `SimDeviceSet`, `SimDevice`, `SimRuntime`, `SimDeviceType`, `SimDeviceBootInfo` and `SimDeviceIOClient` are reached as instances.
  - `objc_getProtocol` finds `SimDisplayRenderable`, `SimDisplayIOSurfaceRenderable` and `SimScreen`.
- Public C used: IOSurface (`IOSurfaceLock`, `IOSurfaceUnlock` and the `IOSurfaceGet*` accessors for base address, bytes per row, pixel format, seed, ID and allocation size), libdispatch, `sysctl(KERN_PROCARGS2)`, `getrusage` and vImage. The selectors are listed under [Call sites](#call-sites).

## Measurements

Timings are wall-clock in milliseconds from `Instant`. CPU is `getrusage(RUSAGE_SELF)` user plus system time over the measured window, as a percentage of one core. The Load columns give the 1-minute load average when the step started; the `measure.sh` logs record the 5 and 15 minute averages too. Stream statistics cover every frame in the window with no warm-up excluded. `bench` excludes 10 warm-up iterations.

### Tests, red then green

- `be8ffad` ran `cargo test` with every pipeline function stubbed. All 22 pipeline unit tests failed on `unimplemented!`, covering the BGRA view, fit-within sizing, the box-filter downscale, BGRA to RGBA, the frame hash, the change detector and the timing summary. `list_matches_simctl` failed with "list: CoreSimulator bindings are not implemented yet".
- `2309e11` made the `list` smoke test pass: every device `xcrun simctl list devices` reports is listed with the same name and state, a runtime and a UDID.
- `64926b8` made all 28 unit tests and both integration tests pass. The 28 are 22 pipeline, 2 exception-guard and loader, 2 `KERN_PROCARGS2`, 1 encoder and 1 vImage.

### Lifecycle

Release build, device type iPhone 17, runtime iOS 26.5. Each command is a separate process. Loading both frameworks and connecting (`+[SimServiceContext sharedServiceContextForDeveloperDir:error:]`) took 323 to 333 ms per process; the times below exclude it.

| Step | Run 1 | Run 2 | Notes |
|---|---|---|---|
| create | 158 ms | 269 ms | |
| first boot, completion handler | 756 ms | 682 ms | the state already reads Booted |
| first boot, boot to terminal `bootStatus` | 43.4 s | 146.2 s | start loads 4.3 and 10.3; an earlier functional run at load 11 took 63.2 s |
| install, first on the device | 20.1 s | 7.45 s | run 1 hit IXErrorDomain 2 once and succeeded 3 s later |
| launch with environment | 144 ms | 132 ms | until the pid arrives |
| terminate | 25 ms | 12 ms | |
| shutdown | 3.36 s | 3.35 s | |
| second boot, boot to terminal `bootStatus` | 10.4 s | 11.2 s | completion handler 675 and 718 ms |
| launch after the second boot | 327 ms | 217 ms | |
| shutdown | 3.08 s | 3.16 s | |
| delete | 5.6 ms | 7.4 ms | completion arrives once the device is marked deleted |

Run 1 started at load 4.27 and 6.86. The first boot then set off a burst of security scanning (XprotectService, syspolicyd) and I/O that pushed the 1-minute load to 198, so run 1's install and later steps ran inside it. Run 2 waited for the burst to pass: install through the second boot started at loads of 4.6 to 5.7, and the second boot's own burst reached 63 before the final shutdown. Reinstalling on the long-running `mobile-agent-spike` took 0.74 and 1.44 s.

Launch with environment was proven four ways:

1. `sysctl(KERN_PROCARGS2)` on the pid that the completion handler returns shows every variable passed (`FIXTURE_INITIAL_TAB=anim` plus a marker), for every launch above.
2. `ps -wwE` on the host shows the same.
3. `xcrun simctl spawn <udid> launchctl print user/501/UIKitApplication:dev.mobileagent.fixture.expo[...]` lists `FIXTURE_INITIAL_TAB => anim` under the job's environment.
4. The fixture opened on the Anim tab instead of its default Login tab. That screenshot came through the spike's own display-surface path.

The `environment` dictionary goes to the process unchanged; simctl's `SIMCTL_CHILD_` prefix is simctl's own convention.

### Frames: static and on-demand changes

Login tab, max-dim 1000 (460x1000), turbojpeg and jpeg-encoder both enabled. Every stream registered the SimScreen callbacks. The surfaces callback reported the current surface 1.7 to 1.8 ms after registration and never again: one surface for the whole session.

| Scenario | Window | Frame callbacks | Changed frames | CPU | Load |
|---|---|---|---|---|---|
| static | 10.0 s | 1, with the same hash as the baseline | 0 | 0.1% | 7.58 |
| appearance dark/light every 2 s, 5 toggles | 12.0 s | 175: 35 per toggle, about 34 presents over 499 to 515 ms | 0; one distinct hash in 170 frames | 8.2% | 7.83 |
| status bar clock 09:41/10:10 every 2 s, 5 toggles | 12.0 s | 34: 2 per toggle, plus 24 before the first toggle from the previous run's appearance restore | 5, one per toggle, 305 to 350 ms after the command started | 1.7% | 7.42 |

Frames stop arriving when the screen is static. Every appearance toggle produces a burst of frames, but the fixture pins `userInterfaceStyle: light` and a dark-content status bar, so the pixels never change. The hash sees that, and a frame count would not. The status bar clock is an on-demand change that does alter pixels: each toggle yields exactly one changed frame, as soon as `xcrun simctl status_bar` returns (287 to 339 ms). The 32-pixel thumbnail hash separates the three clock states.

### Frames: sustained animation

Anim tab (endless rotation), 10 s per run. The capped runs throttle to 30 fps and read on the first callback after three quarters of the interval.

| Output | Encoders | Callbacks | Processed | Changed | CPU | Read, median / p95 | Hash | turbojpeg, median / p95 | jpeg-encoder, median / p95 | Load |
|---|---|---|---|---|---|---|---|---|---|---|
| 460x1000 | none | 59.9 fps | 59.9 fps | 600 of 600 | 16.8% | 2.53 / 2.64 | 0.12 | | | 6.00 |
| 460x1000 | turbojpeg | 60.1 fps | 60.1 fps | 601 of 601 | 20.9% | 2.52 / 2.61 | 0.12 | 0.72 / 0.75 | | 5.46 |
| 460x1000 | jpeg-encoder | 59.9 fps | 59.8 fps | 597 of 599 | 30.9% | 2.54 / 4.41 | 0.12 | | 2.14 / 2.27 | 7.48 |
| 460x1000 | both | 60.2 fps | 60.1 fps | 596 of 602 | 35.7% | 2.54 / 4.72 | 0.12 | 0.73 / 0.75 | 2.14 / 2.28 | 9.01 |
| 460x1000, 30 fps cap | turbojpeg | 60.0 fps | 29.4 fps | 294 of 294 | 11.8% | 2.56 / 4.42 | 0.12 | 0.73 / 0.78 | | 9.90 |
| 460x1000, 30 fps cap | turbojpeg | 60.1 fps | 29.6 fps | 296 of 296 | 11.9% | 2.56 / 4.80 | 0.12 | 0.74 / 0.78 | | 8.84 |
| 460x1000, 30 fps cap | jpeg-encoder | 60.1 fps | 29.6 fps | 296 of 296 | 16.2% | 2.57 / 4.83 | 0.12 | | 2.15 / 2.29 | 9.38 |
| 1206x2622 | none | 59.9 fps | 59.9 fps | 600 of 600 | 9.3% | 0.57 / 0.81 | 0.80 | | | 9.88 |
| 1206x2622 | turbojpeg | 60.0 fps | 60.0 fps | 600 of 600 | 37.2% | 0.62 / 0.83 | 0.80 | 4.57 / 4.81 | | 9.05 |
| 1206x2622 | jpeg-encoder | 60.0 fps | 55.7 fps | 557 of 558 | 87.7% | 0.57 / 0.80 | 0.80 | | 14.25 / 22.48 | 8.52 |
| 1206x2622 | both | 60.1 fps | 50.2 fps | 502 of 502 | 99.6% | 0.49 / 0.64 | 0.78 | 4.47 / 4.75 | 13.83 / 15.07 | 9.56 |

Notes on the table:
- Read is lock, copy or box downscale into the worker's buffer, and unlock.
- Frames not processed were coalesced into the next read. Encoded sizes were 16.3 KiB (turbojpeg) and 16.5 KiB (jpeg-encoder) at 460x1000, and 76.2 and 76.7 KiB at native size.
- Frame callbacks were 16.66 ms apart at the median (p95 18.3 ms).
- At 460x1000 the downscale, not the encoder, is the largest cost: 2.5 ms of about 3.4 ms per frame with turbojpeg.

### Encoders on identical frames

`bench` copies one frame out of the surface, then times each stage 200 times after 10 warm-up iterations. Quality 80, 4:2:0 chroma, standard Huffman tables in both encoders. The Login and Anim frames are sparse; the Big tab's 500-cell grid is dense.

| Frame | Output | turbojpeg, median / p95 | jpeg-encoder, median / p95 | Ratio | turbojpeg bytes | jpeg-encoder bytes |
|---|---|---|---|---|---|---|
| Login | 1206x2622 | 4.41 / 4.58 | 13.84 / 14.72 | 3.1x | 85,809 | 86,037 |
| Login | 460x1000 | 0.69 / 0.73 | 2.09 / 2.14 | 3.0x | 18,493 | 18,791 |
| Login | 331x720 | 0.37 / 0.42 | 1.09 / 1.13 | 2.9x | 11,636 | 11,831 |
| Anim | 1206x2622 | 4.38 / 4.58 | 13.77 / 14.18 | 3.1x | 78,388 | 78,840 |
| Anim | 460x1000 | 0.68 / 0.71 | 2.06 / 2.11 | 3.0x | 16,793 | 16,862 |
| Anim | 331x720 | 0.36 / 0.37 | 1.08 / 1.10 | 3.0x | 10,458 | 10,570 |
| Big | 1206x2622 | 5.51 / 5.73 | 16.16 / 16.50 | 2.9x | 353,301 | 354,044 |
| Big | 460x1000 | 0.98 / 1.04 | 2.68 / 2.82 | 2.7x | 82,219 | 83,105 |
| Big | 331x720 | 0.51 / 0.66 | 1.44 / 1.49 | 2.8x | 45,056 | 45,404 |

The other stages from the same runs:

| Stage | Result |
|---|---|
| copy at native size | 0.37 to 0.43 ms |
| box downscale to 460x1000 | 2.46 to 2.52 ms, against 3.12 to 3.37 ms for vImage's Lanczos on one thread |
| box downscale to 331x720 | 1.78 to 1.84 ms, against 2.50 to 2.91 ms for vImage |
| frame hash | 0.12 ms at 460x1000; 0.78 to 0.80 ms from the native frame |

### Surface facts

- Pixel format is `BGRA`, 1206x2622, 4864 bytes per row: 40 bytes of padding on a 4824-byte row.
- The same IOSurface (one ID) served every stream in the session.
- The seed advanced between every pair of consecutive frames during the animation (599 of 599).
- Tearing: the render server writes the surface without waiting for a reader's read-only lock. The worker checks whether the seed moved between lock and end of copy, which marks a possible tear:
  - Worker reading right after each callback and keeping up: 0 of about 600 frames in four of the 60 fps runs; 9 of 602 with both encoders (5.5 ms of work per frame); 4 to 6 during the bursts of the toggle runs.
  - Worker falling behind (native size with jpeg-encoder): 5 of 558 and 16 of 502.
  - 30 fps throttle reading a stale callback late: 38 of 288 and 46 of 289.
  - 30 fps throttle waiting for a fresh callback: 1, 1 and 0 of about 295.
- `-framebufferSurface`, 200 calls: median 0.305 ms, p95 2.96 ms, maximum 11.2 ms, first call 0.52 ms. Finding the main display (`-io`, `-ioPorts`, 14 descriptors) took 4 to 10 ms.
- A native-size screenshot to PNG took 54.7 ms: 7.3 ms to find the display, 0.4 ms to fetch the surface, 6.8 ms to read and convert, and 40.2 ms to encode the PNG and write it. `xcrun simctl io screenshot` took 0.55 to 0.60 s warm and 1.35 s cold.
- Legacy path, in a 3 s functional run: the damage-rectangles callback also ticked at 59.8 fps. The plural `ioSurfacesChangeCallback` never reported the current surface at registration, so the stream fell back to one `-framebufferSurface` after its 2 s wait.

## Call sites

Every private call site worked from Rust through objc2, and none needed the Objective-C shim. The CoreSimulator objects are real classes: `msg_send!`, including its `error: _` form, worked with objc2's debug-build signature checks enabled. From `-ioPorts` onward, CoreSimDeviceIO hands out ROCKit proxies that answer by forwarding:
- IO ports: `ROCKRemoteProxy-<uuid>-ROCKImpersonateable-SimDeviceIOPortInterface`.
- Display descriptors: `...-SimScreen-SimDeviceIOPortDescriptorInterface-...-SimDisplayRenderable`.
- Descriptor states: `ROCKImmutableProxy`.

`class_getInstanceMethod` finds none of their methods, and objc2's debug-build check of `msg_send!` panics on them with "method not found"; release builds skip that check. The spike sends to them through `forward::send`. That helper calls `objc_msgSend` through objc2's hidden `EncodeArguments::__invoke`, the same `C-unwind` call `msg_send!` makes after its check, so exceptions still reach the guard. In debug builds it compares the Rust signature with the proxy's own `-methodSignatureForSelector:`.

| Call | Receiver | Outcome |
|---|---|---|
| `+sharedServiceContextForDeveloperDir:error:`, `-defaultDeviceSetWithError:`, `-supportedRuntimes`, `-supportedDeviceTypes` | `SimServiceContext` | works |
| `-devices`, `-createDeviceAsyncWithType:runtime:name:completionQueue:completionHandler:`, `-deleteDeviceAsync:completionQueue:completionHandler:` | `SimDeviceSet` | works; block2 blocks, dispatch2 queue |
| `-UDID`, `-name`, `-state`, `-stateString`, `-runtime`, `-deviceType`, `-io` | `SimDevice` | works |
| `-bootAsyncWithOptions:completionQueue:completionHandler:` with `{persist: YES}` | `SimDevice` | works; the device outlives the booting process |
| `-bootStatus`, then `-status` (`I`), `-isTerminalStatus`, `-bootElapsedTime` | `SimDevice`, `SimDeviceBootInfo` | works; terminal status is `0xffffffff` |
| `-shutdownAsyncWithCompletionQueue:completionHandler:` | `SimDevice` | works |
| `-installApplication:withOptions:error:` | `SimDevice` | works; IXErrorDomain 2 arrives as an `NSError` and a retry succeeds |
| `-launchApplicationAsyncWithID:options:completionQueue:completionHandler:` with `arguments` and `environment` | `SimDevice` | works; the block receives `(NSError *, pid_t)` |
| `-terminateApplicationWithID:error:` | `SimDevice` | works; an app that is not running gives `NSPOSIXErrorDomain` 3 |
| `-name`, `-versionString`, `-buildVersionString`, `-identifier` / `-mainScreenSize`, `-mainScreenScale` | `SimRuntime` / `SimDeviceType` | works |
| `-ioPorts` | `SimDeviceIOClient` | works |
| `-descriptor` (`@`) | port proxy | works through `forward::send` |
| `-conformsToProtocol:`, `-respondsToSelector:`, `-methodSignatureForSelector:` | proxies | works; the proxy classes implement them |
| `-state` (`@`), `-displayClass` (`S`) | descriptor proxy, `ROCKImmutableProxy` | works through `forward::send` |
| `-framebufferSurface` (`@`) | descriptor proxy | works through `forward::send` |
| `-registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:` (`v@@@?@?@?`), `-unregisterScreenCallbacksWithUUID:` | descriptor proxy | works through `forward::send` with signed blocks |
| `-registerCallbackWithUUID:ioSurfacesChangeCallback:`, `-registerCallbackWithUUID:damageRectanglesCallback:` and their unregister calls | descriptor proxy | works; kept as a fallback |

ROCKit marshals the SimScreen callbacks to the render server and needs each block's signature. A block2 `RcBlock::new` block has none, and registering it raised `NSInternalInconsistencyException` "Block is missing signature field". `objc::guarded` caught that as an error and the process kept running. Blocks built with `RcBlock::with_encoding` and the signatures `v8@?0`, `v16@?0@8` and `v24@?0@8@16` register and fire. The local CoreSimulator completion blocks work without signatures. The exception guard also turns a real `NSRangeException` into an error in a unit test.

Not exercised: `SimScreenAdapter -enumerateScreensWithCompletionQueue:completionHandler:`, because `displayClass == 0` found the main display and its descriptor conforms to `SimScreen` directly; `-screenProperties`; `-registerNotificationHandler:`; `-dataPath`.

## Verdict

**Go for D6's lifecycle bindings.**
- Loading CoreSimulator with `dlopen` and driving it through objc2 covers list, create, boot, readiness, install, launch with arguments and environment, terminate, shutdown and delete.
- Every error came back as an `NSError` or a caught exception, never a crash.
- Readiness is `-[SimDevice bootStatus]` reaching a terminal status, 10 to 146 s after the boot completion handler, which fires when the state flips to Booted.

**Go for D7's frame path.**
- SimScreen callbacks give a per-present tick: 60 fps during an animation, none on a static screen.
- The surfaces callback hands over the one IOSurface once, so the cache never needs refreshing within a session.
- BGRA is confirmed.
- At max-dim 1000 with turbojpeg the whole pipeline costs 11.8 to 11.9% of one core at 30 fps and 20.9% at 60 fps. That meets task 6.2's gate of 30 fps under 20% with room to spare; jpeg-encoder also meets it at 16.2%.

**Encoder: turbojpeg.**
- On identical frames it is 2.7 to 3.1 times faster than jpeg-encoder: 0.68 to 0.98 ms against 2.06 to 2.68 ms at 460x1000, and 4.4 to 5.5 ms against 13.8 to 16.2 ms at native size.
- Output sizes match within 2%.
- At native size, jpeg-encoder alone cannot keep up with 60 fps: it managed 55.7 fps at 87.7% of a core.
- The cost is a C dependency whose default build needs cmake, and nasm on x86_64. `crates/simulator` has to choose between that build and linking a system copy.
- jpeg-encoder is a dependency-free fallback that still fits the 30 fps budget at max-dim 1000.

What this means for the gated work:
- 6.1 can port `coresim.rs`. Launch should wait for the terminal boot status, not the completion handler.
- 6.2 can port the pipeline with turbojpeg. It should keep three rules:
  - Read on the frame callback.
  - When throttling, skip presents rather than read late.
  - Treat the box downscale as the main per-frame cost. A GPU scaler is the next step if 60 fps must fit in 20%; vImage's Lanczos on the CPU was slower than the box filter.
- For settle (D5, D8) and task 2.11: callbacks overcount. A presentation without a pixel change still fires them, 35 per appearance toggle here, so settle has to use hash changes, as D8 already says, not callback counts.

Proposed corrections and additions to `docs/research/simulator.md`. This spike does not edit that note or design.md.

1. Every object CoreSimDeviceIO vends from `-ioPorts` on is a ROCKit forwarding proxy. ROCKit itself (`/Library/Developer/PrivateFrameworks/ROCKit.framework`, linked by CoreSimulator) is missing from the note's image list. objc2's debug-build `msg_send!` check panics on these proxies, so D6's "objc2 dynamic messaging" needs a forwarded-send helper, or objc2's `disable-encoding-assertions` feature, on the display path.
2. Blocks passed to ROCKit must carry a signature. With block2 that means `RcBlock::with_encoding`. Without one, SimScreen registration raises "Block is missing signature field".
3. The note, following baguette, calls `-framebufferSurface` a synchronous XPC round trip. That is true but cheap: 0.3 ms median, 3 ms p95. D7's reason to cache still holds, and it is stronger than cost: the surface is fixed for the session and reported once at registration.
4. The pixel format is BGRA, and rows are padded to 4864 bytes for a 1206-pixel width. The seed advances on every present. A read can tear, and does when it starts late.
5. The note says S2 needs a readiness signal. It is `-[SimDevice bootStatus]` with `isTerminalStatus`, which idb's boot verification uses; the note does not name it.
6. `SimScreenAdapter` is not needed on 1171.7. The main display's descriptor conforms to `SimScreen`, and `displayClass == 0` finds it among 14 IO ports.
7. The legacy `ioSurfacesChangeCallback` does not report the current surface at registration on 1171.7.
8. The Rust building blocks table needs three notes: `turbojpeg`'s default features require cmake; `jpeg-encoder`'s SIMD is x86 only; `zune-jpeg` has no encoder.
9. "What we take" item 10 plans turbojpeg against ImageIO, and scaling with Core Image or on the CPU. This spike compared turbojpeg with jpeg-encoder, as task 2.5's brief asked. ImageIO and Core Image were not measured. A Rust box filter beat vImage's single-threaded Lanczos.
