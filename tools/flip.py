import struct
import sys
import zlib

src, dst = sys.argv[1], sys.argv[2]
data = open(src, 'rb').read()


def token(pos):
    while data[pos:pos + 1].isspace():
        pos += 1
    start = pos
    while not data[pos:pos + 1].isspace():
        pos += 1
    return data[start:pos], pos


_, pos = token(0)
width, pos = token(pos)
height, pos = token(pos)
_, pos = token(pos)
pos += 1
width, height = int(width), int(height)
stride = width * 3
rows = []
for y in range(height - 1, -1, -1):
    rows.append(b'\x00' + data[pos + y * stride:pos + (y + 1) * stride])
raw = b''.join(rows)


def chunk(kind, payload):
    return (struct.pack('>I', len(payload)) + kind + payload
            + struct.pack('>I', zlib.crc32(kind + payload) & 0xFFFFFFFF))


png = b'\x89PNG\r\n\x1a\n'
png += chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
png += chunk(b'IDAT', zlib.compress(raw, 6))
png += chunk(b'IEND', b'')
open(dst, 'wb').write(png)
print(f'{dst}: {width}x{height} flipped')
