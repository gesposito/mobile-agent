#!/usr/bin/env python3
"""Summarizes one session that drive.py recorded, for reports/10.20-resources.md.

Usage, from the repository root (Python 3 standard library only):

    python3 reports/10.20-resources/analyze.py <dir>            # each window's figures, per process
    python3 reports/10.20-resources/analyze.py --tables <dir>   # the report's tables, as markdown

<dir> is drive.py's --out: marks.jsonl, samples/ (sampler.py's output, with
guest.txt on the emulator) and run/report.json (the harness's report).

Windows:
  idle  <tool>-idle-start .. <tool>-idle-end marks (driver)
  runs  the ten measured login iterations of each tool in the harness run,
        from the harness's own command log (children.jsonl) and its report

CPU of a set of processes over a window [t0, t1]:
  - commands the harness ran inside the window: their exact CPU, self plus
    reaped descendants, from the harness's reaped-children rusage;
  - every other process of the set: its cumulative CPU (self plus reaped
    children) interpolated at max(t0, birth) and min(t1, last sample);
    a process that ended inside the window and whose parent is in the set is
    left out, since its whole CPU reached the parent's reaped-children time,
    and if it was alive at t0 the parent's jump is reduced by its CPU at t0;
    a process whose parent is launchd_sim and which ended inside the window
    takes the launchd_sim reaped-children jump of that tick when it was the
    only child to end then.
Footprint: per tick, the sum over the set's live processes; peak and mean
over the window's ticks.
"""
import bisect
import collections
import datetime
import gzip
import json
import os
import sys

NS = 1e9


def load(session):
    d = os.path.join(session, "samples")
    ticks = []
    for line in open(os.path.join(d, "ticks.csv")):
        f = line.strip().split(",")
        ticks.append((int(f[0]), float(f[1]), float(f[2]), float(f[3]), float(f[4]), int(f[5]), float(f[6])))
    tick_time = {t[0]: t[1] for t in ticks}
    # process instances
    inst = {}  # (pid, start) -> dict
    current = {}  # pid -> key
    procs_by_tick = collections.defaultdict(list)
    for line in open(os.path.join(d, "procs.jsonl")):
        r = json.loads(line)
        key = (r["pid"], r["start"])
        if r["ev"] in ("new", "exec"):
            if key not in inst:
                inst[key] = {"pid": r["pid"], "start": r["start"], "label": r["label"], "cmd": r["cmd"],
                             "comm": r["comm"], "ppid0": r["ppid"], "first": r["tick"], "gone": None,
                             "series": []}
            else:
                old = inst[key]["label"]
                inst[key]["cmd"] = r["cmd"]
                inst[key]["comm"] = r["comm"]
                if r["label"] not in ("other", "other-sim"):
                    inst[key]["label"] = r["label"]
                inst[key].setdefault("labels", []).append(old)
            procs_by_tick[r["tick"]].append(("new", r["pid"], key))
        elif r["ev"] == "gone":
            if key in inst:
                inst[key]["gone"] = r["tick"]
            procs_by_tick[r["tick"]].append(("gone", r["pid"], key))
    # samples: assign each row to the instance current for that pid at that tick
    ordered = sorted(inst.values(), key=lambda x: (x["pid"], x["first"]))
    by_pid = collections.defaultdict(list)
    for x in ordered:
        by_pid[x["pid"]].append(x)

    def instance_for(pid, tick):
        cands = by_pid.get(pid)
        if not cands:
            return None
        best = None
        for x in cands:
            if x["first"] <= tick and (x["gone"] is None or tick < x["gone"]):
                best = x
        return best

    with gzip.open(os.path.join(d, "samples.csv.gz"), "rt") as f:
        for line in f:
            p = line.split(",")
            if len(p) != 7:
                continue
            tick, pid = int(p[0]), int(p[1])
            if int(p[3]) < 0 or int(p[5]) < 0:
                continue  # rusage refused, as for a process on its way out
            x = instance_for(pid, tick)
            if x is None:
                continue
            x["series"].append((tick, int(p[2]), int(p[3]), int(p[4]), int(p[5]), int(p[6])))
    for x in inst.values():
        x["series"].sort()
        x["ticks"] = [s[0] for s in x["series"]]
    marks = [json.loads(l) for l in open(os.path.join(session, "marks.jsonl"))]
    children = []
    cpath = os.path.join(d, "children.jsonl")
    if os.path.exists(cpath):
        children = [json.loads(l) for l in open(cpath)]
    # a command caught between fork and exec reads as a bare name, such as
    # `sh`; the scan's record of the same process has its command line
    for ev in children:
        if ev.get("ev") == "start" and len(ev.get("cmd") or []) == 1 and "/" not in ev["cmd"][0]:
            for x in by_pid.get(ev["pid"], []):
                if abs(x["start"] - ev["start"]) < 0.5 and x["cmd"] and len(x["cmd"]) > 1:
                    ev["cmd"] = x["cmd"]
    report = None
    rpath = os.path.join(session, "run", "report.json")
    if os.path.exists(rpath):
        report = json.load(open(rpath))
    guest = os.path.join(d, "guest.txt")
    return {"ticks": ticks, "tick_time": tick_time, "inst": inst, "marks": marks, "children": children,
            "report": report, "guest": guest if os.path.exists(guest) else None}


def tool_of(label):
    if label.startswith("mobile-agent:"):
        return "mobile-agent"
    if label.startswith("maestro:"):
        return "maestro"
    if label.startswith("agent-device:"):
        return "agent-device"
    return None


# ---- harness command log ---------------------------------------------------------
def commands(children):
    """Every command the harness ran: start, end, argv, exact CPU when known."""
    cmds = {}
    order = []
    unattributed = []
    for ev in children:
        if ev["ev"] == "start":
            c = {"pid": ev["pid"], "start": ev["start"], "seen": ev["t"], "end": None, "cmd": ev["cmd"],
                 "cpu_ns": None}
            cmds[ev["pid"]] = c
            order.append(c)
        elif ev["ev"] in ("end", "cpu"):
            gone = ev.get("gone", [])
            if len(gone) == 1 and gone[0] in cmds:
                c = cmds[gone[0]]
                c["end"] = ev["t"]
                c["cpu_ns"] = ev["dccpu_ns"]
            else:
                for g in gone:
                    if g in cmds:
                        cmds[g]["end"] = ev["t"]
                if ev["dccpu_ns"]:
                    unattributed.append((ev["t"], ev["dccpu_ns"], gone))
    return order, unattributed


def cmd_tool(argv):
    s = " ".join(argv)
    b = os.path.basename(argv[0]) if argv else ""
    if b == "mobile-agent":
        return "mobile-agent"
    if "agent-device" in s:
        return "agent-device"
    if "maestro" in s:
        return "maestro"
    return "harness"


