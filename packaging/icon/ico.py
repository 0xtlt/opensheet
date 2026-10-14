"""Packs PNG files into a Windows .ico (PNG entries, Windows Vista and later).

    python3 ico.py out.ico 16.png 24.png ... 256.png
"""
import struct
import sys

out, pngs = sys.argv[1], sys.argv[2:]
images = [open(p, "rb").read() for p in pngs]
header = struct.pack("<HHH", 0, 1, len(images))
offset = len(header) + 16 * len(images)
entries = b""
for data in images:
    width, height = struct.unpack(">II", data[16:24])
    entries += struct.pack("<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
with open(out, "wb") as f:
    f.write(header + entries + b"".join(images))
