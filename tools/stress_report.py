#!/usr/bin/env python3
"""Summarise a Canopy stress-test run.

Reads Canopy's JSON-lines log (Settings -> Privacy -> Log detail "Detailed", or CANOPY_LOG, so
that `job done`, `snapshot` and `git finished` events are present), the resource sampler's CSV
written by `tools/stress.sh open`, and optionally the action log of `tools/stress.sh churn`.
It prints counts of warnings and errors, job / operation / git-command durations, the slowest
repositories, load times, peaks of memory, threads and descriptors, idle CPU, refresh lag after
external changes, and a verdict against the pass criteria in docs/stress-test.md.

Usage:
    stress_report.py --run DIR               # DIR has canopy.jsonl, samples.csv, meta.txt, churn-*.log
    stress_report.py --log FILE [--csv FILE] [--churn FILE] [--meta FILE]

Exit status: 1 if any pass criterion failed, 2 on unusable input, otherwise 0.
Standard library only.
"""

import argparse
import bisect
import calendar
import csv
import glob
import json
import os
import re
import statistics
import sys
from collections import Counter, defaultdict

# Pass criteria (docs/stress-test.md, "Pass criteria"). Override one with --limit NAME=VALUE.
LIMITS = {
    "all_loaded_s": 120.0,  # last tab's first snapshot, seconds after start
    "repo_loaded_s": 60.0,  # p95 of the tabs' first snapshots
    "first_frame_ms": 2000.0,  # the `first frame` event (INFO)
    "idle_cpu_median": 2.0,  # percent of one core, 100 tabs idle
    "idle_cpu_p95": 5.0,
    "rss_peak_mib": 4096.0,
    "rss_loaded_mib": 2048.0,  # median RSS in the minute after all tabs loaded
    "rss_growth_pct": 10.0,  # heap growth between first and last five minutes
    "rss_growth_slack_mib": 50.0,
    "threads_peak": 450.0,
    "threads_growth": 20.0,
    "fds_frac_of_limit": 0.70,
    "fds_growth": 50.0,
    "watches_frac_of_limit": 0.90,
    "job_p95_ms": 500.0,  # details, diff, conflict, ...
    "job_max_ms": 10000.0,
    "snapshot_p95_ms": 2000.0,
    "snapshot_max_ms": 30000.0,
    "op_p95_ms": 5000.0,  # local operations
    "op_max_ms": 60000.0,
    "net_op_max_ms": 600000.0,  # fetch, pull, push
    "refresh_lag_p95_s": 5.0,
    "refresh_missed": 0.0,
    "frame_hitch_share_pct": 1.0,  # frames over 33 ms
    "frame_max_ms": 100.0,
}
NETWORK_OPS_EXACT = ("Fetching", "Pulling", "Pushing", "Force pushing")
SECOND_WORDS = {"out", "hunk", "lines", "stash", "tag", "remote", "onto", "pushing"}
FRAME_LOAD_PHASE_S = 30.0  # frame stats in the first seconds are loading, not interaction
REFRESH_ACTIONS = {"commit", "checkout", "tag", "branch", "ref-delete", "deep-checkout"}
REFRESH_WINDOW_S = 30.0

TS_RE = re.compile(
    r"^(\d{4})-(\d\d)-(\d\d)[T ](\d\d):(\d\d):(\d\d)(?:\.(\d+))?\s*(Z|[+-]\d\d:?\d\d)?$"
)


def parse_ts(text):
    """RFC 3339 timestamp -> seconds since the epoch (float), or None."""
    m = TS_RE.match(text or "")
    if not m:
        return None
    y, mo, d, h, mi, s, frac, tz = m.groups()
    t = calendar.timegm((int(y), int(mo), int(d), int(h), int(mi), int(s), 0, 0, 0))
    if frac:
        t += float("0." + frac)
    if tz and tz != "Z":
        sign = 1 if tz[0] == "+" else -1
        digits = tz[1:].replace(":", "")
        t -= sign * (int(digits[:2]) * 3600 + int(digits[2:]) * 60)
    return t


def pct(sorted_values, p):
    """Linear-interpolated percentile of an already sorted list (p in 0..100)."""
    if not sorted_values:
        return None
    if len(sorted_values) == 1:
        return sorted_values[0]
    k = (len(sorted_values) - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, len(sorted_values) - 1)
    return sorted_values[lo] + (sorted_values[hi] - sorted_values[lo]) * (k - lo)


class Dist:
    """A list of numbers with the summary the report prints."""

    def __init__(self):
        self.values = []

    def add(self, v):
        self.values.append(float(v))

    def __len__(self):
        return len(self.values)

    def stats(self):
        v = sorted(self.values)
        if not v:
            return None
        return {
            "n": len(v),
            "p50": pct(v, 50),
            "p95": pct(v, 95),
            "p99": pct(v, 99),
            "max": v[-1],
            "sum": sum(v),
        }


def num(v, digits=0):
    if v is None:
        return "-"
    if digits == 0:
        return f"{v:,.0f}"
    return f"{v:,.{digits}f}"


def short_repo(path):
    if not path:
        return "(no repository)"
    parts = [p for p in path.replace("\\", "/").split("/") if p]
    return "/".join(parts[-2:])


def head(text, n=110):
    text = " ".join(str(text).split())
    return text if len(text) <= n else text[: n - 1] + "..."


def section(title):
    print()
    print(title)
    print("-" * len(title))