def is_flow_start(tool, argv):
    s = " ".join(argv)
    if tool == "mobile-agent":
        return " open --relaunch " in s and "--env" not in s
    if tool == "maestro":
        return " test " in s
    if tool == "agent-device":
        return " batch " in s
    return False


def run_windows(data):
    """The ten measured iterations of each tool, as [t0, t1] with their commands."""
    rep = data["report"]
    order, unattributed = commands(data["children"])
    out = {}
    if rep is None:
        return out, order, unattributed
    for m in rep["measurements"]:
        if m.get("flow") is None and "flow" not in json.dumps(m.get("subject", "")):
            pass
        tool = m["tool"]
        samples = m["samples"]
        attempts = [s["attempts"] for s in samples]
        n_attempts = sum(attempts)
        warm_attempts = sum(s["attempts"] for s in samples if s["warmup"])
        starts = [c for c in order if cmd_tool(c["cmd"]) == tool and is_flow_start(tool, c["cmd"])]
        if len(starts) < n_attempts:
            out[tool] = {"error": f"{len(starts)} flow starts in the log, {n_attempts} attempts in the report"}
            continue
        series = starts[-n_attempts:]
        first = series[warm_attempts]
        t0 = first["start"]
        # the window ends with the last command of the last attempt: the flow's
        # one command for Maestro and agent-device; for mobile-agent the
        # commands after its last `open --relaunch`, up to the release
        idx = order.index(series[-1])
        last = series[-1]
        if tool == "mobile-agent":
            for c in order[idx + 1:]:
                s = " ".join(c["cmd"])
                if cmd_tool(c["cmd"]) != tool or is_flow_start(tool, c["cmd"]) \
                        or " status" in s or " shutdown" in s:
                    break
                last = c
        t1 = last["end"] or last["seen"]
        measured_us = [s["us"] for s in samples if not s["warmup"] and s["us"] is not None]
        out[tool] = {"t0": t0, "t1": t1, "attempts": n_attempts - warm_attempts,
                     "sum_us": sum(measured_us), "n": len(measured_us),
                     "p50_us": sorted(measured_us)[(len(measured_us) + 1) // 2 - 1] if measured_us else None,
                     "fails": sum(1 for s in samples if not s["warmup"] and s["outcome"] == "fail"),
                     "flakes": sum(1 for s in samples if not s["warmup"] and s["outcome"] == "flake")}
    return out, order, unattributed


# ---- per-window figures -------------------------------------------------------------
def cum_at(x, t, tick_time):
    """Cumulative CPU ns (self + reaped children) of instance x at time t, interpolated."""
    ser = x["series"]
    if not ser:
        return 0
    times = [tick_time[s[0]] for s in ser]
    vals = [s[4] + s[5] for s in ser]
    if t <= times[0]:
        return vals[0] if x["start"] < times[0] and t >= x["start"] else (vals[0] if t >= times[0] else 0)
    if t >= times[-1]:
        return vals[-1]
    i = bisect.bisect_right(times, t)
    t_a, t_b = times[i - 1], times[i]
    v_a, v_b = vals[i - 1], vals[i]
    return v_a + (v_b - v_a) * (t - t_a) / (t_b - t_a)


def end_time(x, tt):
    """When instance x was last seen alive (its last sample), or None."""
    return tt[x["series"][-1][0]] if x["series"] else None


def window_cpu(data, members, t0, t1, bench_cmds, launchd_sim=None, parents=None):
    """CPU ns of the instances in `members` over [t0, t1]; see the module docstring.

    `bench_cmds` are the harness's commands inside the window, with exact CPU;
    a member that is one of them counts that figure instead of its samples.
    `parents` is the set of instances a member's reaped CPU can flow into
    (default: the members themselves)."""
    tt = data["tick_time"]
    exact = {}
    for c in bench_cmds:
        if c["cpu_ns"] is not None:
            exact[c["pid"]] = c
    by_pid = collections.defaultdict(list)
    for x in (parents if parents is not None else members):
        by_pid[x["pid"]].append(x)
    total = 0.0
    parts = collections.defaultdict(float)
    tails = []
    for x in members:
        ser = x["series"]
        c = exact.get(x["pid"])
        if c is not None and abs(c["start"] - x["start"]) < 0.5:
            total += c["cpu_ns"]
            parts[x["label"]] += c["cpu_ns"]
            continue
        if not ser:
            continue
        last_t = end_time(x, tt)
        if last_t < t0 or x["start"] > t1:
            continue
        ended_inside = x["gone"] is not None and tt.get(x["gone"], 1e18) <= t1
        if ended_inside:
            parent = None
            for p in by_pid.get(x["ppid0"], []):
                if p["start"] <= x["start"] and p is not x:
                    parent = p
            parent_cmd = exact.get(x["ppid0"])
            parent_outlived = parent is not None and (
                parent["gone"] is None or parent["gone"] > x["gone"]
                or (parent_cmd is not None and (parent_cmd["end"] or 0) >= tt.get(x["gone"], 0) - 0.6))
            if parent_outlived:
                # its whole CPU reached the parent's reaped-children time (or
                # the parent command's exact figure); what it used before t0
                # rides along in the parent's jump
                if x["start"] < t0 and parent_cmd is None:
                    total -= cum_at(x, t0, tt)
                    parts[x["label"]] -= cum_at(x, t0, tt)
                continue
        a = 0.0 if x["start"] >= t0 else cum_at(x, t0, tt)
        b = cum_at(x, min(t1, last_t), tt)
        if ended_inside and launchd_sim is not None and x["ppid0"] == launchd_sim["pid"]:
            whole = sole_exit_jump(data, launchd_sim, x)
            if whole is not None and whole >= b:
                tails.append((x["label"], whole - b))
                b = whole
        total += b - a
        parts[x["label"]] += b - a
    return total, dict(parts), tails


def decompose(data, members, t0, t1, bench_cmds, launchd_sim=None):
    """Splits a tool's CPU over [t0, t1] by role, exactly where the record allows.

    A child its parent reaped inside the window takes the jump of the parent's
    reaped-children time at the tick it went (shared by siblings that went at
    the same tick, in proportion to their last sampled CPU); jumps no sampled
    child claims go to '<parent role> (children not sampled)'. A harness
    command takes its exact CPU less the jumps its children took. Every other
    process takes its own CPU from the samples."""
    per, unclaimed = decompose_per_instance(data, members, t0, t1, bench_cmds, launchd_sim, with_unclaimed=True)
    rows = collections.defaultdict(float)
    for x in members:
        if id(x) in per:
            rows[x["label"]] += per[id(x)]
    for label, v in unclaimed.items():
        rows[label + " (children not sampled)"] += v
    return dict(rows)


def sole_exit_jump(data, parent, x):
    """The parent's reaped-children jump at the tick x went, if x was the only
    child of the parent to go then; else None."""
    g = x["gone"]
    if g is None or len(x["series"]) < 2:
        return None
    others = [y for y in data["inst"].values()
              if y is not x and y["ppid0"] == parent["pid"] and y["gone"] in (g - 1, g, g + 1)]
    if others:
        return None
    ps = parent["series"]
    ticks = [s[0] for s in ps]
    i = bisect.bisect_left(ticks, g)
    if i == 0 or i >= len(ps):
        return None
    whole = ps[i][5] - ps[i - 1][5]
    # the tail since the last sample cannot exceed a second at the process's
    # own peak rate; a larger jump carries some other child's exit
    tt = data["tick_time"]
    ser = x["series"]
    rate = max(((b[4] + b[5]) - (a[4] + a[5])) / max(tt[b[0]] - tt[a[0]], 1e-3) for a, b in zip(ser, ser[1:]))
    last = ser[-1][4] + ser[-1][5]
    if whole - last > rate * 1.0:
        return None
    return whole


def window_mem(data, members, t0, t1):
    """Per-tick summed footprint over [t0, t1]: (peak B, mean B, ticks)."""
    tt = data["tick_time"]
    win = [t for t, ts in tt.items() if t0 <= ts <= t1]
    if not win:
        return None
    lo, hi = min(win), max(win)
    sums = collections.Counter()
    for x in members:
        for s in x["series"]:
            if lo <= s[0] <= hi and s[2] >= 0:
                sums[s[0]] += s[2]
    vals = [sums.get(t, 0) for t in range(lo, hi + 1)]
    return max(vals), sum(vals) / len(vals), len(vals)


def load_at(data, t):
    best = min(data["ticks"], key=lambda r: abs(r[1] - t))
    return best[2]


def load_max(data, t0, t1):
    vals = [r[2] for r in data["ticks"] if t0 <= r[1] <= t1]
    return max(vals) if vals else None


ROLE_NAMES = {
    "mobile-agent:daemon": "daemon (`mobile-agent-daemon`)",
    "mobile-agent:reader": "reader (`simulator-reader`, in the Simulator)",
    "mobile-agent:cli": "CLI processes (`mobile-agent`, one per command)",
    "mobile-agent:child": "daemon's children",
    "maestro:jvm": "JVM (`maestro`, one per flow)",
    "maestro:launcher": "launcher script",
    "maestro:driver-xcodebuild": "driver's `xcodebuild`",
    "maestro:driver-app": "driver app (`maestro-driver-iosUITests-Runner`, in the Simulator)",
    "maestro:child": "JVM's other children",
    "agent-device:client": "client (`agent-device`, one node process per command)",
    "agent-device:daemon": "daemon (node)",
    "agent-device:runner-xcodebuild": "runner's `xcodebuild`",
    "agent-device:runner-app": "runner app (`AgentDeviceRunnerUITests-Runner`, in the Simulator)",
    "agent-device:bridge-simctl": "bridge's `simctl spawn`",
    "agent-device:bridge": "bridge (`snapshot-bridge`, in the Simulator)",
    "agent-device:child": "daemon's other children",
}


def summarize(session):
    data = load(session)
    inst = list(data["inst"].values())
    runs, order, unattributed = run_windows(data)
    marks = {m["ev"]: m for m in data["marks"]}
    launchd = [x for x in inst if x["label"] == "device:sim"]
    launchd_sim = launchd[0] if launchd else None
    windows = []
    for tool in ("mobile-agent", "maestro", "agent-device"):
        s, e = marks.get(f"{tool}-idle-start"), marks.get(f"{tool}-idle-end")
        if s and e:
            windows.append((tool, "idle", s["t"], e["t"], None))
        if tool in runs and "t0" in runs[tool]:
            r = runs[tool]
            windows.append((tool, "runs", r["t0"], r["t1"], r))
    result = []
    guest = load_guest(data["guest"]) if data["guest"] else None
    data["guest_data"] = guest
    for tool, kind, t0, t1, r in windows:
        members = [x for x in inst if tool_of(x["label"]) == tool]
        bench_cmds = [c for c in order if c["start"] >= t0 - 0.001 and (c["end"] or c["seen"]) <= t1 + 0.05]
        total, parts, tails = window_cpu(data, members, t0, t1, bench_cmds, launchd_sim)
        # CPU of harness children too short for the poll to see: during a
        # tool's measured iterations every child of the harness is that tool's
        unseen = sum(u[1] for u in unattributed if t0 <= u[0] <= t1 + 0.05) if kind == "runs" else 0
        total += unseen
        roles = collections.defaultdict(list)
        for x in members:
            roles[x["label"]].append(x)
        role_rows = []
        split = decompose(data, members, t0, t1, bench_cmds, launchd_sim)
        if unseen:
            split[tool + ": harness children not seen by the poll"] = unseen
        for label in split:
            roles.setdefault(label, [])
        for label, xs in sorted(roles.items()):
            mem = window_mem(data, xs, t0, t1) if xs else None
            cpu = split.get(label, 0.0)
            subs = sub_roles(data, members, t0, t1, bench_cmds, launchd_sim, label) if label.endswith(":child") else []
            role_rows.append({"label": label, "mem": mem, "cpu_ns": cpu, "subs": subs,
                              "pids": sorted({x["pid"] for x in xs
                                              if x["series"] and data["tick_time"][x["series"][-1][0]] >= t0 and x["start"] <= t1})})
        cmd_cpu = sum(c["cpu_ns"] or 0 for c in bench_cmds if cmd_tool(c["cmd"]) == tool)
        cmd_missing = sum(1 for c in bench_cmds if c["cpu_ns"] is None and cmd_tool(c["cmd"]) == tool)
        other_cmds = [c for c in bench_cmds if cmd_tool(c["cmd"]) != tool]
        unattr = sum(u[1] for u in unattributed if t0 <= u[0] <= t1 + 0.05)
        mem_total = window_mem(data, members, t0, t1)
        # context: device and harness
        ctx = {}
        # each Simulator's rendering services (one SimRenderServer, three
        # SimMetalHost) are children of launchd, started with its launchd_sim
        render_start = launchd_sim["start"] if launchd_sim else None
        for name, pred in (
            ("render", lambda x: render_start is not None and x["cmd"] and os.path.basename(x["cmd"][0]) in (
                "SimRenderServer", "SimMetalHost") and abs(x["start"] - render_start) < 3),
            ("app", lambda x: x["label"] == "device:sim/app"),
            ("sim-other", lambda x: x["label"] in ("device:sim", "device:sim/system", "device:sim/testmanagerd")),
            ("testmanagerd", lambda x: x["label"] == "device:sim/testmanagerd"),
            ("emulator", lambda x: x["label"].startswith("device:emulator")),
            ("adb-server", lambda x: x["label"] == "shared:adb-server"),
            ("coresimulator", lambda x: x["label"] == "shared:coresimulator"),
            ("harness", lambda x: x["label"] == "harness"),
            ("sampler", lambda x: x["label"] == "sampler"),
        ):
            xs = [x for x in inst if pred(x)]
            if not xs:
                continue
            if name == "harness":
                # the harness's own CPU, without its children's
                c = 0.0
                for x in xs:
                    ser = x["series"]
                    if not ser:
                        continue
                    c += own_cpu(x, t0, t1, data["tick_time"])
                ctx[name] = {"cpu_ns": c, "mem": window_mem(data, xs, t0, t1)}
                continue
            c, _, _ = window_cpu(data, xs, t0, t1, [], launchd_sim)
            ctx[name] = {"cpu_ns": c, "mem": window_mem(data, xs, t0, t1)}
        result.append({
            "tool": tool, "kind": kind, "t0": t0, "t1": t1, "secs": t1 - t0, "run": r,
            "cpu_ns": total, "parts": parts, "tails": tails, "roles": role_rows,
            "cmd_cpu_ns": cmd_cpu, "cmd_missing": cmd_missing, "unattributed_ns": unattr,
            "other_cmds": [" ".join(c["cmd"])[:120] for c in other_cmds],
            "mem": mem_total, "load_start": load_at(data, t0), "load_end": load_at(data, t1),
            "load_max": load_max(data, t0, t1), "ctx": ctx,
            "guest": guest_window(guest, t0, t1) if guest else None,
        })
    return data, runs, order, result


def own_cpu(x, t0, t1, tt):
    ser = x["series"]
    times = [tt[s[0]] for s in ser]
    vals = [s[4] for s in ser]

    def at(t):
        if t <= times[0]:
            return vals[0] if t >= x["start"] else 0
        if t >= times[-1]:
            return vals[-1]
        i = bisect.bisect_right(times, t)
        return vals[i - 1] + (vals[i] - vals[i - 1]) * (t - times[i - 1]) / (times[i] - times[i - 1])

    return at(min(t1, times[-1])) - (0 if x["start"] >= t0 else at(t0))


USER_HZ = 100


def guest_label(name, args):
    s = name + " " + args
    if "dev.mobileagent.agent.Main" in s or name.startswith("mobile-agent") or " mobile-agent-" in s:
        return "mobile-agent:agent"
    if name == "dev.mobile.maestro":
        return "maestro:driver"
    if "instrument" in s and "maestro" in s:
        return "maestro:am-instrument"
    if "maestro" in s:
        return "maestro:shell-command"
    if name == "com.callstack.agentdevice.snapshothelper":
        return "agent-device:snapshot-helper"
    if name == "com.callstack.agentdevice.imehelper":
        return "agent-device:ime-helper"
    if "instrument" in s and "agentdevice" in s:
        return "agent-device:am-instrument"
    if "agentdevice" in s:
        return "agent-device:shell-command"
    if name.startswith("dev.mobileagent.fixture"):
        return "app"
    return "shell-command"


def parse_time_plus(tm):
    """toybox TIME+, [[dd-]hh:]mm:ss.hh, in seconds."""
    days = 0
    if "-" in tm:
        d, tm = tm.split("-", 1)
        days = int(d)
    parts = [float(x) for x in tm.split(":")]
    secs = 0.0
    for x in parts:
        secs = secs * 60 + x
    return days * 86400 + secs


def guest_top(g, t0, t1):
    """CPU-s of every guest process over [t0, t1] from the all-process
    snapshots, as [(cpu, name, pid)] sorted by CPU. Processes are keyed by
    pid, since kernel workers rename themselves as they take work; a pid whose
    CPU falls back is a new process."""
    by_pid = collections.defaultdict(list)
    for (pid, name), pts in g["allp"].items():
        for t, v in pts:
            by_pid[pid].append((t, v, name))
    snaps = sorted({t for pts in g["allp"].values() for t, _ in pts})
    before = [t for t in snaps if t <= t0]
    t_before = before[-1] if before else None
    out = []
    for pid, recs in by_pid.items():
        recs.sort()
        runs, cur = [], [recs[0]]
        for r in recs[1:]:
            if r[1] < cur[-1][1]:
                runs.append(cur)
                cur = [r]
            else:
                cur.append(r)
        runs.append(cur)
        for run in runs:
            if run[-1][0] < t0 or run[0][0] > t1:
                continue
            pts = [(t, v) for t, v, _ in run]
            present_before = t_before is not None and any(abs(t - t_before) < 0.01 for t, _ in pts)
            a = _interp(pts, t0) if present_before else 0.0
            b = _interp(pts, min(t1, pts[-1][0]))
            name = run[-1][2]
            if name.startswith("[kworker/"):
                name = name.split("-")[0].rstrip("]") + "]"
            if b - a > 0:
                out.append((b - a, name, pid))
    out.sort(reverse=True)
    return out


def load_guest(path):
    """Guest records: instances keyed by (pid, starttime) with their series,
    the guest-wide CPU line, the guest loop's own CPU, every process's CPU
    from the all-process snapshots, and the loop's start and end events."""
    names = {}
    inst = {}
    totals = []
    loop = []
    ticks = {}
    allp = {}
    events = []
    for line in open(path):
        p = line.split()
        if len(p) < 3:
            continue
        try:
            t = float(p[0])
        except ValueError:
            continue
        kind = p[1]
        try:
            if kind == "T":
                n = int(p[2])
                up = float(p[3])
                user, nice, system, idle, iow, irq, sirq, steal = (int(v) for v in p[4:12])
                totals.append((t, user + nice + system + irq + sirq, up, n))
                ticks[n] = t
            elif kind == "P":
                pid = int(p[3])
                names.setdefault(pid, []).append((p[5], p[6], " ".join(p[7:])))
            elif kind == "S":
                n, pid = int(p[2]), int(p[3])
                ut, st, start, rss = (int(v) for v in p[4:8])
                pss = int(p[8]) if len(p) > 8 and p[8].isdigit() else None
                key = (pid, start)
                x = inst.get(key)
                if x is None:
                    x = inst[key] = {"pid": pid, "start_ticks": start, "series": []}
                x["series"].append((t, ut + st, rss, pss, n))
            elif kind == "M":
                loop.append((t, sum(int(v) for v in p[3:7])))
            elif kind == "A" and len(p) >= 8:
                pid = int(p[3])
                allp.setdefault((pid, " ".join(p[7:])), []).append((t, parse_time_plus(p[6])))
            elif kind == "X":
                events.append((t, p[2]))
        except (ValueError, IndexError):
            continue
    for x in inst.values():
        recs = names.get(x["pid"], [])
        label = "shell-command"
        for user, name, args in recs:
            lab = guest_label(name, args)
            if lab != "shell-command" or label == "shell-command":
                label = lab
        x["label"] = label
        x["names"] = sorted({(u, nm, a[:100]) for u, nm, a in recs})
    return {"inst": inst, "totals": totals, "loop": loop, "ticks": ticks, "allp": allp, "events": events}


def _interp(points, t):
    """Linear interpolation over (time, value) points, clamped at the ends."""
    if not points:
        return 0.0
    if t <= points[0][0]:
        return points[0][1]
    if t >= points[-1][0]:
        return points[-1][1]
    times = [q[0] for q in points]
    i = bisect.bisect_right(times, t)
    (ta, va), (tb, vb) = points[i - 1], points[i]
    return va + (vb - va) * (t - ta) / (tb - ta)


def guest_window(g, t0, t1):
    """Per guest role over [t0, t1]: CPU-s, PSS peak and mean (kB), instances;
    plus the guest-wide busy CPU-s and the guest loop's own."""
    tot = [(t, b) for t, b, up, n in g["totals"]]
    up_pts = [(t, up) for t, b, up, n in g["totals"]]
    busy = (_interp(tot, t1) - _interp(tot, t0)) / USER_HZ
    loop = (_interp(g["loop"], t1) - _interp(g["loop"], t0)) / USER_HZ
    up0 = _interp(up_pts, t0)
    win_ticks = sorted(n for n, t in g["ticks"].items() if t0 <= t <= t1)
    roles = collections.defaultdict(lambda: {"cpu": 0.0, "pss": collections.Counter(), "n": 0})
    for x in g["inst"].values():
        ser = x["series"]
        if not ser or ser[-1][0] < t0 or ser[0][0] > t1:
            continue
        pts = [(s[0], s[1]) for s in ser]
        started_inside = x["start_ticks"] / USER_HZ > up0
        a = 0.0 if started_inside else _interp(pts, t0)
        b = _interp(pts, min(t1, ser[-1][0]))
        r = roles[x["label"]]
        r["cpu"] += (b - a) / USER_HZ
        r["n"] += 1
        for s in ser:
            if win_ticks and win_ticks[0] <= s[4] <= win_ticks[-1] and s[3] is not None:
                r["pss"][s[4]] += s[3]
    out = {}
    for lab, r in roles.items():
        vals = [r["pss"].get(n, 0) for n in win_ticks] if win_ticks else []
        out[lab] = {"cpu": r["cpu"], "n": r["n"],
                    "pss_peak": max(vals) if vals else 0, "pss_mean": sum(vals) / len(vals) if vals else 0}
    return out, busy, loop


def child_kind(x):
    """A short name for a helper process, from its command."""
    cmd = x["cmd"] or [x["comm"]]
    s = " ".join(cmd)
    base = os.path.basename(cmd[0])
    if base == "java" and "sdkmanager" in s:
        return "avdmanager (Android SDK, a JVM)"
    if base in ("sh", "bash", "zsh") and len(cmd) > 1:
        base = os.path.basename(cmd[1])
        if base == "avdmanager":
            return "avdmanager (Android SDK, a JVM)"
    if base == "simctl" or (base == "xcrun" and "simctl" in s):
        verb = next((a for a in cmd[1:] if not a.startswith("-") and a != "simctl"), "")
        if verb == "spawn" and len(cmd) > 3:
            return "simctl spawn " + os.path.basename(cmd[3])
        return "simctl " + verb
    if base == "xcodebuild":
        return "xcodebuild " + (cmd[1] if len(cmd) > 1 else "")
    return base


def sub_roles(data, members, t0, t1, bench_cmds, launchd_sim, label):
    """CPU and footprint of a role's processes grouped by command kind."""
    xs = [x for x in members if x["label"] == label]
    groups = collections.defaultdict(list)
    for x in xs:
        groups[child_kind(x)].append(x)
    full = decompose_per_instance(data, members, t0, t1, bench_cmds, launchd_sim)
    out = []
    for kind, g in sorted(groups.items()):
        cpu = sum(full.get(id(x), 0.0) for x in g)
        n = sum(1 for x in g if x["series"] and end_time(x, data["tick_time"]) >= t0 and x["start"] <= t1)
        if n == 0 and cpu == 0:
            continue
        out.append((kind, n, cpu, window_mem(data, g, t0, t1)))
    return out


def decompose_per_instance(data, members, t0, t1, bench_cmds, launchd_sim=None, with_unclaimed=False):
    """CPU ns per instance (keyed by id) over [t0, t1]; see `decompose`.

    A child that ended inside the window while its parent lived takes only a
    jump of the parent's reaped-children time (at the tick it went, else the
    next one inside the window), never its own samples, so that nothing is
    counted twice; a jump no such child claims stays with the parent's role
    as children not sampled."""
    tt = data["tick_time"]
    in_win = lambda tick: tick is not None and t0 < tt.get(tick, -1) <= t1
    exact = {c["pid"]: c for c in bench_cmds if c["cpu_ns"] is not None}
    per = collections.defaultdict(float)
    final = {}
    reaped = set()
    claimed = collections.defaultdict(float)
    unclaimed = collections.defaultdict(float)
    by_pid = collections.defaultdict(list)
    for p in members:
        by_pid[p["pid"]].append(p)
    for p in members:
        ser = p["series"]
        if len(ser) < 2:
            continue
        kids = [x for x in members if x["ppid0"] == p["pid"] and x is not p and x["start"] >= p["start"]
                and (p["gone"] is None or x["start"] <= tt.get(p["gone"], 1e18))]
        p_end = tt.get(p["gone"], 1e18) if p["gone"] is not None else 1e18
        for x in kids:
            if in_win(x["gone"]) and tt[x["gone"]] <= p_end:
                reaped.add(id(x))
        for i in range(1, len(ser)):
            tick, jump = ser[i][0], ser[i][5] - ser[i - 1][5]
            if jump <= 0:
                continue
            ta, tb = tt[ser[i - 1][0]], tt[tick]
            overlap = max(0.0, min(t1, tb) - max(t0, ta)) / max(tb - ta, 1e-6)
            if overlap <= 0:
                continue
            gone_here = [] if not in_win(tick) else (
                [x for x in kids if id(x) in reaped and x["gone"] == tick and id(x) not in final] or
                [x for x in kids if id(x) in reaped and x["gone"] == tick + 1 and id(x) not in final])
            if gone_here:
                weights = [max(cum_at(x, end_time(x, tt), tt), 1.0) if x["series"] else 1.0 for x in gone_here]
                for x, w in zip(gone_here, weights):
                    final[id(x)] = jump * w / sum(weights)
                claimed[id(p)] += jump
            else:
                # a child no sample saw: its CPU is spread over the interval,
                # as the interpolation of the totals spreads it
                unclaimed[p["label"]] += jump * overlap
                claimed[id(p)] += jump * overlap
    for x in members:
        c = exact.get(x["pid"])
        if c is not None and abs(c["start"] - x["start"]) < 0.5:
            per[id(x)] += c["cpu_ns"] - claimed.get(id(x), 0.0)
            continue
        if not x["series"]:
            # seen only while it went: its whole CPU is the jump it claimed
            per[id(x)] += final.get(id(x), 0.0)
            continue
        if x["start"] > t1 or (x["gone"] is not None and tt.get(x["gone"], 1e18) <= t0):
            continue
        a_cum = cum_at(x, t0, tt) if x["start"] < t0 else 0.0
        if id(x) in reaped:
            if id(x) in final:
                per[id(x)] += final[id(x)] - a_cum
            elif a_cum:
                # its parent's unclaimed jump carries what it used before t0
                parent = next((p for p in by_pid.get(x["ppid0"], []) if p["start"] <= x["start"]), None)
                if parent is not None:
                    unclaimed[parent["label"]] -= a_cum
            continue
        ended_inside = in_win(x["gone"])
        if ended_inside and launchd_sim is not None and x["ppid0"] == launchd_sim["pid"]:
            whole = sole_exit_jump(data, launchd_sim, x)
            if whole is not None and whole >= cum_at(x, end_time(x, tt), tt):
                per[id(x)] += whole - a_cum
                continue
        per[id(x)] += own_cpu(x, t0, t1, tt)
    if with_unclaimed:
        return per, unclaimed
    return per


def fmt_mb(b):
    return "%.1f" % (b / 2 ** 20)


def print_summary(session):
    """Each window's figures, per process and per role."""
    data, runs, order, result = summarize(session)
    print("runs:", json.dumps({k: {kk: vv for kk, vv in v.items()} for k, v in runs.items()}, indent=1))
    for w in result:
        per = w["secs"] / 60 if w["kind"] == "idle" else (w["run"]["n"] if w["run"] else 1)
        unit = "per minute" if w["kind"] == "idle" else "per flow"
        print(f"\n== {w['tool']} {w['kind']} {w['secs']:.1f}s load {w['load_start']}->{w['load_end']} (max {w['load_max']})")
        if w["run"]:
            print(f"   report: n={w['run']['n']} sum={w['run']['sum_us']/1e6:.2f}s p50={w['run']['p50_us']/1e6:.3f}s fails={w['run']['fails']} flakes={w['run']['flakes']} attempts={w['run']['attempts']}")
        print(f"   CPU total {w['cpu_ns']/NS:.3f}s = {w['cpu_ns']/NS/per:.3f}s {unit}; commands exact {w['cmd_cpu_ns']/NS:.3f}s (missing {w['cmd_missing']}), unattributed {w['unattributed_ns']/NS:.3f}s")
        if w["mem"]:
            print(f"   footprint total peak {fmt_mb(w['mem'][0])} MB mean {fmt_mb(w['mem'][1])} MB over {w['mem'][2]} ticks")
        for r in w["roles"]:
            m = r["mem"]
            print(f"   - {r['label']:32s} pids={len(r['pids']):3d} peak={fmt_mb(m[0]) if m else '-':>7s} mean={fmt_mb(m[1]) if m else '-':>7s} cpu={r['cpu_ns']/NS:8.3f}s ({r['cpu_ns']/NS/per:.3f} {unit})")
            for kind, n, cpu, mm in r.get("subs", []):
                print(f"       . {kind:40s} n={n:3d} peak={fmt_mb(mm[0]) if mm else '-':>7s} cpu={cpu/NS:8.3f}s ({cpu/NS/per:.3f} {unit})")
        resid = w["cpu_ns"] - sum(r["cpu_ns"] for r in w["roles"])
        print(f"   = not in the samples (short-lived, last half-second): {resid/NS:.3f}s ({resid/NS/per:.3f} {unit})")
        if w["tails"]:
            print(f"   tails recovered: {[(l, round(v/NS, 3)) for l, v in w['tails']]}")
        if w["other_cmds"]:
            print(f"   other commands in window: {w['other_cmds'][:5]}")
        for k, v in w["ctx"].items():
            m = v["mem"]
            print(f"   ctx {k:14s} cpu={v['cpu_ns']/NS:8.3f}s ({v['cpu_ns']/NS/per:.3f} {unit}) peak={fmt_mb(m[0]) if m else '-'} mean={fmt_mb(m[1]) if m else '-'}")
        if w["guest"]:
            roles, busy, loop = w["guest"]
            print(f"   guest busy {busy:.2f}s ({busy/per:.3f} {unit}), guest loop {loop:.2f}s ({loop/per:.3f} {unit})")
            for lab, r in sorted(roles.items()):
                print(f"   guest {lab:30s} n={r['n']:3d} cpu={r['cpu']:7.2f}s ({r['cpu']/per:.3f} {unit}) pss peak={r['pss_peak']/1024:.1f} mean={r['pss_mean']/1024:.1f} MB")


# ---- the report's tables -------------------------------------------------------------
MB = 2 ** 20
TOOLS = ("mobile-agent", "maestro", "agent-device")
TOOL_NAME = {"mobile-agent": "mobile-agent", "maestro": "Maestro", "agent-device": "agent-device"}

# Each tool's rows on the Simulator: (row key, name); helpers (":child") are
# grouped by command into child:* keys, see child_bucket.
ORDER_SIM = {
    "mobile-agent": [
        ("mobile-agent:daemon", "daemon, `mobile-agent-daemon`"),
        ("mobile-agent:reader", "reader, `simulator-reader`, in the Simulator"),
        ("mobile-agent:cli", "CLI, `mobile-agent`, one process per command"),
        ("mobile-agent:child", "daemon's helpers"),
        ("mobile-agent:daemon (children not sampled)", "daemon's helpers too short for a sample"),
    ],
    "maestro": [
        ("maestro:jvm", "JVM, `maestro test`, one per flow"),
        ("child:avdmanager", "Android SDK `avdmanager`, a JVM"),
        ("child:simctl", "`simctl` (list, launch, privacy, spawn, uninstall, terminate)"),
        ("child:other", "other helpers (`devicectl`, `xcodebuild -version`, `applesimutils`, shell tools)"),
        ("child:DTServiceHub", "Xcode's `DTServiceHub`, started by the driver's `xcodebuild`"),
        ("maestro:jvm (children not sampled)", "JVM's helpers too short for a sample"),
        ("maestro:driver-xcodebuild", "driver's `xcodebuild test-without-building`"),
        ("maestro:driver-app", "driver app, `maestro-driver-iosUITests-Runner`, in the Simulator"),
    ],
    "agent-device": [
        ("agent-device:daemon", "daemon, node"),
        ("agent-device:client", "client, `agent-device`, one node process per command"),
        ("agent-device:runner-xcodebuild", "runner's `xcodebuild test-without-building`"),
        ("agent-device:runner-app", "runner app, `AgentDeviceRunnerUITests-Runner`, in the Simulator"),
        ("agent-device:bridge-simctl", "bridge's `simctl spawn`"),
        ("agent-device:bridge", "bridge, `snapshot-bridge`, in the Simulator"),
        ("child:simctl", "daemon's `simctl` (launch, `spawn launchctl list`, list)"),
        ("child:other", "daemon's other helpers (`pgrep`, Xcode's `DTServiceHub`)"),
        ("agent-device:daemon (children not sampled)", "daemon's helpers too short for a sample"),
    ],
}

ORDER_EMU = {
    "mobile-agent": [
        ("mobile-agent:daemon", "daemon, `mobile-agent-daemon`"),
        ("mobile-agent:cli", "CLI, `mobile-agent`, one process per command"),
        ("mobile-agent:daemon (children not sampled)", "daemon's helpers too short for a sample"),
    ],
    "maestro": [
        ("maestro:jvm", "JVM, `maestro test`, one per flow"),
        ("child:avdmanager", "Android SDK `avdmanager`, a JVM"),
        ("child:simctl", "`xcrun simctl list`"),
        ("child:adb", "`adb`"),
        ("child:other", "other helpers (`devicectl`, `emulator`, `xcodebuild -version`, shell tools)"),
        ("maestro:jvm (children not sampled)", "JVM's helpers too short for a sample"),
    ],
    "agent-device": [
        ("agent-device:daemon", "daemon, node"),
        ("agent-device:client", "client, `agent-device`, one node process per command"),
        ("child:adb", "daemon's `adb` commands"),
        ("child:other", "daemon's other helpers"),
        ("agent-device:daemon (children not sampled)", "daemon's helpers too short for a sample"),
    ],
}

GUEST_ORDER = {
    "mobile-agent": [("mobile-agent:agent", "agent, `app_process` as the shell user, named `mobile-agent-0.4.1-…`")],
    "maestro": [("maestro:driver", "driver, the instrumented `dev.mobile.maestro` process"),
                ("maestro:am-instrument", "`am instrument -w` that runs it, an `app_process` as the shell user")],
    "agent-device": [("agent-device:snapshot-helper", "snapshot helper, the instrumented `com.callstack.agentdevice.snapshothelper`"),
                     ("agent-device:am-instrument", "`am instrument -w` that runs it, an `app_process` as the shell user"),
                     ("agent-device:ime-helper", "input-method helper, `com.callstack.agentdevice.imehelper`")],
}

GUEST_SHORT = {
    "android.hardware.graphics.composer3-service.ranchu": "composer HAL",
    "com.google.android.inputmethod.latin": "Gboard",
    "com.google.android.apps.wellbeing": "Digital Wellbeing",
    "com.google.android.apps.nexuslauncher": "launcher",
    "com.android.systemui": "System UI",
    "android.hardware.sensors-service.multihal": "sensors HAL",
}


def utc(t):
    return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).strftime("%H:%M:%S")


