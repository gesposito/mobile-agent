#!/usr/bin/env python3
"""Runs one device's measurement session of reports/10.20-resources.md.

Usage, from the repository root once `cargo build --release` has built the
workspace and the device is booted (Python 3 standard library only):

    python3 reports/10.20-resources/drive.py --device sim --out <dir>
    python3 reports/10.20-resources/drive.py --device emu --out <dir>

It starts sampler.py (with its guest loop on the emulator), waits until the
1-minute load has stayed under 4 for 60 s, runs the harness's login series for
mobile-agent, Maestro and agent-device (`mobile-agent-bench run --flow login
--iterations 10 --warmup 1`), then each tool's idle window: one login flow run
with the harness's commands and flow files, 60 s with the tool attached and no
command running, and the release. Every step is a timestamped mark, with the
load average, in <dir>/marks.jsonl; the samples go to <dir>/samples and the
harness's reports to <dir>/run. analyze.py summarizes the directory.

Defaults are the published runs':
  --target       7B292E1E-7A29-41A5-9C69-2ABA1CEB3683 (sim), emulator-5554 (emu)
  --repo         the repository this script is in, for target/release
  --socket-dir   $MOBILE_AGENT_SOCKET_DIR, else a fresh /tmp/ma-* directory
  --state-dir    $AGENT_DEVICE_STATE_DIR, else a fresh /tmp/ad-* directory
  MOBILE_AGENT_DRIVERS   simulator (sim) or android (emu), unless already set
Maestro and agent-device must be on PATH.
"""
import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
PY = sys.executable
SESSION = "mobile-agent-bench"
DEFAULT_TARGET = {"sim": "7B292E1E-7A29-41A5-9C69-2ABA1CEB3683", "emu": "emulator-5554"}
DRIVERS = {"sim": "simulator", "emu": "android"}
# After the harness, agent-device's daemon idles for five minutes on Android,
# where the harness's release is `close` alone (task 10.27); wait it out.
AFTER_HARNESS_WAIT = {"sim": 30.0, "emu": 330.0}


