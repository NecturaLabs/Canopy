#!/usr/bin/env python3
"""Checks Translucent surfaces (the backdrop blur) from screenshots and a soak run.

Usage:
    tools/glass-check.py compare OFF.png ON.png --region X0,Y0,X1,Y1 [--region ...]
                         [--base BASE.png] [--text RRGGBB:RATIO ...] [--min-spread S]
    tools/glass-check.py soak [--frames N] [--max-growth-kb KB] [PATH-TO-CANOPY]
    tools/glass-check.py selftest

compare  OFF and ON are the same screen rendered with `--translucent off` and `on`
         (tools/screenshot.sh); each --region is a glass layer in pixels. Fails unless
         (a) every pixel outside the regions is identical in both,
         (b) what shows through each region is blurred: over the region's plain background (away
             from text and edges) the difference ON-OFF, box-averaged to remove the dither, has a
             99th-percentile Laplacian under --max-sharpness (default 0.75) and, given --base (the
             same screen without the layer), under that of BASE there; given --min-spread S, the
             difference must also vary by S grey levels, which tells the blur from an opaque fill
             where something visible lies behind (a light theme's luminance floor can flatten a
             dimmed backdrop completely),
         (c) each --text colour keeps its contrast ratio against every background pixel of the
             glass in ON (for example --text E8E9EC:7 for Midnight's text).
soak     Runs a debug build with `--glass-soak N` (default 2000) on a private Xvfb display
         (software OpenGL) twice: with Translucent surfaces on, and off as the baseline (the same
         layers, opaque). Fails unless the blur built, deleted every OpenGL object it created and
         stayed on, and resident memory grew over the measured frames (after a warm-up of one
         full cycle of the soak's layer sizes) by at most --max-growth-kb (default 2048), both in
         the glass run and beyond the baseline's growth. Builds the debug binary unless given
         one.
selftest Runs this script's own tests.

Standard library only (PNG decoding with zlib and struct).
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


# ----------------------------------------------------------------------------------- PNG


class Image:
    """An 8-bit RGB or RGBA image: `rows[y]` is a bytes-like row of `width * channels` bytes."""

    def __init__(self, width: int, height: int, channels: int, rows: list[bytes]):
        self.width = width
        self.height = height
        self.channels = channels
        self.rows = rows

    def rgb(self, x: int, y: int) -> tuple[int, int, int]:
        i = x * self.channels
        row = self.rows[y]
        return row[i], row[i + 1], row[i + 2]


def read_png(path: Path) -> Image:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path}: not a PNG")
    pos, idat = 8, bytearray()
    width = height = channels = 0
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            width, height, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if depth != 8 or color not in (2, 6) or interlace != 0:
                raise ValueError(f"{path}: only 8-bit RGB/RGBA, not interlaced")
            channels = 4 if color == 6 else 3
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    rows: list[bytes] = []
    prev = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        kind = raw[start]
        line = bytearray(raw[start + 1 : start + 1 + stride])
        unfilter(kind, line, prev, channels)
        rows.append(bytes(line))
        prev = line
    return Image(width, height, channels, rows)


def unfilter(kind: int, line: bytearray, prev: bytearray, bpp: int) -> None:
    n = len(line)
    if kind == 0:
        return
    if kind == 1:
        for i in range(bpp, n):
            line[i] = (line[i] + line[i - bpp]) & 255
    elif kind == 2:
        for i in range(n):
            line[i] = (line[i] + prev[i]) & 255
    elif kind == 3:
        for i in range(n):
            left = line[i - bpp] if i >= bpp else 0
            line[i] = (line[i] + ((left + prev[i]) >> 1)) & 255
    elif kind == 4:
        for i in range(n):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
            line[i] = (line[i] + pred) & 255
    else:
        raise ValueError(f"unknown PNG filter {kind}")


# ------------------------------------------------------------------------------- colour


def luminance(rgb: tuple[int, int, int]) -> float:
    def lin(v: int) -> float:
        s = v / 255
        return s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4

    r, g, b = (lin(v) for v in rgb)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a: tuple[int, int, int], b: tuple[int, int, int]) -> float:
    x, y = luminance(a), luminance(b)
    return (max(x, y) + 0.05) / (min(x, y) + 0.05)


def parse_hex(text: str) -> tuple[int, int, int]:
    if not re.fullmatch(r"[0-9a-fA-F]{6}", text):
        raise argparse.ArgumentTypeError(f"not a colour: {text}")
    return int(text[0:2], 16), int(text[2:4], 16), int(text[4:6], 16)


# ------------------------------------------------------------------------------- compare


Region = tuple[int, int, int, int]


def parse_region(text: str) -> Region:
    parts = text.split(",")
    if len(parts) != 4 or not all(p.strip().lstrip("-").isdigit() for p in parts):
        raise argparse.ArgumentTypeError(f"region must be X0,Y0,X1,Y1: {text}")
    x0, y0, x1, y1 = (int(p) for p in parts)
    if x1 <= x0 or y1 <= y0:
        raise argparse.ArgumentTypeError(f"empty region: {text}")
    return x0, y0, x1, y1


def outside_differences(off: Image, on: Image, regions: list[Region]) -> int:
    """Pixels outside every region that differ between the two images."""
    bad = 0
    ch = off.channels
    for y in range(off.height):
        spans = sorted((x0, x1) for x0, y0, x1, y1 in regions if y0 <= y < y1)
        a, b = off.rows[y], on.rows[y]
        if not spans:
            if a != b:
                bad += sum(1 for x in range(off.width) if a[x * ch : x * ch + 3] != b[x * ch : x * ch + 3])
            continue
        cursor = 0
        for x0, x1 in spans + [(off.width, off.width)]:
            x0 = max(0, min(off.width, x0))
            if x0 > cursor:
                sa, sb = a[cursor * ch : x0 * ch], b[cursor * ch : x0 * ch]
                if sa != sb:
                    bad += sum(
                        1 for x in range(cursor, x0) if a[x * ch : x * ch + 3] != b[x * ch : x * ch + 3]
                    )
            cursor = max(cursor, min(off.width, x1))
    return bad


def grey(img: Image, x: int, y: int) -> float:
    r, g, b = img.rgb(x, y)
    return 0.299 * r + 0.587 * g + 0.114 * b


def background_mask(off: Image, region: Region, margin: int = 4) -> list[tuple[int, int]]:
    """Pixels of the glass background in OFF: the region's most common colour, at least `margin`
    pixels from anything else (text, the ring) and from the region's edge."""
    x0, y0, x1, y1 = region
    counts: dict[tuple[int, int, int], int] = {}
    for y in range(y0, y1):
        for x in range(x0, x1):
            c = off.rgb(x, y)
            counts[c] = counts.get(c, 0) + 1
    ground = max(counts, key=counts.get)
    plain = [[off.rgb(x, y) == ground for x in range(x0, x1)] for y in range(y0, y1)]
    w, h = x1 - x0, y1 - y0
    # Erode by `margin` with running minima along rows, then columns.
    def erode_rows(grid):
        out = []
        for row in grid:
            bad = [i for i, v in enumerate(row) if not v]
            line = [True] * len(row)
            for i in bad:
                for j in range(max(0, i - margin), min(len(row), i + margin + 1)):
                    line[j] = False
            out.append(line)
        return out

    eroded = erode_rows(plain)
    columns = [[eroded[y][x] for y in range(h)] for x in range(w)]
    columns = erode_rows(columns)
    return [
        (x0 + x, y0 + y)
        for x in range(margin, w - margin)
        for y in range(margin, h - margin)
        if columns[x][y]
    ]


