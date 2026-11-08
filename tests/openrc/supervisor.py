#!/usr/bin/env python3
"""Opt-in unprivileged supervisor test. No host init services or Stutter operations.

Usage: python3 tests/openrc/supervisor.py --package STORE_PATH --out NEW_DIRECTORY
This exercises real OpenRC process supervision, not product/backend qualification.
"""
import argparse
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--child", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.child:
        with args.child.open("a") as log:
            log.write(json.dumps({"pid": os.getpid(), "start_ticks": start_ticks(os.getpid())}) + "\n")
        while True:
            time.sleep(1)
    if os.geteuid() == 0:
        parser.error("run as an unprivileged account")
    if args.package is None or args.out is None:
        parser.error("--package and --out are required")
    root = args.out.resolve()
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    state = root / "state"
    state.mkdir(mode=0o700)
    env = {key: value for key, value in os.environ.items() if not key.startswith("RC_")}
    env.update(RC_SVCDIR=str(state), RC_USER_SERVICES="yes", XDG_RUNTIME_DIR=str(root))
    binary = args.package.resolve() / "sbin/supervise-daemon"
    name = "stutter-private-fixture"
    env["RC_SVCNAME"] = name
    pidfile = root / "supervisor.pid"
    children = root / "children.jsonl"
    handles = []
    supervisor_handles = []

    def run(*argv):
        result = subprocess.run([str(binary), name, *map(str, argv)], env=env,
                                capture_output=True, timeout=12, check=False)
        with (root / "commands.jsonl").open("a") as log:
            log.write(json.dumps({"argv": list(map(str, argv)), "status": result.returncode,
                                  "stdout": result.stdout.decode(errors="replace"),
                                  "stderr": result.stderr.decode(errors="replace")}) + "\n")
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors="replace"))

    def wait_for(predicate):
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(0.02)
        raise RuntimeError("supervisor lifecycle deadline exceeded")

    def capture_child(count):
        def capture():
            rows = children.read_text().splitlines() if children.exists() else []
            if len(rows) < count:
                return None
            row = json.loads(rows[count - 1])
            fd = os.pidfd_open(row["pid"])
            assert start_ticks(row["pid"]) == row["start_ticks"]
            handles.append(fd)
            return row, fd
        return wait_for(capture)

    def exited(fd):
        return bool(select.select([fd], [], [], 0)[0])

    def start():
        run("--start", "--pidfile", pidfile, "--respawn-delay", "1",
            "--respawn-max", "3", "--respawn-period", "60", "--no-new-privs",
            "--stop-group", "--retry", "TERM/2/KILL/1",
            "--stdout", root / "stdout.log", "--stderr", root / "stderr.log",
            sys.executable, "--", Path(__file__).resolve(), "--child", children)
        pid = int(wait_for(lambda: pidfile.read_text().strip() if pidfile.exists() else None))
        supervisor_handles.append(os.pidfd_open(pid))

    try:
        start()
        first, fd = capture_child(1)
        status = Path(f'/proc/{first["pid"]}/status').read_text()
        assert "NoNewPrivs:\t1" in status
        signal.pidfd_send_signal(fd, signal.SIGKILL)
        second, _ = capture_child(2)
        assert second != first
        wait_for(lambda: exited(fd))
        run("--stop", "--pidfile", pidfile)
        wait_for(lambda: all(exited(fd) for fd in handles + supervisor_handles))
        assert not pidfile.exists()
        start()
        capture_child(3)
        run("--stop", "--pidfile", pidfile)
        wait_for(lambda: all(exited(fd) for fd in handles + supervisor_handles))
        assert not pidfile.exists()
        (root / "result.json").write_text(json.dumps({"result": "PASS", "scope": "unprivileged_dummy_supervisor",
            "backend_qualified": False, "child_executions": 3,
            "checks": ["start", "child_sigkill_respawn", "stop", "restart", "no_new_privs", "all_recorded_processes_exited"]}, indent=2) + "\n")
    finally:
        try:
            if pidfile.exists():
                run("--stop", "--pidfile", pidfile)
        finally:
            for fd in handles + supervisor_handles:
                try:
                    if not exited(fd):
                        signal.pidfd_send_signal(fd, signal.SIGKILL)
                finally:
                    os.close(fd)


def start_ticks(pid):
    return int(Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19])


if __name__ == "__main__":
    main()