class Driver:
    def __init__(self, a):
        self.a = a
        os.makedirs(a.out, exist_ok=True)
        self.marks = open(os.path.join(a.out, "marks.jsonl"), "a", buffering=1)
        self.log = open(os.path.join(a.out, "drive.log"), "a", buffering=1)
        self.cli = os.path.join(a.repo, "target/release/mobile-agent")
        self.bench = os.path.join(a.repo, "target/release/mobile-agent-bench")
        self.platform = "ios" if a.device == "sim" else "android"
        self.app = {"sim": "dev.mobileagent.fixture.swiftui", "emu": "dev.mobileagent.fixture.compose"}[a.device]
        self.fixture = {"sim": "swiftui", "emu": "compose"}[a.device]
        self.sampler = None

    def mark(self, ev, **extra):
        rec = {"t": time.time(), "ev": ev, "load": [round(x, 2) for x in os.getloadavg()]}
        rec.update(extra)
        self.marks.write(json.dumps(rec) + "\n")
        self.say(f"mark {ev} {rec['load']} {extra if extra else ''}")

    def say(self, msg):
        self.log.write(time.strftime("%H:%M:%S ") + msg + "\n")

    def run(self, argv, name, timeout=600):
        t0 = time.time()
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        out = (p.stdout or "") + (p.stderr or "")
        self.say(f"$ {' '.join(argv)}  -> {p.returncode} in {time.time() - t0:.2f}s\n{out[-1500:]}")
        return p.returncode, p.stdout

    def wait_quiet(self, label):
        need = self.a.quiet_secs
        limit = self.a.load_limit
        t0 = time.time()
        calm_since = None
        while True:
            l1 = os.getloadavg()[0]
            now = time.time()
            if l1 < limit:
                calm_since = calm_since or now
                if now - calm_since >= need:
                    break
            else:
                calm_since = None
            if now - t0 > self.a.quiet_timeout:
                self.mark("quiet-timeout", window=label)
                raise SystemExit(f"load did not stay under {limit} for {need}s within {self.a.quiet_timeout}s")
            time.sleep(5)
        self.mark("quiet", window=label, waited=round(time.time() - t0, 1))

    # ---- tool processes ------------------------------------------------------
    def tool_pids(self):
        p = subprocess.run(["ps", "-A", "-ww", "-o", "pid=,command="], capture_output=True, text=True)
        found = []
        for line in p.stdout.splitlines():
            pid, _, cmd = line.strip().partition(" ")
            if ("mobile-agent-daemon" in cmd or "agent-device/dist/src/internal/daemon" in cmd
                    or "AgentDeviceRunner" in cmd or "snapshot-bridge" in cmd
                    or ("java" in cmd and "maestro" in cmd) or "maestro-driver-ios" in cmd
                    or "simulator-reader-" in cmd):
                found.append((pid, cmd[:160]))
        return found

    def wait_gone(self, label, limit=30):
        t0 = time.time()
        while time.time() - t0 < limit:
            left = self.tool_pids()
            if not left:
                self.mark("gone", window=label, after=round(time.time() - t0, 2))
                return
            time.sleep(0.25 if limit <= 30 else 2.0)
        self.mark("left-running", window=label, left=self.tool_pids())

    # ---- tools ------------------------------------------------------------------
    def dev_flag(self):
        return ["--udid", self.a.target] if self.a.device == "sim" else ["--serial", self.a.target]

    def ma(self, *args):
        return self.run([self.cli, "--json", "--session", SESSION, "--device", self.a.target, *args], "mobile-agent")

    def ma_flow(self):
        self.ma("open", "--relaunch", "--", self.app)
        self.ma("fill", "--timeout", "10000", "--", "id=login-username", "agent")
        self.ma("fill", "--timeout", "10000", "--", "id=login-password", "hunter2")
        self.ma("click", "--timeout", "10000", "--", "id=login-submit")
        self.ma("wait", "--timeout", "10000", "--text=Welcome, agent")
        return self.ma("get", "text", "--timeout", "10000", "--", "id=login-status")

    def ma_release(self):
        self.ma("status")
        for _ in range(10):
            code, out = self.run([self.cli, "--json", "shutdown"], "mobile-agent")
            if '"stopped":true' not in out.replace(" ", ""):
                break

    def maestro_flow(self):
        flow = os.path.join(self.a.flows, f"maestro-login-{self.a.target}.yaml")
        return self.run(["maestro", "--device", self.a.target, "test", flow], "maestro")

    def ad_flow(self):
        flow = os.path.join(self.a.flows, f"agent-device-login-{self.a.target}.json")
        return self.run(["agent-device", "batch", "--steps-file", flow, "--on-error", "stop", "--json",
                         "--platform", self.platform, *self.dev_flag(), "--session", SESSION], "agent-device")

    def ad_release(self):
        """`close`, then `daemon stop --clean`, which the harness runs on a
        Simulator only; on Android it ends the daemon the release would leave
        idling (task 10.27)."""
        self.run(["agent-device", "close", "--json", "--platform", self.platform, *self.dev_flag(),
                  "--session", SESSION], "agent-device")
        self.run(["agent-device", "daemon", "stop", "--clean", "--json"], "agent-device")

    # ---- session ----------------------------------------------------------------
    def start_sampler(self):
        argv = [PY, os.path.join(HERE, "sampler.py"), "--out", os.path.join(self.a.out, "samples")]
        if self.a.device == "sim":
            argv += ["--sim-udid", self.a.target]
        else:
            argv += ["--emulator", self.a.target]
        self.sampler = subprocess.Popen(argv, stdout=open(os.path.join(self.a.out, "sampler.log"), "a"),
                                        stderr=subprocess.STDOUT)
        self.mark("sampler-start", pid=self.sampler.pid)
        time.sleep(3)

    def stop_sampler(self):
        if self.sampler:
            self.sampler.send_signal(signal.SIGTERM)
            try:
                self.sampler.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.sampler.kill()
            self.mark("sampler-stop", code=self.sampler.returncode)

    def reset_device(self):
        if self.a.device == "sim":
            self.run(["xcrun", "simctl", "terminate", self.a.target, self.app], "reset")
        else:
            s = self.a.target
            self.run(["adb", "-s", s, "shell", "am", "force-stop", self.app], "reset")
            self.run(["adb", "-s", s, "shell", "input", "keyevent", "KEYCODE_HOME"], "reset")

    def harness(self):
        self.reset_device()
        self.wait_quiet("harness")
        argv = [self.bench, "run", "--target", self.a.target, "--cli", self.cli, "--fixture", self.fixture,
                "--flow", "login", "--iterations", str(self.a.iterations), "--warmup", str(self.a.warmup),
                "--out", os.path.join(self.a.out, "run")]
        self.mark("harness-start", argv=argv)
        t0 = time.time()
        with open(os.path.join(self.a.out, "run.log"), "a") as f:
            p = subprocess.run(argv, stdout=f, stderr=subprocess.STDOUT)
        self.mark("harness-end", code=p.returncode, secs=round(time.time() - t0, 1))
        self.wait_gone("after-harness", limit=self.a.after_harness_wait)

    def idle(self, tool, flow, release):
        self.reset_device()
        self.wait_quiet(f"{tool}-idle")
        self.mark(f"{tool}-warmup-start")
        flow()
        before = self.tool_pids()
        self.mark(f"{tool}-idle-start", tools=before)
        time.sleep(self.a.idle_secs)
        self.mark(f"{tool}-idle-end")
        self.mark(f"{tool}-idle-tools", tools=self.tool_pids())
        if release:
            self.mark(f"{tool}-release-start")
            release()
            self.mark(f"{tool}-release-end")
        self.wait_gone(f"{tool}-idle")

    def session(self):
        self.mark("session-start", device=self.a.device, target=self.a.target,
                  drivers=os.environ.get("MOBILE_AGENT_DRIVERS"))
        left = self.tool_pids()
        if left:
            self.mark("abort-tools-running", left=left)
            raise SystemExit(f"tool processes already running: {left}")
        self.start_sampler()
        try:
            steps = self.a.steps.split(",")
            if "harness" in steps:
                self.harness()
            if "idle" in steps:
                self.idle("mobile-agent", self.ma_flow, self.ma_release)
                self.idle("maestro", self.maestro_flow, None)
                self.idle("agent-device", self.ad_flow, self.ad_release)
            self.reset_device()
            time.sleep(3)
        finally:
            self.stop_sampler()
            self.mark("session-end")