def smooth_residual(off: Image | None, on: Image, region: Region, mask: list[tuple[int, int]]) -> dict:
    """What the blur left in the region's background: the difference ON-OFF, box-averaged over
    5 x 5 pixels to remove the dither. Its spread (2nd to 98th percentile) is near zero when the
    glass is an opaque fill and grows with what shows through; its discrete Laplacian (99th
    percentile) stays small only when what shows through is blurred. Without OFF, the same for ON
    itself."""
    x0, y0, x1, y1 = region
    diff = {}
    for y in range(y0, y1):
        for x in range(x0, x1):
            diff[(x, y)] = grey(on, x, y) - (grey(off, x, y) if off else 0.0)

    def box(x: int, y: int) -> float | None:
        total = 0.0
        for dy in range(-2, 3):
            for dx in range(-2, 3):
                v = diff.get((x + dx, y + dy))
                if v is None:
                    return None
                total += v
        return total / 25

    inside = set(mask)
    levels, laplacians = [], []
    for x, y in mask:
        if not all((x + dx, y + dy) in inside for dx, dy in ((-3, 0), (3, 0), (0, -3), (0, 3))):
            continue
        c, l, r, u, d = box(x, y), box(x - 1, y), box(x + 1, y), box(x, y - 1), box(x, y + 1)
        if None in (c, l, r, u, d):
            continue
        levels.append(c)
        laplacians.append(abs(l + r + u + d - 4 * c))
    levels.sort()
    laplacians.sort()
    n = len(levels)
    spread = levels[int(n * 0.98)] - levels[int(n * 0.02)] if n else 0.0
    p99 = laplacians[int(n * 0.99)] if n else 0.0
    return {"pixels": len(mask), "samples": n, "spread": spread, "laplacian_p99": p99}


