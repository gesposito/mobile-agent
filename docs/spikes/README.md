# Spike reports

One report per spike from tasks.md group 2, named `s<N>-<slug>.md` after the spike's directory under `spikes/`, for example `s2-simulator-frames.md`. The report is the lasting output of a spike; design.md decisions cite it by file name.

Every report has these sections, in this order:

- **Setup**: host hardware and OS, Xcode or Android SDK and API level, device or emulator model, tool and crate versions, the commit of the spike code, and how to rerun it.
- **Measurements**: the numbers the spike was asked for, with sample counts, p50 and p95 for timings, and warm-up iterations excluded.
- **Verdict**: go or no-go, the design decision it confirms or changes, and what that means for the tasks it gates.

`docs/overview.md` cites these reports beside the end-to-end reports that followed them, to answer how mobile-agent differs from other tools and where it is faster.
