# Spike reports

One report per spike, S1 to S7 from group 2 of the archived change `mobile-automation-foundation` and S8 from task 1.2 of `microbenchmarks`, S9 to S11 from group 2 of `physical-ios`, S13 from task 2.1 of `mcp-server`, S14 from task 1.1 of `real-app-fixes`, and S15 from task 1.1 of `profiling`, named `s<N>-<slug>.md` after the spike's program, for example `s2-simulator-frames.md`. The report is the lasting output of a spike; design.md decisions cite it by file name.

The programs, throwaway code the crates never built on, and their raw results were in `spikes/` until the repository was made public, and are kept privately since: each report's Setup says how its program ran and at which commit, and its Measurements carry the numbers. S13's to S15's programs were written afterwards and kept privately from the start, S15's with the profiles, flamegraphs and exports it folded, which hold the host's paths. The paths and commands under `spikes/` that the reports and the archived changes name are those of the private copy.

Every report has these sections, in this order:

- **Setup**: host hardware and OS, Xcode or Android SDK and API level, device or emulator model, tool and crate versions, the commit of the spike code, and how to rerun it.
- **Measurements**: the numbers the spike was asked for, with sample counts, p50 and p95 for timings, and warm-up iterations excluded.
- **Verdict**: go or no-go, the design decision it confirms or changes, and what that means for the tasks it gates.

`docs/overview.md` cites these reports beside the end-to-end reports that followed them, to answer how mobile-agent differs from other tools and where it is faster.
