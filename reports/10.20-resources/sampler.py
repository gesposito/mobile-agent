#!/usr/bin/env python3
"""Records what every process on a macOS host costs, for reports/10.20-resources.md.

Usage, from the repository root (Python 3 standard library only):

    python3 reports/10.20-resources/sampler.py --out <dir> [--sim-udid <udid>]
    python3 reports/10.20-resources/sampler.py --out <dir> --emulator emulator-5554

It runs until SIGTERM or Ctrl-C. drive.py starts and stops it around a session;
analyze.py reads what it writes. --sim-udid names the Simulator whose
launchd_sim tree counts as the device (the published runs used
7B292E1E-7A29-41A5-9C69-2ABA1CEB3683); --emulator names the adb serial whose
guest the guest loop samples.

Host, every --interval seconds (0.5 by default): the load average and, for every
process, pid, ppid, physical footprint, resident size, accumulated CPU time of
the process itself and of its reaped children, all from proc_pid_rusage
(RUSAGE_INFO_V4); each process's command line and tool label are written once
when it is first seen, and again when it execs.

Harness, every --child-poll seconds (0.02 by default): the children of
mobile-agent-bench and the harness's reaped-children CPU, so that every command
the harness runs gets its start, its end and its exact CPU time.

Guest (with --emulator SERIAL): guest.sh, pushed to the device and run as root
through `adb shell su 0`, which needs a userdebug image. Every --interval
seconds it prints the CPU ticks and PSS of the processes whose name matches
--guest-names, found every --discover-every ticks, when it also lists every
guest process's accumulated CPU. The loop starts again after a guest reboot.

Output, under --out:
  ticks.csv        tick, epoch, load1, load5, load15, processes, scan ms
  samples.csv.gz   tick, pid, ppid, footprint B, rss B, cpu ns, child cpu ns
                   (labelled processes every tick; unlabelled ones when a value
                   changed and on every 20th tick)
  procs.jsonl      one record per process start or exec, and one when it goes
  children.jsonl   the harness's children and its reaped-children CPU deltas
  guest.txt        the guest loop's raw lines, each prefixed with the host epoch
"""
import argparse
import ctypes
import ctypes.util
import gzip
import json
import os
import shlex
import signal
import subprocess
import sys
import threading
import time

LIBC = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
LIBPROC = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)