def mb(x):
    return "%.1f" % (x / MB)


def mb_short(x):
    """MB with one decimal under 100, else whole and with thousands separated."""
    v = x / MB
    return "%.1f" % v if v < 100 else "{:,.0f}".format(v)


def child_bucket(kind):
    if kind == "adb":
        return "child:adb"
    if kind.startswith("avdmanager"):
        return "child:avdmanager"
    if kind.startswith("simctl"):
        return "child:simctl"
    if kind == "DTServiceHub":
        return "child:DTServiceHub"
    return "child:other"


def per_unit(w):
    """The window's divisor: minutes for an idle window, flows for the runs."""
    return (w["secs"] / 60) if w["kind"] == "idle" else w["run"]["n"]


def ordered(results):
    """Idle windows first, then the runs, each in the harness's tool order."""
    return sorted(results, key=lambda w: (w["kind"] != "idle", TOOLS.index(w["tool"])))


def rows_for(data, w, members, bench_cmds, launchd):
    """{row key: [cpu ns, [instances]]} for a window."""
    per, unclaimed = decompose_per_instance(data, members, w["t0"], w["t1"], bench_cmds, launchd, with_unclaimed=True)
    rows = collections.defaultdict(lambda: [0.0, []])
    for x in members:
        lab = x["label"]
        key = child_bucket(child_kind(x)) if lab.endswith(":child") else lab
        rows[key][0] += per.get(id(x), 0.0)
        rows[key][1].append(x)
    for lab, v in unclaimed.items():
        rows[lab + " (children not sampled)"][0] += v
    return rows