def table(rows, header, widths=None):
    """Print rows (lists of strings) under header with right-aligned numeric columns."""
    allrows = [header] + rows
    if widths is None:
        widths = [max(len(str(r[i])) for r in allrows) for i in range(len(header))]
    for n, r in enumerate(allrows):
        cells = []
        for i, c in enumerate(r):
            cells.append(str(c).ljust(widths[i]) if i == 0 else str(c).rjust(widths[i]))
        print("  " + "  ".join(cells))
        if n == 0:
            print("  " + "  ".join("-" * w for w in widths))


# --------------------------------------------------------------------------------------
# The JSON-lines log
# --------------------------------------------------------------------------------------
class LogData:
    def __init__(self):
        self.lines = 0
        self.bad_lines = 0
        self.start = None  # epoch of `canopy started`
        self.first_ts = None
        self.last_ts = None
        self.version = ""
        self.levels = Counter()
        self.problems = Counter()  # (level, target, summary) -> count
        self.problem_example = {}
        self.panics = 0
        self.log_capped = False
        self.cannot_open = Counter()
        self.jobs = defaultdict(Dist)  # kind -> ms
        self.snapshots = Dist()
        self.snapshot_commits = Dist()
        self.ops = defaultdict(Dist)  # verb -> ms (all outcomes)
        self.op_fail = Counter()
        self.op_ok = Counter()
        self.git = defaultdict(Dist)  # command -> ms
        self.git_fail = Counter()
        self.repo_busy = defaultdict(float)  # repo -> ms
        self.repo_events = Counter()
        self.repo_max = {}  # repo -> (ms, what)
        self.repo_slow = Counter()  # events over 1 s
        self.repo_problems = Counter()
        self.first_snapshot = {}  # repo -> epoch
        self.first_shown = {}  # repo -> epoch of "loading the tab shown for the first time"
        self.snapshot_times = defaultdict(list)  # repo -> sorted epochs
        self.open_ops = {}  # (repo, label) -> epoch started
        self.frames = []  # (epoch, dict)
        self.first_frame_ms = None
        self.tabs_loaded_event = None
        self.event_times = []

    def note_repo_time(self, repo, ms, what):
        self.repo_busy[repo] += ms
        if ms >= 1000:
            self.repo_slow[repo] += 1
        cur = self.repo_max.get(repo)
        if cur is None or ms > cur[0]:
            self.repo_max[repo] = (ms, what)


