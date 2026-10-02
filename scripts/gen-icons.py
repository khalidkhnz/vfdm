#!/usr/bin/env python3
"""Generate the app icon (blue rounded square, white down-arrow into a tray)
as PNGs with only the standard library. Writes:
  apps/desktop/app-icon.png            1024x1024 (input for `tauri icon`)
  apps/extension/public/icons/{16,48,128}.png
"""
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BG = (37, 99, 235)
FG = (255, 255, 255)


def png(w, h, rows):
    raw = b"".join(b"\x00" + bytes(r) for r in rows)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def render(size):
    s = size
    r = s * 0.22
    rows = []
    for y in range(s):
        row = bytearray()
        for x in range(s):
            # rounded square mask
            dx = max(r - x, 0, x - (s - 1 - r))
            dy = max(r - y, 0, y - (s - 1 - r))
            inside = (dx * dx + dy * dy) <= r * r
            if not inside:
                row += bytes((0, 0, 0, 0))
                continue
            fx, fy = x / s, y / s
            fg = False
            # arrow shaft
            if 0.43 <= fx <= 0.57 and 0.20 <= fy <= 0.52:
                fg = True
            # arrow head (triangle)
            if 0.52 <= fy <= 0.70:
                half = (0.70 - fy) / 0.18 * 0.24
                if abs(fx - 0.5) <= half:
                    fg = True
            # tray
            if 0.76 <= fy <= 0.84 and 0.24 <= fx <= 0.76:
                fg = True
            if (0.66 <= fy <= 0.84) and (0.24 <= fx <= 0.31 or 0.69 <= fx <= 0.76):
                fg = True
            row += bytes((*FG, 255) if fg else (*BG, 255))
        rows.append(row)
    return rows


def downsample(rows, src, dst):
    f = src // dst
    out = []
    for y in range(dst):
        row = bytearray()
        for x in range(dst):
            acc = [0, 0, 0, 0]
            for yy in range(f):
                r = rows[y * f + yy]
                for xx in range(f):
                    i = (x * f + xx) * 4
                    for c in range(4):
                        acc[c] += r[i + c]
            row += bytes(v // (f * f) for v in acc)
        out.append(row)
    return out


def main():
    big = render(1024)
    (ROOT / "apps/desktop").mkdir(parents=True, exist_ok=True)
    (ROOT / "apps/desktop/app-icon.png").write_bytes(png(1024, 1024, big))
    out = ROOT / "apps/extension/public/icons"
    out.mkdir(parents=True, exist_ok=True)
    base = downsample(big, 1024, 128)
    (out / "128.png").write_bytes(png(128, 128, base))
    (out / "48.png").write_bytes(png(48, 48, downsample(render(384), 384, 48)))
    (out / "16.png").write_bytes(png(16, 16, downsample(render(128), 128, 16)))
    print("icons written")


if __name__ == "__main__":
    main()
