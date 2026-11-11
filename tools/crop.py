#!/usr/bin/env python3
"""Crop and nearest-neighbour scale a P6 PPM into a PNG."""
import struct
import sys
import zlib


def read_token(data, pos):
    while pos < len(data):
        if data[pos:pos + 1] == b'#':
            while pos < len(data) and data[pos:pos + 1] != b'\n':
                pos += 1
        elif data[pos:pos + 1].isspace():
            pos += 1
        else:
            break
    start = pos
    while pos < len(data) and not data[pos:pos + 1].isspace():
        pos += 1
    return data[start:pos], pos


def main():
    src, dst, x, y, w, h, scale = sys.argv[1:8]
    flip = len(sys.argv) > 8 and sys.argv[8] == 'flip'
    x, y, w, h, scale = int(x), int(y), int(w), int(h), int(scale)
    data = open(src, 'rb').read()
    _, pos = read_token(data, 0)
    width, pos = read_token(data, pos)
    height, pos = read_token(data, pos)
    _, pos = read_token(data, pos)
    width, height = int(width), int(height)
    pos += 1
    src_stride = width * 3
    rows = []
    source_rows = range(h) if not flip else range(h - 1, -1, -1)
    for oy in source_rows:
        row = bytearray()
        for ox in range(w):
            sx, sy = x + ox, y + oy
            if 0 <= sx < width and 0 <= sy < height:
                off = pos + sy * src_stride + sx * 3
                pixel = data[off:off + 3]
            else:
                pixel = b'\x00\x00\x00'
            row += pixel * scale
        for _ in range(scale):
            rows.append(b'\x00' + bytes(row))
    raw = b''.join(rows)

    def chunk(kind, payload):
        return (struct.pack('>I', len(payload)) + kind + payload
                + struct.pack('>I', zlib.crc32(kind + payload) & 0xFFFFFFFF))

    png = b'\x89PNG\r\n\x1a\n'
    png += chunk(b'IHDR', struct.pack('>IIBBBBB', w * scale, h * scale, 8, 2, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(raw, 6))
    png += chunk(b'IEND', b'')
    open(dst, 'wb').write(png)
    print(f'{dst}: {w*scale}x{h*scale}')


if __name__ == '__main__':
    main()