def windows_table(results):
    lines = ["| Window | Start to end (UTC) | Length | 1-minute load: start, end, highest | Flow p50 (harness) |",
             "|---|---|---|---|---|"]
    for w in sorted(results, key=lambda w: w["t0"]):
        name = f"{TOOL_NAME[w['tool']]}, {'idle' if w['kind'] == 'idle' else 'ten login runs'}"
        p50 = "%.2f s" % (w["run"]["p50_us"] / 1e6) if w["run"] else ""
        lines.append(f"| {name} | {utc(w['t0'])} to {utc(w['t1'])} | {w['secs']:.1f} s | "
                     f"{w['load_start']:.2f}, {w['load_end']:.2f}, {w['load_max']:.2f} | {p50} |")
    return "\n".join(lines)


def tool_table(data, results, tool, order_map):
    """A tool's host processes: footprint and CPU per row, and the total; also
    what the rows leave unexplained, in CPU-s per window."""
    by_kind = {w["kind"]: w for w in results if w["tool"] == tool}
    inst = list(data["inst"].values())
    launchd = next((x for x in inst if x["label"] == "device:sim"), None)
    members = [x for x in inst if tool_of(x["label"]) == tool]
    order = commands(data["children"])[0]
    cols = {}
    for kind, w in by_kind.items():
        bench_cmds = [c for c in order if c["start"] >= w["t0"] - 0.001 and (c["end"] or c["seen"]) <= w["t1"] + 0.05]
        cols[kind] = (w, rows_for(data, w, members, bench_cmds, launchd))
    lines = ["| Process | Idle: peak / mean MB | Idle: CPU-s per minute | Ten runs: peak / mean MB | Ten runs: CPU-s per flow |",
             "|---|---|---|---|---|"]
    for key, name in order_map[tool]:
        cells = []
        present = False
        for kind in ("idle", "runs"):
            if kind not in cols:
                cells += ["", ""]
                continue
            w, rows = cols[kind]
            cpu, xs = rows.get(key, (0.0, []))
            mem = window_mem(data, xs, w["t0"], w["t1"]) if xs else None
            alive = mem is not None and mem[0] > 0
            shown = alive or abs(cpu) >= 0.0005 * NS
            present = present or shown
            cells.append(f"{mb(mem[0])} / {mb(mem[1])}" if alive else "none")
            cells.append("%.3f" % (cpu / NS / per_unit(w)) if shown else "0")
        if present:
            lines.append(f"| {name} | " + " | ".join(cells) + " |")
    tot = []
    for kind in ("idle", "runs"):
        if kind not in cols:
            tot += ["", ""]
            continue
        w = cols[kind][0]
        tot.append(f"{mb(w['mem'][0])} / {mb(w['mem'][1])}" if w["mem"] and w["mem"][0] > 0 else "none")
        tot.append("%.3f" % (w["cpu_ns"] / NS / per_unit(w)))
    lines.append("| **all of the tool's processes** | " + " | ".join(f"**{c}**" for c in tot) + " |")
    resid = {kind: (w["cpu_ns"] - sum(v[0] for v in rows.values())) / NS for kind, (w, rows) in cols.items()}
    return "\n".join(lines), resid


