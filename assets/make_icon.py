"""Draws the medialib icon without any image library.

Writes icon.ico (embedded into the .exe by build.rs) and icon-64.rgba (raw
RGBA, used as the window icon at runtime). Run from this directory:
    python make_icon.py
"""
import struct

TOP = (0x3A, 0x6F, 0xD8)
BOTTOM = (0x14, 0x22, 0x46)
GLYPH = (0xF4, 0xF7, 0xFF)
SS = 4  # supersampling factor per axis, for anti-aliased edges


def inside_rounded_rect(x, y, radius):
    # Unit square [0,1]^2 with rounded corners.
    cx = min(max(x, radius), 1 - radius)
    cy = min(max(y, radius), 1 - radius)
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius ** 2


def inside_triangle(x, y):
    # Play glyph, nudged right of centre so it looks optically centred.
    ax, ay = 0.38, 0.28
    bx, by = 0.38, 0.72
    cx, cy = 0.74, 0.50

    def side(px, py, qx, qy, rx, ry):
        return (px - rx) * (qy - ry) - (qx - rx) * (py - ry)

    d1 = side(x, y, ax, ay, bx, by)
    d2 = side(x, y, bx, by, cx, cy)
    d3 = side(x, y, cx, cy, ax, ay)
    neg = d1 < 0 or d2 < 0 or d3 < 0
    pos = d1 > 0 or d2 > 0 or d3 > 0
    return not (neg and pos)


def render(size):
    """Returns rows of (r, g, b, a), top row first."""
    rows = []
    for py in range(size):
        row = []
        for px in range(size):
            bg = glyph = 0
            for sy in range(SS):
                for sx in range(SS):
                    x = (px + (sx + 0.5) / SS) / size
                    y = (py + (sy + 0.5) / SS) / size
                    if inside_rounded_rect(x, y, 0.22):
                        bg += 1
                        if inside_triangle(x, y):
                            glyph += 1
            n = SS * SS
            t = py / max(size - 1, 1)
            base = [TOP[i] + (BOTTOM[i] - TOP[i]) * t for i in range(3)]
            if bg:
                g = glyph / bg
                rgb = [round(base[i] * (1 - g) + GLYPH[i] * g) for i in range(3)]
            else:
                rgb = [0, 0, 0]
            row.append((*rgb, round(255 * bg / n)))
        rows.append(row)
    return rows


def bmp_entry(rows):
    size = len(rows)
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    pixels = b"".join(
        bytes((b, g, r, a)) for row in reversed(rows) for (r, g, b, a) in row
    )
    mask_row = ((size + 31) // 32) * 4
    return header + pixels + b"\0" * (mask_row * size)


def main():
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = {s: render(s) for s in sizes}

    entries = [bmp_entry(images[s]) for s in sizes]
    out = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    for s, data in zip(sizes, entries):
        dim = 0 if s == 256 else s
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    out += b"".join(entries)
    with open("icon.ico", "wb") as f:
        f.write(out)

    with open("icon-64.rgba", "wb") as f:
        f.write(b"".join(bytes(px) for row in images[64] for px in row))

    with open("icon-256.png", "wb") as f:
        f.write(png(images[256]))


def png(rows):
    """Minimal RGBA PNG encoder, for the README logo."""
    import zlib

    size = len(rows)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    raw = b"".join(b"\0" + b"".join(bytes(px) for px in row) for row in rows)
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    main()
