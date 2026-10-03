#!/usr/bin/env python3
"""
Usage: tools/stall-check.py [--require] [PATH-TO-CANOPY]

Fails when Canopy stops answering its compositor while its window is hidden.

On Wayland a compositor draws no window on another workspace and sends it no frame callbacks.
A renderer that presents anyway blocks in the buffer swap, the thread stops dispatching events and
Hyprland shows "Application Not Responding". eframe's glow backend did exactly that until
vendor/eframe added the missing `pre_present_notify` (vendor/README.md); this check is what keeps
an eframe upgrade from bringing it back.

It builds tools/stall-check/wlstub.c, a stand-in compositor that can withhold frame callbacks and
pings its client every second, and runs Canopy against it on a private socket with software OpenGL,
so nothing appears on the real session. It hides the window, makes it repaint (changes in the
repository it shows, and resizes) and fails when three pings in a row go unanswered, the condition
that raises the dialog. It then shows the window again and fails when it does not repaint.

Needs Linux, a C compiler, wayland-scanner, pkg-config, libwayland-server and wayland-protocols.
Without them it says so and exits 0, like ShellCheck in tools/check.sh; with --require that is an
error. Without a path it builds the debug app with cargo. Standard library only. Part of
tools/check.sh.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STUB_SOURCE = ROOT / "tools" / "stall-check" / "wlstub.c"
SOCKET = "wl-stall-check"
# The app must draw this many frames before the window is hidden.
FIRST_FRAMES = 3
# Repaints forced while the window is hidden. One is not enough: the swap that follows the last
# frame callback the compositor sent still returns.
FORCED_REPAINTS = 3
# How long the hidden window is watched after the last repaint. A stalled client misses a ping
# per second, so the third miss (the dialog's condition) arrives well inside this.
HIDDEN_WATCH = 6.0
START_TIMEOUT = 90.0
SHOW_TIMEOUT = 20.0


class Skip(Exception):
    """A prerequisite is missing."""


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    """Runs a command; on failure shows its error output and stops."""
    try:
        return subprocess.run(cmd, check=True, capture_output=True, text=True, **kwargs)
    except subprocess.CalledProcessError as failed:
        sys.stderr.write(failed.stderr or "")
        raise SystemExit(f"stall-check: `{' '.join(cmd[:4])}` failed ({failed.returncode})")


def tool_output(cmd: list[str]) -> str | None:
    try:
        return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def prerequisites() -> Path:
    """The xdg-shell protocol file; raises Skip naming what is missing."""
    if sys.platform != "linux":
        raise Skip("it needs Linux (Wayland)")
    for tool in ("cc", "wayland-scanner", "pkg-config"):
        if shutil.which(tool) is None:
            raise Skip(f"{tool} is not installed")
    if tool_output(["pkg-config", "--exists", "wayland-server"]) is None:
        raise Skip("libwayland-server is not installed")
    data = tool_output(["pkg-config", "--variable=pkgdatadir", "wayland-protocols"])
    xml = Path(data or "/nonexistent") / "stable" / "xdg-shell" / "xdg-shell.xml"
    if not xml.is_file():
        raise Skip("wayland-protocols is not installed")
    return xml


def build_stub(work: Path, xdg_xml: Path) -> Path:
    header, glue, binary = work / "xdg-shell-server-protocol.h", work / "xdg-shell-protocol.c", work / "wlstub"
    run(["wayland-scanner", "server-header", str(xdg_xml), str(header)])
    run(["wayland-scanner", "private-code", str(xdg_xml), str(glue)])
    flags = run(["pkg-config", "--cflags", "--libs", "wayland-server"]).stdout.split()
    run(["cc", "-std=gnu17", "-O1", "-Wall", f"-I{work}", "-o", str(binary), str(STUB_SOURCE), str(glue), *flags])
    return binary


def build_canopy() -> Path:
    """Builds the debug app and returns its executable."""
    out = run(
        ["cargo", "build", "-p", "canopy-app", "--locked", "--message-format=json-render-diagnostics"],
        cwd=ROOT,
    ).stdout
    for line in reversed(out.splitlines()):
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if message.get("reason") == "compiler-artifact" and message.get("executable"):
            if message["target"]["name"] == "canopy":
                return Path(message["executable"])
    raise SystemExit("stall-check: cargo built no `canopy` executable")


class Stub:
    """The stand-in compositor and everything it printed."""

    def __init__(self, binary: Path, runtime: Path) -> None:
        env = {**os.environ, "XDG_RUNTIME_DIR": str(runtime)}
        self.process = subprocess.Popen(
            [str(binary), SOCKET], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env
        )
        self.lines: list[str] = []
        self.lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            with self.lock:
                self.lines.append(line.rstrip("\n"))

    def snapshot(self) -> list[str]:
        with self.lock:
            return list(self.lines)

    def commits(self) -> int:
        for line in reversed(self.snapshot()):
            m = re.match(r"STATUS .*\bcommits=(\d+)", line)
            if m:
                return int(m.group(1))
        return 0

    def missed(self) -> int:
        for line in reversed(self.snapshot()):
            m = re.match(r"STATUS .*\bmissed=(\d+)", line)
            if m:
                return int(m.group(1))
        return 0

    def signal(self, sig: int) -> None:
        self.process.send_signal(sig)


def wait_for(condition, timeout: float, interval: float = 0.25) -> bool:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if condition():
            return True
        time.sleep(interval)
    return condition()


def eframe_is_vendored() -> str | None:
    """Why the build does not use vendor/eframe, or None when it does. Cargo only warns when a
    `[patch]` entry stops applying (after an eframe upgrade), so the check looks itself."""
    out = tool_output(["cargo", "metadata", "--locked", "--format-version", "1", "--manifest-path", str(ROOT / "Cargo.toml")])
    if out is None:
        return "`cargo metadata` failed, so it cannot tell which eframe the build uses"
    manifests = [p["manifest_path"] for p in json.loads(out)["packages"] if p["name"] == "eframe"]
    wanted = str(ROOT / "vendor" / "eframe" / "Cargo.toml")
    if manifests != [wanted]:
        return f"the build uses eframe from {manifests}, not vendor/eframe (the patch in Cargo.toml no longer applies)"
    return None


def make_repository(work: Path) -> Path:
    repo = work / "repo"
    run([str(ROOT / "tools" / "mkfixture.sh"), str(repo), "linear"])
    return repo


def canopy_env(work: Path, runtime: Path) -> dict[str, str]:
    """Canopy sees only the stand-in compositor, an empty home and no git configuration."""
    home = work / "home"
    empty_gitconfig = work / "gitconfig"
    empty_gitconfig.write_text("")
    for sub in ("data", "config", "cache", "state"):
        (home / sub).mkdir(parents=True, exist_ok=True)
    env = {
        "PATH": os.environ.get("PATH", ""),
        "XDG_RUNTIME_DIR": str(runtime),
        "WAYLAND_DISPLAY": SOCKET,
        "HOME": str(home),
        "XDG_DATA_HOME": str(home / "data"),
        "XDG_CONFIG_HOME": str(home / "config"),
        "XDG_CACHE_HOME": str(home / "cache"),
        "XDG_STATE_HOME": str(home / "state"),
        "TMPDIR": str(work),
        "GIT_CONFIG_GLOBAL": str(empty_gitconfig),
        "GIT_CONFIG_NOSYSTEM": "1",
        "LIBGL_ALWAYS_SOFTWARE": "1",
    }
    return env


def check(canopy: Path, stub_binary: Path, work: Path) -> list[str]:
    """Runs the scenario and returns what went wrong, empty when nothing did."""
    runtime = work / "run"
    runtime.mkdir(mode=0o700)
    if len(str(runtime / SOCKET)) > 100:
        return [f"TMPDIR is too deep for a Wayland socket ({runtime}); point TMPDIR at a shorter folder"]
    repo = make_repository(work)
    stub = Stub(stub_binary, runtime)
    app = None
    try:
        if not wait_for(lambda: any(l.startswith("READY") for l in stub.snapshot()), 10):
            return ["the stand-in compositor did not start: " + " | ".join(stub.snapshot())]
        # A file, not a pipe: nothing reads the app's output while the scenario runs, and a full pipe
        # would block it.
        app_log = work / "canopy.log"
        with app_log.open("w") as log:
            app = subprocess.Popen(
                [str(canopy), str(repo)],
                env=canopy_env(work, runtime),
                stdout=subprocess.DEVNULL,
                stderr=log,
            )
        if not wait_for(lambda: stub.commits() >= FIRST_FRAMES or app.poll() is not None, START_TIMEOUT):
            return [f"Canopy drew {stub.commits()} frames in {START_TIMEOUT:.0f} s; the check cannot run"]
        if app.poll() is not None:
            return [f"Canopy exited early ({app.returncode}): {app_log.read_text(errors='replace').strip()[-400:]}"]

        # Start from a client that is answering, so a slow start-up is not counted as a hang.
        if not wait_for(lambda: stub.missed() == 0, 30):
            return ["Canopy was not answering pings even while its window was shown"]
        stub.signal(signal.SIGUSR1)
        if not wait_for(lambda: "HIDDEN" in stub.snapshot(), 5):
            return ["the stand-in compositor did not hide the window"]
        hidden_from = stub.snapshot().index("HIDDEN")
        for n in range(FORCED_REPAINTS):
            (repo / f"stall-check-{n}.txt").write_text(f"change {n}\n")
            stub.signal(signal.SIGHUP)
            time.sleep(1)
        wait_for(lambda: any(l.startswith("ANR") for l in stub.snapshot()[hidden_from:]), HIDDEN_WATCH)
        hidden = stub.snapshot()[hidden_from:]
        problems = []
        if any(l.startswith("ANR") for l in hidden):
            missed = [l for l in hidden if l.startswith("MISSED")]
            problems.append(
                "Canopy stopped answering pings while its window was hidden "
                f"({len(missed)} missed; last: {missed[-1] if missed else 'none'})"
            )
        if app.poll() is not None:
            problems.append(f"Canopy exited while hidden ({app.returncode})")

        commits_hidden = stub.commits()
        stub.signal(signal.SIGUSR2)
        repainted = wait_for(lambda: stub.commits() > commits_hidden, SHOW_TIMEOUT)
        if not repainted and not problems:
            problems.append(f"Canopy did not repaint within {SHOW_TIMEOUT:.0f} s of being shown again")
        if problems:
            problems.append("compositor output: " + " | ".join(l for l in stub.snapshot() if not l.startswith("STATUS"))[-600:])
        return problems
    finally:
        if app is not None and app.poll() is None:
            app.kill()
            app.wait()
        stub.process.terminate()
        stub.process.wait()


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fails when Canopy stops answering its compositor while its window is hidden."
    )
    parser.add_argument("--require", action="store_true", help="a missing prerequisite is an error, not a skip")
    parser.add_argument("canopy", nargs="?", metavar="PATH-TO-CANOPY", help="the executable to test (default: build the debug app)")
    args = parser.parse_args()
    try:
        xdg_xml = prerequisites()
    except Skip as why:
        print(f"stall-check: skipped, {why}")
        return 2 if args.require else 0

    not_vendored = eframe_is_vendored()
    if not_vendored:
        print(f"stall-check: {not_vendored}")
        return 1

    work = Path(tempfile.mkdtemp(prefix="canopy-stall-"))
    try:
        stub_binary = build_stub(work, xdg_xml)
        canopy = Path(args.canopy).resolve() if args.canopy else build_canopy()
        problems = check(canopy, stub_binary, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    if problems:
        for problem in problems:
            print(f"stall-check: {problem}")
        return 1
    print("stall-check: ok (answered every ping while hidden, repainted when shown)")
    return 0


if __name__ == "__main__":
    # Exiting through SystemExit lets the `finally` blocks stop the children and remove the work folder.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    sys.exit(main())