def guest_table(results, tool):
    by_kind = {w["kind"]: w for w in results if w["tool"] == tool}
    lines = ["| Guest process | Idle: peak / mean PSS MB | Idle: CPU-s per minute | Ten runs: peak / mean PSS MB | Ten runs: CPU-s per flow |",
             "|---|---|---|---|---|"]
    for key, name in GUEST_ORDER[tool]:
        cells = []
        present = False
        for kind in ("idle", "runs"):
            w = by_kind.get(kind)
            r = (w["guest"][0] if w and w["guest"] else {}).get(key)
            if r and (r["pss_peak"] > 0 or r["cpu"] > 0.0005):
                present = True
                cells.append("%.1f / %.1f" % (r["pss_peak"] / 1024, r["pss_mean"] / 1024))
                cells.append("%.3f" % (r["cpu"] / per_unit(w)))
            else:
                cells += ["none", "0"]
        if present:
            lines.append(f"| {name} | " + " | ".join(cells) + " |")
    return "\n".join(lines)


def sim_context_table(results):
    """The Simulator's processes that belong to no tool, and the instruments."""
    lines = ["| Window | Fixture app | The Simulator's other processes | Of them, `testmanagerd` | Rendering services | Harness | Sampler |",
             "|---|---|---|---|---|---|---|"]
    for w in ordered(results):
        per = per_unit(w)
        cells = []
        for key in ("app", "sim-other", "testmanagerd", "render", "harness", "sampler"):
            v = w["ctx"].get(key)
            if v is None or (v["cpu_ns"] == 0 and not (v["mem"] and v["mem"][1] > 0)):
                cells.append("")
            elif key == "testmanagerd" or not v["mem"]:
                cells.append("%.2f" % (v["cpu_ns"] / NS / per))
            else:
                cells.append("%.2f, %s MB" % (v["cpu_ns"] / NS / per, mb_short(v["mem"][1])))
        label = f"{TOOL_NAME[w['tool']]}, {'idle' if w['kind'] == 'idle' else 'ten runs'}"
        lines.append(f"| {label} | " + " | ".join(cells) + " |")
    return "\n".join(lines)