def compare(args: argparse.Namespace) -> int:
    off, on = read_png(Path(args.off)), read_png(Path(args.on))
    base = read_png(Path(args.base)) if args.base else None
    if any((img.width, img.height) != (on.width, on.height) for img in (off, base) if img):
        print("glass-check: the images differ in size", file=sys.stderr)
        return 1
    failures = []
    outside = outside_differences(off, on, args.region)
    print(f"(a) pixels outside the glass that differ: {outside}")
    if outside:
        failures.append("pixels outside the glass regions changed")
    for region in args.region:
        mask = background_mask(off, region)
        stats = smooth_residual(off, on, region, mask)
        print(
            f"(b) region {region}: {stats['pixels']} background pixels ({stats['samples']} sampled), "
            f"spread {stats['spread']:.2f}, sharpness {stats['laplacian_p99']:.3f}"
        )
        if stats["samples"] < 100:
            failures.append(f"region {region}: too little plain background to judge")
        elif stats["spread"] < args.min_spread:
            failures.append(f"region {region}: nothing shows through the glass (an opaque fill?)")
        if stats["laplacian_p99"] > args.max_sharpness:
            failures.append(f"region {region}: detail behind the glass is still sharp")
        if base:
            behind = smooth_residual(None, base, region, mask)["laplacian_p99"]
            print(f"(b) region {region}: sharpness of the content behind it without the layer {behind:.3f}")
            if stats["laplacian_p99"] >= behind:
                failures.append(f"region {region}: no sharper behind the glass than through it")
        for colour, ratio in args.text:
            worst = min((contrast(colour, on.rgb(x, y)) for x, y in mask), default=0.0)
            print(f"(c) region {region}: #{colour[0]:02X}{colour[1]:02X}{colour[2]:02X} at least {worst:.2f}:1 (needs {ratio}:1)")
            if worst < ratio:
                failures.append(f"region {region}: text contrast {worst:.2f} under {ratio}")
    for f in failures:
        print(f"glass-check: {f}", file=sys.stderr)
    return 1 if failures else 0


def parse_text(text: str) -> tuple[tuple[int, int, int], float]:
    colour, _, ratio = text.partition(":")
    try:
        return parse_hex(colour), float(ratio)
    except ValueError as e:
        raise argparse.ArgumentTypeError(f"--text takes RRGGBB:RATIO: {text}") from e


# ---------------------------------------------------------------------------------- soak


SOAK_LINE = re.compile(
    r"glass-soak: frames=(\d+) warmup=(\d+) created=(\d+) deleted=(\d+) rss_start_kb=(\d+|-) "
    r"rss_end_kb=(\d+|-) blur=(.*)"
)


def parse_soak(output: str) -> dict | None:
    for line in output.splitlines():
        m = SOAK_LINE.search(line)
        if m:
            num = lambda s: None if s == "-" else int(s)  # noqa: E731
            return {
                "frames": int(m[1]),
                "warmup": int(m[2]),
                "created": int(m[3]),
                "deleted": int(m[4]),
                "rss_start": num(m[5]),
                "rss_end": num(m[6]),
                "blur": m[7].strip(),
            }
    return None