def main():
    ap = argparse.ArgumentParser(description="Runs one device's measurement session; see the module docstring.")
    ap.add_argument("--device", choices=["sim", "emu"], required=True)
    ap.add_argument("--target", help="Simulator UDID or adb serial [the published device]")
    ap.add_argument("--repo", default=os.path.dirname(os.path.dirname(HERE)),
                    help="repository whose target/release holds mobile-agent and mobile-agent-bench")
    ap.add_argument("--out", required=True, help="directory for the session's marks, samples and reports")
    ap.add_argument("--socket-dir", default=os.environ.get("MOBILE_AGENT_SOCKET_DIR"),
                    help="mobile-agent's socket directory [$MOBILE_AGENT_SOCKET_DIR, else a fresh /tmp/ma-*]")
    ap.add_argument("--state-dir", default=os.environ.get("AGENT_DEVICE_STATE_DIR"),
                    help="agent-device's state directory [$AGENT_DEVICE_STATE_DIR, else a fresh /tmp/ad-*]")
    ap.add_argument("--flows", help="directory with the harness's flow files [<out>/run/flows]")
    ap.add_argument("--steps", default="harness,idle", help="which parts to run [harness,idle]")
    ap.add_argument("--iterations", type=int, default=10)
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--idle-secs", type=float, default=60)
    ap.add_argument("--load-limit", type=float, default=4.0, help="1-minute load to stay under before a window [4]")
    ap.add_argument("--quiet-secs", type=float, default=60, help="how long it must stay under [60]")
    ap.add_argument("--quiet-timeout", type=float, default=3600, help="give up after this many seconds [3600]")
    ap.add_argument("--after-harness-wait", type=float,
                    help="seconds to wait for the tools' processes to go after the harness [30 sim, 330 emu]")
    a = ap.parse_args()
    a.target = a.target or DEFAULT_TARGET[a.device]
    a.flows = a.flows or os.path.join(a.out, "run", "flows")
    if a.after_harness_wait is None:
        a.after_harness_wait = AFTER_HARNESS_WAIT[a.device]
    # Unix socket paths are limited to 103 bytes, so the directories are short.
    a.socket_dir = a.socket_dir or tempfile.mkdtemp(prefix="ma-", dir="/tmp")
    a.state_dir = a.state_dir or tempfile.mkdtemp(prefix="ad-", dir="/tmp")
    os.environ["MOBILE_AGENT_SOCKET_DIR"] = a.socket_dir
    os.environ["AGENT_DEVICE_STATE_DIR"] = a.state_dir
    os.environ.setdefault("MOBILE_AGENT_DRIVERS", DRIVERS[a.device])
    Driver(a).session()


if __name__ == "__main__":
    main()
