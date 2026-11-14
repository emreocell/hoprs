#!/usr/bin/env python3
"""Packs the rendered hoprs-N.png files into hoprs.ico and hoprs.icns (standard library
only; called by scripts/icons.sh). Both containers just wrap the PNGs, byte for byte.

usage: pack-icons.py <icon dir>
"""
import struct
import sys
from pathlib import Path

d = Path(sys.argv[1])


def png(n):
    return (d / f"hoprs-{n}.png").read_bytes()


# ICO: ICONDIR (reserved 0, type 1, count) + one 16-byte ICONDIRENTRY per frame + the
# PNG frames. Width/height 0 mean 256.
ico_sizes = [16, 24, 32, 48, 64, 128, 256]
offset = 6 + 16 * len(ico_sizes)
head, body = struct.pack("<HHH", 0, 1, len(ico_sizes)), b""
for n in ico_sizes:
    data = png(n)
    head += struct.pack(
        "<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(data), offset + len(body)
    )
    body += data
(d / "hoprs.ico").write_bytes(head + body)

# ICNS: "icns" + total length, then (type, length incl. header, PNG) chunks. The @2x
# types reuse the PNG of twice the size.
icns_types = [
    ("ic04", 16), ("ic05", 32), ("ic07", 128), ("ic08", 256), ("ic09", 512),
    ("ic10", 1024), ("ic11", 32), ("ic12", 64), ("ic13", 256), ("ic14", 512),
]
chunks = b""
for t, n in icns_types:
    data = png(n)
    chunks += t.encode() + struct.pack(">I", len(data) + 8) + data
(d / "hoprs.icns").write_bytes(b"icns" + struct.pack(">I", len(chunks) + 8) + chunks)