def run_soak(binary: str, frames: int, translucent: str) -> tuple[dict | None, str]:
    """One soak run on a private Xvfb display with private data, config, cache and Git
    configuration; returns the parsed report and everything the app printed."""
    with tempfile.TemporaryDirectory(prefix="canopy-glass-soak.") as tmp:
        env = {k: v for k, v in os.environ.items() if k != "WAYLAND_DISPLAY"}
        Path(f"{tmp}/gitconfig").write_text("[user]\n\tname = Soak\n\temail = soak@example.com\n")
        env.update(
            LIBGL_ALWAYS_SOFTWARE="1",
            XDG_DATA_HOME=f"{tmp}/data",
            XDG_CONFIG_HOME=f"{tmp}/config",
            XDG_CACHE_HOME=f"{tmp}/cache",
            GIT_CONFIG_GLOBAL=f"{tmp}/gitconfig",
            GIT_CONFIG_NOSYSTEM="1",
        )
        run = subprocess.run(
            ["timeout", "600", "xvfb-run", "-a", "-s", "-screen 0 1440x900x24", binary,
             "--glass-soak", str(frames), "--translucent", translucent, "--size", "1440x900"],
            env=env, capture_output=True, text=True,
        )
    return parse_soak(run.stdout), run.stdout + run.stderr


def growth(result: dict) -> int | None:
    if result["rss_start"] is None or result["rss_end"] is None:
        return None
    return result["rss_end"] - result["rss_start"]


def soak(args: argparse.Namespace) -> int:
    if not shutil.which("xvfb-run"):
        print("glass-check: soak needs xvfb-run (it never opens a window on your session)", file=sys.stderr)
        return 1
    binary = args.binary
    if binary is None:
        subprocess.run(
            ["cargo", "build", "-p", "canopy-app", "--locked", "--quiet", "--manifest-path", str(ROOT / "Cargo.toml")],
            check=True,
        )
        target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        binary = str(target / "debug" / "canopy")
    runs = {}
    for label, translucent in (("glass", "on"), ("opaque baseline", "off")):
        result, output = run_soak(binary, args.frames, translucent)
        if result is None:
            print(output, file=sys.stderr)
            print("glass-check: the soak printed no report (a release build ignores --glass-soak)", file=sys.stderr)
            return 1
        runs[label] = result
        print(
            f"{label}: {result['warmup']} warm-up and {result['frames']} measured frames, OpenGL objects "
            f"created {result['created']} and deleted "
            f"{result['deleted']}, resident {result['rss_start']} -> {result['rss_end']} kB "
            f"({growth(result)} kB), blur {result['blur']}"
        )
    glass, base = runs["glass"], runs["opaque baseline"]
    failures = []
    if glass["created"] == 0:
        failures.append("the blur never built (is OpenGL available?)")
    if glass["created"] != glass["deleted"]:
        failures.append("OpenGL objects leaked")
    if base["created"] != 0:
        failures.append("the opaque baseline built the blur")
    if "unavailable" in glass["blur"] and "window closed" not in glass["blur"]:
        failures.append(f"the blur turned itself off: {glass['blur']}")
    grew, base_grew = growth(glass), growth(base)
    if grew is None or base_grew is None:
        failures.append("no resident memory figures (Linux only)")
    else:
        print(f"resident memory the blur added over the baseline: {grew - base_grew} kB")
        if grew > args.max_growth_kb:
            failures.append(f"resident memory grew by {grew} kB with glass")
        if grew - base_grew > args.max_growth_kb:
            failures.append(f"the blur added {grew - base_grew} kB of resident memory over the baseline")
    for f in failures:
        print(f"glass-check: {f}", file=sys.stderr)
    return 1 if failures else 0


# ------------------------------------------------------------------------------ selftest


def write_png(path: Path, width: int, height: int, pixel) -> None:
    raw = bytearray()
    for y in range(height):
        raw.append(1 if y % 2 else 4)  # exercise the Sub and Paeth filters on encode
        line = bytearray()
        for x in range(width):
            line += bytes(pixel(x, y))
        if y % 2:
            enc = bytearray(line)
            for i in range(len(line) - 1, 3, -1):
                enc[i] = (line[i] - line[i - 4]) & 255
        else:
            prev = raw_rows[-1] if raw_rows else bytearray(len(line))
            enc = bytearray(len(line))
            for i in range(len(line)):
                a = line[i - 4] if i >= 4 else 0
                b = prev[i]
                c = prev[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                enc[i] = (line[i] - pred) & 255
        raw += enc
        raw_rows.append(line)

    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw)))
        + chunk(b"IEND", b"")
    )