def as_float(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def read_log(path, t_from, t_until):
    d = LogData()
    # First pass collects everything with timestamps; the window is applied by timestamp, relative
    # to the run's start, which is only known once `canopy started` is seen, so buffer events.
    events = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            d.lines += 1
            try:
                obj = json.loads(line)
            except ValueError:
                d.bad_lines += 1
                continue
            if not isinstance(obj, dict):
                d.bad_lines += 1
                continue
            fields = obj.get("fields")
            if isinstance(fields, dict):
                for k, v in fields.items():
                    obj.setdefault(k, v)
            ts = parse_ts(obj.get("timestamp"))
            events.append((ts, obj))
            if ts is not None:
                d.first_ts = ts if d.first_ts is None else min(d.first_ts, ts)
                d.last_ts = ts if d.last_ts is None else max(d.last_ts, ts)
            if d.start is None and obj.get("message") == "canopy started" and ts is not None:
                d.start = ts
                d.version = str(obj.get("version", ""))
    if d.start is None:
        d.start = d.first_ts
    lo = d.start + t_from if (d.start is not None and t_from is not None) else None
    hi = d.start + t_until if (d.start is not None and t_until is not None) else None

    for ts, obj in events:
        if ts is not None:
            if lo is not None and ts < lo:
                continue
            if hi is not None and ts > hi:
                continue
            d.event_times.append(ts)
        level = str(obj.get("level", "")).upper()
        msg = str(obj.get("message", ""))
        target = str(obj.get("target", ""))
        span = obj.get("span") if isinstance(obj.get("span"), dict) else {}
        repo = span.get("path") if span.get("name") == "repo" else None
        d.levels[level] += 1
        if repo:
            d.repo_events[repo] += 1
        ms = as_float(obj.get("ms"))

        if "log size limit reached" in msg:
            d.log_capped = True
        if level == "ERROR" and ("crash report written" in msg or "panicked" in msg):
            d.panics += 1

        if level in ("WARN", "ERROR"):
            if msg == "operation failed":
                words = str(obj.get("operation", "?")).split(" ")
                key = "operation failed: " + " ".join(words[:2] if len(words) > 1 and words[1] in SECOND_WORDS else words[:1])
                detail = obj.get("error", "")
            elif msg == "git failed":
                key = f"git failed: {obj.get('command', '?')}"
                detail = obj.get("stderr", "")
            else:
                key = head(msg, 80)
                detail = obj.get("error", "")
            d.problems[(level, target, key)] += 1
            d.problem_example.setdefault((level, target, key), head(detail))
            if repo:
                d.repo_problems[repo] += 1
            if msg == "cannot open the repository":
                d.cannot_open[repo or "?"] += 1

        if msg == "job done" and ms is not None:
            kind = str(obj.get("job", "?"))
            if kind != "operation":  # operations have their own table
                d.jobs[kind].add(ms)
            if repo:
                d.note_repo_time(repo, ms, f"job {kind}")
        elif msg == "snapshot" and ms is not None:
            d.snapshots.add(ms)
            c = as_float(obj.get("commits"))
            if c is not None:
                d.snapshot_commits.add(c)
            if repo:
                d.note_repo_time(repo, ms, "snapshot")
                if ts is not None:
                    d.first_snapshot.setdefault(repo, ts)
                    d.snapshot_times[repo].append(ts)
        elif msg == "loading the tab shown for the first time" and ts is not None:
            # Logged by the UI thread (no repo span): the folder is a field. A tab loads when it
            # is first shown (#62), so its load time runs from here, not from process start.
            shown = obj.get("path")
            if shown:
                d.first_shown.setdefault(str(shown), ts)
        elif msg == "operation started":
            if repo and ts is not None:
                d.open_ops[(repo, str(obj.get("operation", "?")))] = ts
        elif msg in ("operation succeeded", "operation failed"):
            label = str(obj.get("operation", "?"))
            words = label.split(" ")
            verb = words[0] if label else "?"
            if len(words) > 1 and words[1] in SECOND_WORDS:
                verb = f"{words[0]} {words[1]}"
            if ms is not None:
                d.ops[verb].add(ms)
                if repo:
                    d.note_repo_time(repo, ms, f"op {verb}")
            (d.op_ok if msg == "operation succeeded" else d.op_fail)[verb] += 1
            if repo:
                d.open_ops.pop((repo, label), None)
        elif msg in ("git finished", "git failed") and ms is not None:
            # "git succeeded" repeats "git finished" for streaming commands: not counted twice.
            cmd = str(obj.get("command", "?"))
            d.git[cmd].add(ms)
            if msg == "git failed":
                d.git_fail[cmd] += 1
        elif msg == "frame stats" and ts is not None:
            d.frames.append((ts, obj))
        elif msg == "first frame" and ms is not None:
            d.first_frame_ms = ms
        elif msg == "tabs loaded":
            d.tabs_loaded_event = (ts, obj)

    for v in d.snapshot_times.values():
        v.sort()
    d.event_times.sort()
    return d


# --------------------------------------------------------------------------------------
# The sampler CSV
# --------------------------------------------------------------------------------------
def read_csv(path):
    rows = []
    with open(path, newline="", encoding="utf-8", errors="replace") as f:
        for r in csv.DictReader(f):
            row = {}
            for k, v in r.items():
                row[k] = as_float(v) if v not in (None, "") else None
            if row.get("ts") is not None:
                rows.append(row)
    return rows


def series(rows, key):
    return [(r["ts"], r[key]) for r in rows if r.get(key) is not None]


def median(values):
    return statistics.median(values) if values else None


def quiet_windows(event_times, t0, t1, gap):
    """Intervals (start, end) with no log event for at least `gap` seconds."""
    pts = [t0] + [t for t in event_times if t0 <= t <= t1] + [t1]
    out = []
    for a, b in zip(pts, pts[1:]):
        if b - a >= gap:
            out.append((a, b))
    return out


# --------------------------------------------------------------------------------------
# Churn log
# --------------------------------------------------------------------------------------
def read_churn(path):
    rows = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            p = line.split()
            if len(p) < 6:
                continue
            try:
                rows.append(
                    {
                        "ts": int(p[0]) / 1000.0,
                        "repo": p[1],
                        "action": p[2],
                        "rc": int(p[3]),
                        "ms": float(p[4]),
                        "note": p[5],
                    }
                )
            except ValueError:
                continue
    return rows


def read_meta(path):
    meta = {}
    if path and os.path.exists(path):
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if "=" in line:
                    k, v = line.rstrip("\n").split("=", 1)
                    meta[k] = v
    return meta


# --------------------------------------------------------------------------------------
# The report
# --------------------------------------------------------------------------------------
class Verdict:
    def __init__(self):
        self.items = []

    def add(self, name, status, value, limit=""):
        self.items.append((name, status, value, limit))

    def check(self, name, ok, value, limit=""):
        self.add(name, "PASS" if ok else "FAIL", value, limit)

    def na(self, name, why):
        self.add(name, "n/a", why, "")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--run", help="run folder made by `tools/stress.sh open`")
    ap.add_argument("--log", help="Canopy JSON-lines log")
    ap.add_argument("--csv", help="sampler CSV")
    ap.add_argument("--churn", help="action log of `tools/stress.sh churn`")
    ap.add_argument("--meta", help="meta.txt of the run")
    ap.add_argument("--top", type=int, default=10, help="rows in the 'slowest' tables (10)")
    ap.add_argument("--expect-repos", type=int, help="number of tabs opened (default: meta.txt)")
    ap.add_argument("--from", dest="t_from", type=float, help="only seconds >= this after start")
    ap.add_argument("--until", dest="t_until", type=float, help="only seconds <= this after start")
    ap.add_argument("--idle-from", type=float, help="idle period start, seconds after start")
    ap.add_argument("--idle-to", type=float, help="idle period end, seconds after start")
    ap.add_argument("--quiet-gap", type=float, default=20.0,
                    help="log silence (s) that counts as idle when --idle-from/to is not given")
    ap.add_argument("--warmup", type=float, default=300.0,
                    help="seconds ignored before judging memory and thread growth (300)")
    ap.add_argument("--limit", action="append", default=[], metavar="NAME=VALUE",
                    help="override a pass criterion; names: " + ", ".join(sorted(LIMITS)))
    ap.add_argument("--allow", action="append", default=[], metavar="REGEX",
                    help="warnings whose summary matches are expected (e.g. 'operation failed: Merging'); "
                         "any other WARN is marked REVIEW in the verdict")
    ap.add_argument("--json", metavar="FILE", help="also write the summary as JSON")
    args = ap.parse_args()

    for item in args.limit:
        name, _, value = item.partition("=")
        if name not in LIMITS or as_float(value) is None:
            ap.error(f"bad --limit {item!r}; names: {', '.join(sorted(LIMITS))}")
        LIMITS[name] = float(value)

    log_path, csv_path, churn_path, meta_path = args.log, args.csv, args.churn, args.meta
    if args.run:
        run = args.run
        log_path = log_path or os.path.join(run, "canopy.jsonl")
        csv_path = csv_path or os.path.join(run, "samples.csv")
        meta_path = meta_path or os.path.join(run, "meta.txt")
        if not churn_path:
            found = sorted(glob.glob(os.path.join(run, "churn-*.log")))
            churn_path = found[-1] if found else None
    if not log_path and not csv_path:
        ap.error("give --run, or --log and/or --csv")
    if args.run:  # a run folder may lack the log (no log file was found) or the samples
        if log_path and not os.path.exists(log_path):
            print(f"stress_report: no log in {args.run}; reporting resources only", file=sys.stderr)
            log_path = None
        if csv_path and not os.path.exists(csv_path):
            print(f"stress_report: no samples in {args.run}; reporting the log only", file=sys.stderr)
            csv_path = None
    for p in (log_path, csv_path):
        if p and not os.path.exists(p):
            print(f"stress_report: no such file: {p}", file=sys.stderr)
            return 2
    if not log_path and not csv_path:
        print("stress_report: nothing to report", file=sys.stderr)
        return 2

    meta = read_meta(meta_path)
    verdict = Verdict()
    summary = {}

    log = read_log(log_path, args.t_from, args.t_until) if log_path else None
    rows_all = read_csv(csv_path) if csv_path else []
    t0 = None
    if log is not None and log.start is not None:
        t0 = log.start
    elif rows_all:
        t0 = rows_all[0]["ts"]
    rows = rows_all
    if t0 is not None:
        lo = t0 + args.t_from if args.t_from is not None else -1e18
        hi = t0 + args.t_until if args.t_until is not None else 1e18
        rows = [r for r in rows_all if lo <= r["ts"] <= hi]

    expect = args.expect_repos or (int(meta["repos"]) if meta.get("repos", "").isdigit() else None)

    # ---- header ------------------------------------------------------------------------
    print("Canopy stress-test report")
    print("=" * 25)
    if meta:
        for key in ("label", "date_utc", "canopy_version", "cpu", "cpu_threads", "mem_total_kb",
                    "os", "kernel", "cache_fs", "repos", "canopy_log", "exit_code"):
            if key in meta and meta[key]:
                print(f"  {key:<16} {meta[key]}")
    if log is not None:
        span = (log.last_ts - log.start) if (log.last_ts and log.start) else 0
        print(f"  log              {log_path}: {log.lines:,} lines, {span / 60:.1f} min, "
              f"{log.bad_lines} unreadable")
        if args.t_from is not None or args.t_until is not None:
            print(f"  window           {args.t_from or 0:g} s to "
                  f"{('end' if args.t_until is None else format(args.t_until, 'g') + ' s')} after start")
    if rows_all:
        print(f"  samples          {csv_path}: {len(rows_all):,} rows")

    # ---- errors ------------------------------------------------------------------------
    if log is not None:
        section("Warnings and errors")
        errors = log.levels.get("ERROR", 0)
        warns = log.levels.get("WARN", 0)
        print(f"  ERROR {errors}   WARN {warns}   panics/crash reports logged {log.panics}")
        if log.log_capped:
            print("  !! the log reached its size limit (20 MiB): later events are missing")
        if log.problems:
            rowsp = []
            for (level, target, key), n in log.problems.most_common(15):
                rowsp.append([level, n, head(key, 60), head(log.problem_example.get((level, target, key), ""), 70)])
            table(rowsp, ["level", "count", "what", "first detail"])
        verdict.check("no ERROR events", errors == 0, errors, "0")
        allowed = [re.compile(a) for a in args.allow]
        unexpected = sum(
            n for (level, target, key), n in log.problems.items()
            if level == "WARN" and not any(a.search(key) for a in allowed)
        )
        verdict.add("no unexpected warnings", "PASS" if unexpected == 0 else "REVIEW", unexpected,
                    "0; read the table above, then use --allow for the expected ones")
        verdict.check("no panics", log.panics == 0, log.panics, "0")
        verdict.check("log not truncated", not log.log_capped, "capped" if log.log_capped else "complete")
        verdict.check("every repository opened", not log.cannot_open,
                      f"{len(log.cannot_open)} failed", "0")

    crashes = meta_path and os.path.join(os.path.dirname(meta_path), "crashes.txt")
    if crashes and os.path.exists(crashes):
        with open(crashes, encoding="utf-8", errors="replace") as f:
            files = [l.strip() for l in f if l.strip()]
        print(f"  crash report files: {len(files)}")
        for p in files[:5]:
            print(f"    {p}")
        verdict.check("no crash report files", not files, len(files), "0")
    code = meta.get("exit_code")
    if code is not None and code.isdigit():
        ok = int(code) in (0, 143)  # 143: stopped by the harness's --duration
        verdict.check("Canopy exited cleanly", ok, f"exit {code}", "0 (or 143 when stopped by --duration)")

    # ---- load --------------------------------------------------------------------------
    if log is not None:
        section("Loading")
        if log.first_snapshot and log.start:
            # Each tab's load time runs from when it was first shown (a log without that event
            # falls back to the process start, as before lazy tabs).
            def load_s(repo, t):
                return max(0.0, t - log.first_shown.get(repo, log.start))

            times = sorted(load_s(r, t) for r, t in log.first_snapshot.items())
            print(f"  tabs with a first snapshot: {len(times)}" + (f" of {expect}" if expect else ""))
            print(f"  first snapshot after the tab was first shown: p50 {pct(times, 50):.1f} s   "
                  f"p95 {pct(times, 95):.1f} s   slowest {times[-1]:.1f} s")
            slow = sorted(((load_s(r, t), r) for r, t in log.first_snapshot.items()), reverse=True)[: args.top]
            print("  slowest to load:")
            for t, r in slow:
                print(f"    {t:8.1f} s  {short_repo(r)}")
            all_loaded = (expect is None or len(times) >= expect)
            # Canopy loads a tab when it is first shown (#62): a run that never showed some tabs
            # fails here by design (docs/stress-test.md, scenario (a)).
            verdict.check("all tabs loaded (each tab shown once)", all_loaded,
                          f"{len(times)}" + (f"/{expect}" if expect else ""), str(expect or "all"))
            verdict.check("slowest tab loaded in time", times[-1] <= LIMITS["all_loaded_s"],
                          f"{times[-1]:.1f} s", f"<= {LIMITS['all_loaded_s']:g} s")
            verdict.check("95% of tabs loaded in time", pct(times, 95) <= LIMITS["repo_loaded_s"],
                          f"p95 {pct(times, 95):.1f} s", f"<= {LIMITS['repo_loaded_s']:g} s")
            summary["load_last_s"] = times[-1]
        else:
            print("  no `snapshot` events: run with the Detailed log level (see docs/stress-test.md)")
            verdict.na("all tabs loaded", "no snapshot events (log level too low?)")
        if log.first_frame_ms is not None:
            print(f"  first frame: {log.first_frame_ms:.0f} ms after process start")
            verdict.check("first frame", log.first_frame_ms <= LIMITS["first_frame_ms"],
                          f"{log.first_frame_ms:.0f} ms", f"<= {LIMITS['first_frame_ms']:g} ms")
        else:
            print("  first frame: no `first frame` event (an older build, or the run's window starts after it)")
            verdict.na("first frame", "no `first frame` event in the log")
        if log.tabs_loaded_event:
            print(f"  `tabs loaded` event: {head(json.dumps(log.tabs_loaded_event[1]), 120)}")

    # ---- durations ---------------------------------------------------------------------
    if log is not None:
        section("Repository jobs (ms)   [message `job done`]")
        rowsj = []
        for kind, dist in sorted(log.jobs.items(), key=lambda kv: -(kv[1].stats() or {"sum": 0})["sum"]):
            s = dist.stats()
            rowsj.append([kind, s["n"], num(s["p50"]), num(s["p95"]), num(s["p99"]), num(s["max"]), num(s["sum"] / 1000, 1)])
            if kind != "status" and kind != "refresh":
                verdict.check(f"job {kind}: p95", s["p95"] <= LIMITS["job_p95_ms"], f"{s['p95']:.0f} ms",
                              f"<= {LIMITS['job_p95_ms']:g}")
                verdict.check(f"job {kind}: max", s["max"] <= LIMITS["job_max_ms"], f"{s['max']:.0f} ms",
                              f"<= {LIMITS['job_max_ms']:g}")
        if rowsj:
            table(rowsj, ["job", "n", "p50", "p95", "p99", "max", "total s"])
        else:
            print("  none (needs Detailed log level)")
        sn = log.snapshots.stats()
        if sn:
            print(f"  snapshot (history refresh): n {sn['n']}  p50 {num(sn['p50'])}  p95 {num(sn['p95'])}  "
                  f"p99 {num(sn['p99'])}  max {num(sn['max'])} ms; commits per snapshot "
                  f"max {num((log.snapshot_commits.stats() or {}).get('max'))}")
            verdict.check("snapshot p95", sn["p95"] <= LIMITS["snapshot_p95_ms"], f"{sn['p95']:.0f} ms",
                          f"<= {LIMITS['snapshot_p95_ms']:g}")
            verdict.check("snapshot max", sn["max"] <= LIMITS["snapshot_max_ms"], f"{sn['max']:.0f} ms",
                          f"<= {LIMITS['snapshot_max_ms']:g}")

        section("Operations (ms)   [messages `operation succeeded` / `operation failed`]")
        rowso = []
        for verb, dist in sorted(log.ops.items(), key=lambda kv: -(kv[1].stats() or {"max": 0})["max"]):
            s = dist.stats()
            rowso.append([verb, log.op_ok[verb], log.op_fail[verb], num(s["p50"]), num(s["p95"]), num(s["max"])])
            net = verb in NETWORK_OPS_EXACT
            if net:
                verdict.check(f"operation {verb}: max", s["max"] <= LIMITS["net_op_max_ms"],
                              f"{s['max'] / 1000:.1f} s", f"<= {LIMITS['net_op_max_ms'] / 1000:g} s")
            else:
                verdict.check(f"operation {verb}: p95", s["p95"] <= LIMITS["op_p95_ms"], f"{s['p95']:.0f} ms",
                              f"<= {LIMITS['op_p95_ms']:g}")
                verdict.check(f"operation {verb}: max", s["max"] <= LIMITS["op_max_ms"], f"{s['max']:.0f} ms",
                              f"<= {LIMITS['op_max_ms']:g}")
        if rowso:
            table(rowso, ["operation", "ok", "failed", "p50", "p95", "max"])
        else:
            print("  none")
        end_ts = max(log.last_ts or 0, rows_all[-1]["ts"] if rows_all else 0)
        unfinished = []
        for (repo, label), t in log.open_ops.items():
            age = end_ts - t
            net = label.split(" ")[0] in ("Fetching", "Pulling", "Pushing", "Force")
            unfinished.append((age, repo, label, age > (LIMITS["net_op_max_ms"] if net else 60000.0) / 1000.0))
        unfinished.sort(reverse=True)
        if unfinished:
            print(f"  operations started and not finished when the run ended: {len(unfinished)}")
            for age, repo, label, hung in unfinished[:10]:
                print(f"    {short_repo(repo)}: {label} ({age:.0f} s{'  HUNG' if hung else ''})")
        hung_n = sum(1 for u in unfinished if u[3])
        verdict.check("no hung operations", hung_n == 0, f"{hung_n} hung, {len(unfinished)} unfinished",
                      "0 (local > 60 s, network > 600 s)")

        section("git commands (ms)   [messages `git finished` / `git failed`]")
        rowsg = []
        for cmd, dist in sorted(log.git.items(), key=lambda kv: -(kv[1].stats() or {"sum": 0})["sum"])[:12]:
            s = dist.stats()
            rowsg.append([cmd, s["n"], log.git_fail[cmd], num(s["p50"]), num(s["p95"]), num(s["max"]), num(s["sum"] / 1000, 1)])
        if rowsg:
            table(rowsg, ["command", "n", "failed", "p50", "p95", "max", "total s"])
        else:
            print("  none (needs Detailed log level)")

        section(f"Slowest repositories (top {args.top} by time spent in jobs, snapshots and operations)")
        busy = sorted(log.repo_busy.items(), key=lambda kv: -kv[1])[: args.top]
        rowsr = []
        for repo, ms in busy:
            mx = log.repo_max.get(repo, (0, ""))
            rowsr.append([short_repo(repo), num(ms / 1000, 1), log.repo_events[repo], num(mx[0]), mx[1],
                          log.repo_slow[repo], log.repo_problems[repo]])
        if rowsr:
            table(rowsr, ["repository", "busy s", "events", "max ms", "max was", ">1 s", "warn/err"])
        else:
            print("  none")
        if expect and len(log.repo_events) < expect:
            print(f"  only {len(log.repo_events)} of {expect} repositories logged anything")

        # frames: DEBUG `frame stats` events, about every 5 s while frames are produced
        section("Frame times   [message `frame stats`]")
        if not log.frames:
            print("  none: no `frame stats` events (they are DEBUG: run with the Detailed log level, see")
            print("  docs/stress-test.md; an idle app produces no frames and so no events).")
            verdict.na("frame times", "no `frame stats` events (log level too low, or no frames)")
        else:
            total = hitches = 0
            load_frames = load_hitches = 0
            load_worst = 0.0
            worst = []
            for ts, o in log.frames:
                n = as_float(o.get("frames")) or 0
                h33 = as_float(o.get("hitches_33")) or 0
                mx = as_float(o.get("max_ms")) or 0
                if log.start and ts - log.start < FRAME_LOAD_PHASE_S:
                    load_frames += n
                    load_hitches += h33
                    load_worst = max(load_worst, mx)
                elif log.start:
                    total += n
                    hitches += h33
                    worst.append((mx, ts - log.start, o))
            worst.sort(key=lambda w: -w[0])
            share = 100.0 * hitches / total if total else 0.0
            print(f"  loading (first {FRAME_LOAD_PHASE_S:g} s, left out of the table and the verdicts): "
                  f"{int(load_frames):,} frames, {int(load_hitches):,} over 33 ms, worst {load_worst:.0f} ms")
            print(f"  {int(total):,} frames after the first {FRAME_LOAD_PHASE_S:g} s; "
                  f"{int(hitches):,} over 33 ms ({share:.2f}%)")
            rowsf = []
            for mx, t, o in worst[: args.top]:
                rowsf.append([f"{t:.0f}", num(as_float(o.get("frames"))), num(as_float(o.get("p50_ms")), 1),
                              num(as_float(o.get("p95_ms")), 1), num(as_float(o.get("p99_ms")), 1), num(mx, 1),
                              num(as_float(o.get("hitches_33"))), head(o.get("context", ""), 20)])
            table(rowsf, ["at s", "frames", "p50", "p95", "p99", "max", ">33ms", "context"])
            if total:
                verdict.check("frames over 33 ms", share <= LIMITS["frame_hitch_share_pct"], f"{share:.2f}%",
                              f"<= {LIMITS['frame_hitch_share_pct']:g}%")
                verdict.check("worst frame", worst[0][0] <= LIMITS["frame_max_ms"], f"{worst[0][0]:.0f} ms",
                              f"<= {LIMITS['frame_max_ms']:g} ms")
            print("  (scrolling p95 <= 16.7 ms: use --from/--until on the scrolling minutes and read the p95 column)")

    # ---- resources ---------------------------------------------------------------------
    if rows:
        section("Resources   [sampler CSV]")
        base = t0 if t0 is not None else rows[0]["ts"]
        dur = rows[-1]["ts"] - rows[0]["ts"]
        gaps = [b["ts"] - a["ts"] for a, b in zip(rows, rows[1:])]
        print(f"  {len(rows):,} samples over {dur / 60:.1f} min; longest gap between samples "
              f"{max(gaps) if gaps else 0:.1f} s")

        def peak(key, scale=1.0):
            s = series(rows, key)
            if not s:
                return None
            t, v = max(s, key=lambda x: x[1])
            return v / scale, max(0.0, t - base)

        for key, label, scale, unit in (
            ("rss_kb", "RSS", 1024.0, "MiB"),
            ("rss_anon_kb", "RSS heap (anon)", 1024.0, "MiB"),
            ("rss_file_kb", "RSS mapped files", 1024.0, "MiB"),
            ("threads", "threads", 1.0, ""),
            ("fds", "open descriptors", 1.0, ""),
            ("children", "child processes (git)", 1.0, ""),
            ("inotify_fds", "inotify instances", 1.0, ""),
            ("inotify_watches", "inotify watches", 1.0, ""),
        ):
            p = peak(key, scale)
            if p:
                print(f"  peak {label:<22} {p[0]:>10,.0f} {unit:<4} at {p[1]:.0f} s")
        avail = series(rows, "mem_avail_kb")
        if avail and meta.get("mem_total_kb", "").isdigit():
            low = min(v for _, v in avail)
            print(f"  lowest MemAvailable        {low / 1024:>10,.0f} MiB of {int(meta['mem_total_kb']) / 1024:,.0f} MiB")
        cpu = [v for _, v in series(rows, "cpu_pct")]
        if cpu:
            cs = sorted(cpu)
            print(f"  CPU (100 = one core)       p50 {pct(cs, 50):.1f}   p95 {pct(cs, 95):.1f}   max {cs[-1]:.1f}")

        # peaks
        rss_peak = peak("rss_kb", 1024.0)
        if rss_peak:
            verdict.check("peak RSS", rss_peak[0] <= LIMITS["rss_peak_mib"], f"{rss_peak[0]:.0f} MiB",
                          f"<= {LIMITS['rss_peak_mib']:g} MiB")
        thr_peak = peak("threads")
        if thr_peak:
            verdict.check("peak threads", thr_peak[0] <= LIMITS["threads_peak"], f"{thr_peak[0]:.0f}",
                          f"<= {LIMITS['threads_peak']:g}")
        fd_peak = peak("fds")
        nofile = as_float(meta.get("ulimit_nofile_soft"))
        if fd_peak and nofile:
            lim = nofile * LIMITS["fds_frac_of_limit"]
            verdict.check("peak descriptors", fd_peak[0] <= lim, f"{fd_peak[0]:.0f} of {nofile:.0f}",
                          f"<= {LIMITS['fds_frac_of_limit'] * 100:.0f}% of the limit")
        elif fd_peak:
            verdict.na("peak descriptors", "meta.txt has no ulimit_nofile_soft")
        ifd = peak("inotify_fds")
        if ifd and expect:
            verdict.check("a file watcher per tab", ifd[0] >= expect, f"{ifd[0]:.0f} instances for {expect} tabs",
                          ">= tabs (a missing one means a watcher failed silently)")
        iw = peak("inotify_watches")
        wlim = as_float(meta.get("inotify_max_user_watches"))
        if iw and wlim:
            verdict.check("inotify watches", iw[0] <= wlim * LIMITS["watches_frac_of_limit"],
                          f"{iw[0]:.0f} of {wlim:.0f}", f"<= {LIMITS['watches_frac_of_limit'] * 100:.0f}% of the limit")

        # memory after load and growth
        loaded_at = None
        if log is not None and log.first_snapshot and log.start:
            loaded_at = max(log.first_snapshot.values())
        if loaded_at is not None:
            after = [v for t, v in series(rows, "rss_kb") if loaded_at < t <= loaded_at + 60]
            if after:
                m = median(after) / 1024
                print(f"  RSS in the minute after all tabs loaded: {m:,.0f} MiB")
                verdict.check("RSS after load", m <= LIMITS["rss_loaded_mib"], f"{m:.0f} MiB",
                              f"<= {LIMITS['rss_loaded_mib']:g} MiB")
        key = "rss_anon_kb" if series(rows, "rss_anon_kb") else "rss_kb"
        s = [(t - base, v / 1024) for t, v in series(rows, key) if t - base >= args.warmup]
        if s and s[-1][0] - s[0][0] >= 600:
            first = [v for t, v in s if t <= s[0][0] + 300]
            last = [v for t, v in s if t >= s[-1][0] - 300]
            a, b = median(first), median(last)
            allowed = a * (1 + LIMITS["rss_growth_pct"] / 100) + LIMITS["rss_growth_slack_mib"]
            print(f"  heap growth: first 5 min after warm-up {a:,.0f} MiB, last 5 min {b:,.0f} MiB "
                  f"({(b - a):+,.0f} MiB)")
            verdict.check("memory does not grow", b <= allowed, f"{a:.0f} -> {b:.0f} MiB",
                          f"<= +{LIMITS['rss_growth_pct']:g}% and {LIMITS['rss_growth_slack_mib']:g} MiB")
        else:
            verdict.na("memory does not grow", f"needs 10+ minutes after the {args.warmup:g} s warm-up")

        # threads and descriptors returning to baseline
        for key, label, lim in (("threads", "threads", "threads_growth"), ("fds", "descriptors", "fds_growth")):
            s = [(t, v) for t, v in series(rows, key)]
            if loaded_at is not None and s and s[-1][0] - loaded_at >= 300:
                basev = [v for t, v in s if loaded_at + 30 <= t <= loaded_at + 90]
                endv = [v for t, v in s if t >= s[-1][0] - 60]
                if basev and endv:
                    b0, b1 = median(basev), median(endv)
                    verdict.check(f"{label} return to baseline", b1 <= b0 + LIMITS[lim],
                                  f"{b0:.0f} -> {b1:.0f}", f"<= +{LIMITS[lim]:g}")
            else:
                verdict.na(f"{label} return to baseline", "needs 5+ minutes after all tabs loaded")

        # idle CPU
        idle_cpu = []
        if args.idle_from is not None and args.idle_to is not None:
            idle_cpu = [v for t, v in series(rows, "cpu_pct") if args.idle_from <= t - base <= args.idle_to]
            how = f"seconds {args.idle_from:g}-{args.idle_to:g}"
        elif log is not None and log.event_times:
            wins = quiet_windows(log.event_times, rows[0]["ts"], rows[-1]["ts"], args.quiet_gap)
            for a, b in wins:
                idle_cpu += [v for t, v in series(rows, "cpu_pct") if a + 2 <= t <= b - 2]
            how = f"{len(wins)} stretches with no log event for {args.quiet_gap:g}+ s"
        else:
            how = "no log to find idle stretches in"
        if idle_cpu:
            ic = sorted(idle_cpu)
            print(f"  idle CPU ({how}; {len(ic)} samples): p50 {pct(ic, 50):.1f}  p95 {pct(ic, 95):.1f}  max {ic[-1]:.1f}")
            verdict.check("idle CPU p50", pct(ic, 50) <= LIMITS["idle_cpu_median"], f"{pct(ic, 50):.1f}%",
                          f"<= {LIMITS['idle_cpu_median']:g}%")
            verdict.check("idle CPU p95", pct(ic, 95) <= LIMITS["idle_cpu_p95"], f"{pct(ic, 95):.1f}%",
                          f"<= {LIMITS['idle_cpu_p95']:g}%")
            summary["idle_cpu_p50"] = pct(ic, 50)
        else:
            print(f"  idle CPU: no idle stretch found ({how}); leave the app untouched for five minutes "
                  "after loading, or pass --idle-from/--idle-to")
            verdict.na("idle CPU", "no idle stretch in the run")

    # ---- churn -------------------------------------------------------------------------
    if churn_path and os.path.exists(churn_path):
        section(f"External churn   [{churn_path}]")
        ch = read_churn(churn_path)
        byact = defaultdict(Dist)
        fails = Counter()
        for r in ch:
            byact[r["action"]].add(r["ms"])
            if r["rc"] != 0 and r["action"] != "skip":
                fails[(r["action"], r["note"])] += 1
        rowsc = []
        for a, dist in sorted(byact.items()):
            s = dist.stats()
            rowsc.append([a, s["n"], num(s["p50"]), num(s["p95"]), num(s["max"])])
        table(rowsc, ["action", "n", "p50 ms", "p95 ms", "max ms"])
        if fails:
            print("  failed actions (a repository the app or git held locked, local changes, ...):")
            for (a, note), n in fails.most_common(8):
                print(f"    {n:5}  {a}: {head(note, 70)}")
        locks = sum(n for (a, note), n in fails.items() if note == "index-lock")
        print(f"  index.lock collisions: {locks} (git could not take the lock: another process held it)")

        # Refresh lag: time from a change under .git to the next history snapshot of that repo.
        if log is not None and log.snapshot_times and log.start:
            by_key = {}
            for repo, times in log.snapshot_times.items():
                by_key["/".join(repo.replace("\\", "/").rstrip("/").split("/")[-2:]).lower()] = times
            lags, missed = Dist(), 0
            for r in ch:
                if r["rc"] != 0 or r["action"] not in REFRESH_ACTIONS:
                    continue
                times = by_key.get(r["repo"].lower())
                if not times:
                    continue
                i = bisect.bisect_left(times, r["ts"])
                if i < len(times) and times[i] - r["ts"] <= REFRESH_WINDOW_S:
                    lags.add(times[i] - r["ts"])
                else:
                    missed += 1
            s = lags.stats()
            if s:
                print(f"  refresh lag after changes under .git: n {s['n']}  p50 {s['p50']:.2f} s  "
                      f"p95 {s['p95']:.2f} s  max {s['max']:.2f} s;  no refresh within {REFRESH_WINDOW_S:g} s: {missed}")
                verdict.check("refresh lag p95", s["p95"] <= LIMITS["refresh_lag_p95_s"], f"{s['p95']:.2f} s",
                              f"<= {LIMITS['refresh_lag_p95_s']:g} s")
                verdict.check("no missed refresh", missed <= LIMITS["refresh_missed"], missed, "0")
            else:
                verdict.na("refresh lag", "no matching snapshots (Detailed log? same cache folder?)")
        print("  (changes to working-tree files only trigger a status refresh, which the app does not log)")

    # ---- verdict -----------------------------------------------------------------------
    section("Verdict against the pass criteria")
    fails = 0
    for name, status, value, limit in verdict.items:
        fails += status == "FAIL"
        print(f"  {status:<6} {name:<44} {str(value):<28} {limit}")
    na = sum(1 for v in verdict.items if v[1] == "n/a")
    print()
    review = sum(1 for v in verdict.items if v[1] == "REVIEW")
    print(f"  {sum(1 for v in verdict.items if v[1] == 'PASS')} passed, {fails} failed, "
          f"{review} to review, {na} not measurable")
    print("  Criteria whose events are missing from the log (frame times, first frame, other instrumentation) are `n/a`.")

    if args.json:
        summary["verdict"] = [dict(name=n, status=s, value=str(v), limit=l) for n, s, v, l in verdict.items]
        summary["levels"] = dict(log.levels) if log else {}
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(summary, f, indent=2)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