class TimebaseInfo(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


class BsdInfo(ctypes.Structure):
    _fields_ = [
        ("pbi_flags", ctypes.c_uint32), ("pbi_status", ctypes.c_uint32),
        ("pbi_xstatus", ctypes.c_uint32), ("pbi_pid", ctypes.c_uint32),
        ("pbi_ppid", ctypes.c_uint32), ("pbi_uid", ctypes.c_uint32),
        ("pbi_gid", ctypes.c_uint32), ("pbi_ruid", ctypes.c_uint32),
        ("pbi_rgid", ctypes.c_uint32), ("pbi_svuid", ctypes.c_uint32),
        ("pbi_svgid", ctypes.c_uint32), ("rfu_1", ctypes.c_uint32),
        ("pbi_comm", ctypes.c_char * 16), ("pbi_name", ctypes.c_char * 32),
        ("pbi_nfiles", ctypes.c_uint32), ("pbi_pgid", ctypes.c_uint32),
        ("pbi_pjobc", ctypes.c_uint32), ("e_tdev", ctypes.c_uint32),
        ("e_tpgid", ctypes.c_uint32), ("pbi_nice", ctypes.c_int32),
        ("pbi_start_tvsec", ctypes.c_uint64), ("pbi_start_tvusec", ctypes.c_uint64),
    ]


V4_FIELDS = (
    "user_time system_time pkg_idle_wkups interrupt_wkups pageins wired_size "
    "resident_size phys_footprint proc_start_abstime proc_exit_abstime "
    "child_user_time child_system_time child_pkg_idle_wkups child_interrupt_wkups "
    "child_pageins child_elapsed_abstime diskio_bytesread diskio_byteswritten "
    "cpu_time_qos_default cpu_time_qos_maintenance cpu_time_qos_background "
    "cpu_time_qos_utility cpu_time_qos_legacy cpu_time_qos_user_initiated "
    "cpu_time_qos_user_interactive billed_system_time serviced_system_time "
    "logical_writes lifetime_max_phys_footprint instructions cycles billed_energy "
    "serviced_energy interval_max_phys_footprint runnable_time"
).split()


class RusageV4(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [(f, ctypes.c_uint64) for f in V4_FIELDS]


_tb = TimebaseInfo()
LIBC.mach_timebase_info(ctypes.byref(_tb))
TICK_NS_NUM, TICK_NS_DEN = _tb.numer, _tb.denom  # 125/3 on Apple silicon

PROC_PIDTBSDINFO = 3
RUSAGE_INFO_V4 = 4
CTL_KERN, KERN_ARGMAX, KERN_PROCARGS2 = 1, 8, 49

# Where guest.sh goes on the Android device.
GUEST_SCRIPT = "/data/local/tmp/res-guest.sh"


def ticks_to_ns(t):
    return t * TICK_NS_NUM // TICK_NS_DEN


def list_pids():
    n = LIBPROC.proc_listallpids(None, 0)
    buf = (ctypes.c_int * (n + 512))()
    n = LIBPROC.proc_listallpids(buf, ctypes.sizeof(buf))
    return [buf[i] for i in range(max(n, 0)) if buf[i] > 0]


def list_children(ppid):
    buf = (ctypes.c_int * 256)()
    n = LIBPROC.proc_listchildpids(ppid, buf, ctypes.sizeof(buf))
    return [buf[i] for i in range(max(n, 0)) if buf[i] > 0]


def bsdinfo(pid):
    b = BsdInfo()
    n = LIBPROC.proc_pidinfo(pid, PROC_PIDTBSDINFO, ctypes.c_uint64(0), ctypes.byref(b), ctypes.sizeof(b))
    return b if n == ctypes.sizeof(b) else None


def rusage(pid):
    r = RusageV4()
    if LIBPROC.proc_pid_rusage(pid, RUSAGE_INFO_V4, ctypes.byref(r)) != 0:
        return None
    return r


def _argmax():
    mib = (ctypes.c_int * 2)(CTL_KERN, KERN_ARGMAX)
    val = ctypes.c_int(0)
    size = ctypes.c_size_t(ctypes.sizeof(val))
    LIBC.sysctl(mib, 2, ctypes.byref(val), ctypes.byref(size), None, 0)
    return val.value or 1 << 20


ARGMAX = _argmax()
_ARGBUF = ctypes.create_string_buffer(ARGMAX)


def procargs(pid):
    """argv of a process, from KERN_PROCARGS2; None when it cannot be read."""
    mib = (ctypes.c_int * 3)(CTL_KERN, KERN_PROCARGS2, pid)
    size = ctypes.c_size_t(ARGMAX)
    if LIBC.sysctl(mib, 3, _ARGBUF, ctypes.byref(size), None, 0) != 0:
        return None
    raw = _ARGBUF.raw[: size.value]
    if len(raw) < 4:
        return None
    argc = int.from_bytes(raw[:4], "little")
    rest = raw[4:]
    end = rest.find(b"\0")
    if end < 0:
        return None
    pos = end
    while pos < len(rest) and rest[pos] == 0:
        pos += 1
    args = []
    for _ in range(argc):
        nxt = rest.find(b"\0", pos)
        if nxt < 0:
            break
        args.append(rest[pos:nxt].decode("utf-8", "replace"))
        pos = nxt + 1
    return args


def procpath(pid):
    buf = ctypes.create_string_buffer(4096)
    n = LIBPROC.proc_pidpath(pid, buf, 4096)
    return buf.value.decode("utf-8", "replace") if n > 0 else None


class Labeller:
    """Names each process's tool from its tree and its command."""

    def __init__(self, sim_udid, emulator_serial):
        self.sim_udid = sim_udid
        self.emulator = emulator_serial

    def label(self, pid, comm, cmd, parent_label, parent_cmd):
        c = " ".join(cmd) if cmd else comm
        exe = cmd[0] if cmd else comm
        base = os.path.basename(exe)
        if pid == os.getpid():
            return "sampler"
        # mobile-agent
        if base == "mobile-agent-daemon":
            return "mobile-agent:daemon"
        if base == "mobile-agent":
            return "mobile-agent:cli"
        if "dev.mobileagent.simulator/simulator-reader" in c:
            return "mobile-agent:reader"
        if base == "mobile-agent-bench":
            return "harness"
        # agent-device
        if base in ("node", "agent-device") and "agent-device" in c:
            if "daemon" in c and ("dist/src/daemon" in c or "internal/daemon" in c or c.rstrip().endswith("daemon.js")):
                return "agent-device:daemon"
            return "agent-device:client"
        in_sim_bundle = "/data/Containers/Bundle/Application/" in exe
        if "AgentDeviceRunner" in c:
            if base == "xcodebuild":
                return "agent-device:runner-xcodebuild"
            if in_sim_bundle:
                return "agent-device:runner-app"
        if base == "snapshot-bridge":
            return "agent-device:bridge"
        if base in ("simctl", "xcrun") and "spawn" in cmd and "snapshot-bridge" in c:
            return "agent-device:bridge-simctl"
        # Maestro
        if base == "java" and "JvmVersion" in c and "maestro" in c.lower():
            return "maestro:launcher"
        if base == "java" and ("maestro" in c.lower()):
            return "maestro:jvm"
        if "maestro-driver" in c or "dev.mobile.maestro" in c:
            if base == "xcodebuild":
                return "maestro:driver-xcodebuild"
            if in_sim_bundle:
                return "maestro:driver-app"
        if base in ("sh", "bash") and len(cmd) > 1 and os.path.basename(cmd[1]) == "maestro":
            return "maestro:launcher"
        # devices and shared services
        if base == "launchd_sim":
            if self.sim_udid and self.sim_udid in c:
                return "device:sim"
            return "other-sim"
        if base.startswith("qemu-system") or (base == "emulator" and "-avd" in cmd):
            return "device:emulator"
        if base == "adb" and "fork-server" in c:
            return "shared:adb-server"
        if "CoreSimulatorService" in c:
            return "shared:coresimulator"
        # inherited labels
        if parent_label:
            if parent_label == "device:sim" or parent_label.startswith("device:sim/"):
                if "dev.mobileagent.fixture" in c or "/Fixture" in c:
                    return "device:sim/app"
                if base == "testmanagerd":
                    return "device:sim/testmanagerd"
                return "device:sim/system"
            if parent_label == "other-sim":
                return "other-sim"
            if parent_label.startswith("device:emulator"):
                return "device:emulator/child"
            for tool in ("mobile-agent", "agent-device", "maestro"):
                if parent_label.startswith(tool + ":"):
                    return tool + ":child"
            if parent_label.startswith("harness"):
                return "harness:child"
            if parent_label == "sampler" or parent_label == "sampler:child":
                return "sampler:child"
        return "other"


class Sampler:
    def __init__(self, args):
        self.args = args
        self.out = args.out
        os.makedirs(self.out, exist_ok=True)
        self.labeller = Labeller(args.sim_udid, args.emulator)
        self.procs = {}  # pid -> dict(start, comm, label, cmd)
        self.last_row = {}  # pid -> tuple written last
        self.stop = threading.Event()
        self.bench_pid = None
        self.lock = threading.Lock()
        self.ticks = open(os.path.join(self.out, "ticks.csv"), "a", buffering=1)
        self.samples = gzip.open(os.path.join(self.out, "samples.csv.gz"), "at", compresslevel=1)
        self.procs_f = open(os.path.join(self.out, "procs.jsonl"), "a", buffering=1)
        self.children_f = open(os.path.join(self.out, "children.jsonl"), "a", buffering=1)
        self.guest_f = None
        self.guest_proc = None

    # ---- host scan -------------------------------------------------------
    def scan(self, tick):
        t0 = time.perf_counter()
        now = time.time()
        pids = list_pids()
        seen = set()
        infos = {}
        for pid in pids:
            b = bsdinfo(pid)
            if b is None:
                continue
            infos[pid] = b
        # parents first so labels inherit: sort by start time
        order = sorted(infos, key=lambda p: (infos[p].pbi_start_tvsec, infos[p].pbi_start_tvusec, p))
        rows = []
        for pid in order:
            b = infos[pid]
            start = b.pbi_start_tvsec + b.pbi_start_tvusec / 1e6
            comm = b.pbi_comm.decode("utf-8", "replace")
            seen.add(pid)
            known = self.procs.get(pid)
            if known is None or known["start"] != start or known["comm"] != comm:
                cmd = procargs(pid)
                if not cmd:
                    path = procpath(pid)
                    cmd = [path] if path else [comm]
                parent = self.procs.get(b.pbi_ppid)
                label = self.labeller.label(
                    pid, comm, cmd,
                    parent["label"] if parent else None,
                    parent["cmd"] if parent else None,
                )
                if known is not None and known["start"] == start and known["label"] not in ("other",) and label in ("other",):
                    label = known["label"]
                ev = "exec" if (known is not None and known["start"] == start) else "new"
                if known is not None and known["start"] != start:
                    self.procs_f.write(json.dumps({"ev": "gone", "pid": pid, "start": known["start"], "tick": tick, "t": now}) + "\n")
                self.procs[pid] = {"start": start, "comm": comm, "label": label, "cmd": cmd}
                self.procs_f.write(json.dumps({
                    "ev": ev, "pid": pid, "ppid": b.pbi_ppid, "start": start, "uid": b.pbi_uid,
                    "comm": comm, "label": label, "cmd": cmd, "tick": tick, "t": now,
                }) + "\n")
                self.last_row.pop(pid, None)
            label = self.procs[pid]["label"]
            if label == "harness":
                with self.lock:
                    self.bench_pid = pid
            r = rusage(pid)
            if r is None:
                row = (b.pbi_ppid, -1, -1, -1, -1)
            else:
                row = (
                    b.pbi_ppid, r.phys_footprint, r.resident_size,
                    ticks_to_ns(r.user_time + r.system_time),
                    ticks_to_ns(r.child_user_time + r.child_system_time),
                )
            if label not in ("other", "other-sim") or tick % 20 == 0 or self.last_row.get(pid) != row:
                rows.append("%d,%d,%d,%d,%d,%d,%d\n" % ((tick, pid) + row))
                self.last_row[pid] = row
        for pid in list(self.procs):
            if pid not in seen:
                known = self.procs.pop(pid)
                self.last_row.pop(pid, None)
                self.procs_f.write(json.dumps({"ev": "gone", "pid": pid, "start": known["start"], "tick": tick, "t": now}) + "\n")
        self.samples.write("".join(rows))
        load = os.getloadavg()
        scan_ms = (time.perf_counter() - t0) * 1000
        self.ticks.write("%d,%.6f,%.2f,%.2f,%.2f,%d,%.2f\n" % (tick, now, load[0], load[1], load[2], len(seen), scan_ms))

    def host_loop(self):
        interval = self.args.interval
        start = time.monotonic()
        tick = 0
        while not self.stop.is_set():
            self.scan(tick)
            tick += 1
            if tick % 20 == 0:
                self.samples.flush()
            nxt = start + tick * interval
            delay = nxt - time.monotonic()
            if delay > 0:
                self.stop.wait(delay)
        self.samples.flush()

    # ---- harness children -------------------------------------------------
    def child_loop(self):
        period = self.args.child_poll
        prev = {}
        prev_ccpu = None
        prev_bench = None
        while not self.stop.is_set():
            with self.lock:
                bench = self.bench_pid
            if bench is None:
                self.stop.wait(period)
                continue
            r = rusage(bench)
            now = time.time()
            if r is None:
                if prev_bench is not None:
                    self.children_f.write(json.dumps({"t": now, "ev": "bench-gone", "bench": prev_bench}) + "\n")
                with self.lock:
                    if self.bench_pid == bench:
                        self.bench_pid = None
                prev, prev_ccpu, prev_bench = {}, None, None
                self.stop.wait(period)
                continue
            if bench != prev_bench:
                b = bsdinfo(bench)
                self.children_f.write(json.dumps({
                    "t": now, "ev": "bench", "bench": bench,
                    "start": (b.pbi_start_tvsec + b.pbi_start_tvusec / 1e6) if b else None,
                    "cmd": procargs(bench),
                    "cpu_ns": ticks_to_ns(r.user_time + r.system_time),
                    "ccpu_ns": ticks_to_ns(r.child_user_time + r.child_system_time),
                }) + "\n")
                prev, prev_ccpu, prev_bench = {}, ticks_to_ns(r.child_user_time + r.child_system_time), bench
            kids = list_children(bench)
            ccpu = ticks_to_ns(r.child_user_time + r.child_system_time)
            # read the children list again after the rusage, so a child reaped
            # between the two reads is seen gone and its CPU is already counted
            cur = {}
            for pid in kids:
                if pid in prev:
                    cur[pid] = prev[pid]
                    continue
                b = bsdinfo(pid)
                if b is None:
                    continue
                start = b.pbi_start_tvsec + b.pbi_start_tvusec / 1e6
                cmd = procargs(pid) or [b.pbi_comm.decode("utf-8", "replace")]
                cur[pid] = start
                self.children_f.write(json.dumps({"t": now, "ev": "start", "pid": pid, "start": start, "cmd": cmd}) + "\n")
            gone = [p for p in prev if p not in cur]
            if gone or ccpu != prev_ccpu:
                self.children_f.write(json.dumps({
                    "t": now, "ev": "end" if gone else "cpu", "gone": gone,
                    "dccpu_ns": ccpu - prev_ccpu, "ccpu_ns": ccpu,
                    "bench_cpu_ns": ticks_to_ns(r.user_time + r.system_time),
                }) + "\n")
            prev, prev_ccpu = cur, ccpu
            self.stop.wait(period)

    # ---- guest -------------------------------------------------------------
    def guest_loop(self):
        """Runs the guest loop, and starts it again once the guest has booted
        whenever it ends, as when the guest reboots."""
        serial = self.args.emulator
        script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "guest.sh")
        self.guest_f = open(os.path.join(self.out, "guest.txt"), "a", buffering=1)
        remote = "su 0 sh %s %s %d %s" % (
            GUEST_SCRIPT, self.args.interval, self.args.discover_every, shlex.quote(self.args.guest_names))
        while not self.stop.is_set():
            try:
                booted = subprocess.run(["adb", "-s", serial, "shell", "getprop", "sys.boot_completed"],
                                        capture_output=True, text=True, timeout=10).stdout.strip() == "1"
            except subprocess.TimeoutExpired:
                booted = False
            if not booted:
                self.stop.wait(2)
                continue
            try:
                subprocess.run(["adb", "-s", serial, "push", script, GUEST_SCRIPT],
                               check=True, capture_output=True, timeout=30)
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                self.stop.wait(2)
                continue
            self.guest_f.write("%.6f X loop-start\n" % time.time())
            self.guest_proc = subprocess.Popen(["adb", "-s", serial, "shell", remote], stdout=subprocess.PIPE,
                                               stderr=subprocess.STDOUT, text=True, bufsize=1)
            for line in self.guest_proc.stdout:
                self.guest_f.write("%.6f %s" % (time.time(), line))
                if self.stop.is_set():
                    break
            self.guest_f.write("%.6f X loop-end\n" % time.time())
            self.stop.wait(1)

    def run(self):
        threads = [threading.Thread(target=self.host_loop, daemon=True),
                   threading.Thread(target=self.child_loop, daemon=True)]
        if self.args.emulator:
            threads.append(threading.Thread(target=self.guest_loop, daemon=True))
        for t in threads:
            t.start()

        def _stop(*_):
            self.stop.set()

        signal.signal(signal.SIGTERM, _stop)
        signal.signal(signal.SIGINT, _stop)
        while not self.stop.is_set():
            self.stop.wait(1.0)
        if self.guest_proc:
            subprocess.run(["adb", "-s", self.args.emulator, "shell",
                            "su 0 pkill -f %s; true" % os.path.basename(GUEST_SCRIPT)], capture_output=True)
            self.guest_proc.terminate()
        for t in threads[:2]:
            t.join(timeout=5)
        self.samples.close()
        self.ticks.close()
        self.procs_f.close()
        self.children_f.close()
        if self.guest_f:
            self.guest_f.close()


def main():
    ap = argparse.ArgumentParser(description="Records what every process on the host costs; see the module docstring.")
    ap.add_argument("--out", required=True, help="directory for the samples")
    ap.add_argument("--interval", type=float, default=0.5, help="seconds between scans [0.5]")
    ap.add_argument("--child-poll", type=float, default=0.02, help="seconds between polls of the harness's children [0.02]")
    ap.add_argument("--sim-udid", help="the Simulator whose launchd_sim tree is the device")
    ap.add_argument("--emulator", help="adb serial of the emulator whose guest to sample, such as emulator-5554")
    ap.add_argument("--discover-every", type=int, default=4,
                    help="guest ticks between scans for the tools' processes [4]")
    ap.add_argument("--guest-names", default="mobile-agent|maestro|agentdevice|dev.mobileagent.fixture|instrument",
                    help="extended regular expression for the guest processes to follow")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    with open(os.path.join(args.out, "sampler.pid"), "w") as f:
        f.write(str(os.getpid()))
    Sampler(args).run()


if __name__ == "__main__":
    main()