raw_rows: list[bytearray] = []


def selftest() -> int:
    global raw_rows
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        base = lambda x, y: (40, 40, 48, 255) if (x // 3) % 2 else (200, 60, 60, 255)  # noqa: E731
        raw_rows = []
        write_png(tmp / "off.png", 64, 48, lambda x, y: (28, 28, 32, 255) if 16 <= x < 48 and 8 <= y < 40 else base(x, y))
        img = read_png(tmp / "off.png")
        assert (img.width, img.height, img.channels) == (64, 48, 4)
        assert img.rgb(0, 0) == (200, 60, 60) and img.rgb(3, 5) == (40, 40, 48)
        assert img.rgb(20, 20) == (28, 28, 32)
        # Smooth glass: a gentle ramp inside the region.
        raw_rows = []
        write_png(tmp / "on.png", 64, 48, lambda x, y: (28 + x // 8, 28, 32, 255) if 16 <= x < 48 and 8 <= y < 40 else base(x, y))
        on = read_png(tmp / "on.png")
        region = (16, 8, 48, 40)
        assert outside_differences(img, on, [region]) == 0
        assert outside_differences(img, on, []) > 0
        mask = background_mask(img, region)
        stats = smooth_residual(img, on, region, mask)
        assert stats["spread"] > 0.5 and stats["laplacian_p99"] < 0.5, stats
        # An opaque fill shows nothing through.
        raw_rows = []
        write_png(tmp / "flat.png", 64, 48, lambda x, y: (30, 30, 34, 255) if 16 <= x < 48 and 8 <= y < 40 else base(x, y))
        assert smooth_residual(img, read_png(tmp / "flat.png"), region, mask)["spread"] == 0
        # Sharp stripes behind the glass are caught.
        raw_rows = []
        write_png(tmp / "sharp.png", 64, 48, lambda x, y: (28 + 40 * ((x // 3) % 2), 28, 32, 255) if 16 <= x < 48 and 8 <= y < 40 else base(x, y))
        sharp = read_png(tmp / "sharp.png")
        assert smooth_residual(img, sharp, region, mask)["laplacian_p99"] > 1.0
        # Without OFF, the content itself: the stripes are sharper than the smooth glass.
        assert smooth_residual(None, sharp, region, mask)["laplacian_p99"] > stats["laplacian_p99"]
        assert abs(contrast((255, 255, 255), (0, 0, 0)) - 21) < 1e-9
        assert parse_region("1,2,3,4") == (1, 2, 3, 4)
        assert parse_text("E8E9EC:7") == ((0xE8, 0xE9, 0xEC), 7.0)
        report = "noise\nglass-soak: frames=2000 warmup=1440 created=27 deleted=27 rss_start_kb=90000 rss_end_kb=90100 blur=unavailable (the window closed)\n"
        assert parse_soak(report) == {
            "frames": 2000, "warmup": 1440, "created": 27, "deleted": 27, "rss_start": 90000, "rss_end": 90100,
            "blur": "unavailable (the window closed)",
        }
        assert parse_soak("nothing") is None
    print("glass-check selftest: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("compare")
    c.add_argument("off")
    c.add_argument("on")
    c.add_argument("--region", type=parse_region, action="append", required=True)
    c.add_argument("--text", type=parse_text, action="append", default=[])
    c.add_argument("--base")
    c.add_argument("--min-spread", type=float, default=0.0)
    c.add_argument("--max-sharpness", type=float, default=0.75)
    s = sub.add_parser("soak")
    s.add_argument("binary", nargs="?")
    s.add_argument("--frames", type=int, default=2000)
    s.add_argument("--max-growth-kb", type=int, default=2048)
    sub.add_parser("selftest")
    args = parser.parse_args()
    if args.command == "compare":
        return compare(args)
    if args.command == "soak":
        return soak(args)
    return selftest()


if __name__ == "__main__":
    sys.exit(main())
