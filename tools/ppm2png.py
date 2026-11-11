#!/usr/bin/env python3
"""Convert a binary PPM (P6) screenshot into a PNG without third-party libs."""
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
    src, dst = sys.argv[1], sys.argv[2]
    data = open(src, 'rb').read()
    magic, pos = read_token(data, 0)
    if magic != b'P6':
        raise SystemExit('not a P6 PPM')
    width, pos = read_token(data, pos)
    height, pos = read_token(data, pos)
    maxval, pos = read_token(data, pos)
    width, height, maxval = int(width), int(height), int(maxval)
    pos += 1  # single whitespace after maxval
    raw = data[pos:pos + width * height * 3]

    rows = b''.join(b'\x00' + raw[y * width * 3:(y + 1) * width * 3] for y in range(height))

    def chunk(kind, payload):
        return (struct.pack('>I', len(payload)) + kind + payload
                + struct.pack('>I', zlib.crc32(kind + payload) & 0xFFFFFFFF))

    png = b'\x89PNG\r\n\x1a\n'
    png += chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(rows, 6))
    png += chunk(b'IEND', b'')
    open(dst, 'wb').write(png)
    print(f'{dst}: {width}x{height} ({len(png)} bytes)')


if __name__ == '__main__':
    main()