def guest_context_table(results, data):
    """The emulator's guest beyond the tools' processes, and the host's
    emulator process and adb server."""
    g = data["guest_data"]
    lines = ["| Window | Guest busy | Fixture app (guest) | Busiest other guest processes | Emulator process on the host | adb server |",
             "|---|---|---|---|---|---|"]
    skip = ("sh", "ps", "sed", "grep", "sleep", "su", "app_process")
    for w in ordered(results):
        per = per_unit(w)
        roles, busy, loop = w["guest"]
        app = roles.get("app")
        agg = collections.Counter()
        for c, n, _ in guest_top(g, w["t0"], w["t1"]):
            if n in skip or n.startswith(("dev.mobileagent.fixture", "mobile-agent ", "dev.mobile.maestro",
                                          "com.callstack.agentdevice")):
                continue
            agg[n] += c
        tops = ", ".join("%s %.2f" % (GUEST_SHORT.get(n, "`%s`" % n), c / per) for n, c in agg.most_common(5))
        emu = w["ctx"].get("emulator")
        adb = w["ctx"].get("adb-server")
        label = f"{TOOL_NAME[w['tool']]}, {'idle' if w['kind'] == 'idle' else 'ten runs'}"
        lines.append(f"| {label} | %.2f | %s | %s | %s | %s |" % (
            (busy - loop) / per,
            ("%.2f, %.1f MB PSS" % (app["cpu"] / per, app["pss_mean"] / 1024)) if app else "",
            tops,
            ("%.2f, %s MB" % (emu["cpu_ns"] / NS / per, mb_short(emu["mem"][1]))) if emu else "",
            ("%.2f" % (adb["cpu_ns"] / NS / per)) if adb else ""))
    return "\n".join(lines)


def print_tables(session):
    """The tables of reports/10.20-resources.md, as markdown."""
    data, runs, order, results = summarize(session)
    emu = data.get("guest_data") is not None
    print("Windows\n")
    print(windows_table(results))
    for tool in TOOLS:
        table, resid = tool_table(data, results, tool, ORDER_EMU if emu else ORDER_SIM)
        print(f"\n{TOOL_NAME[tool]}\n")
        print(table)
        if emu:
            print()
            print(guest_table(results, tool))
        print("\nThe rows leave unexplained, in CPU-s per window: " +
              ", ".join(f"{k} {v:.3f}" for k, v in sorted(resid.items())))
    print("\nThe device and the instruments\n")
    print(guest_context_table(results, data) if emu else sim_context_table(results))


def main():
    args = sys.argv[1:]
    tables = "--tables" in args
    paths = [a for a in args if a != "--tables"]
    if len(paths) != 1 or any(a.startswith("-") for a in paths):
        sys.exit("usage: analyze.py [--tables] <session directory>")
    if tables:
        print_tables(paths[0])
    else:
        print_summary(paths[0])


if __name__ == "__main__":
    main()
