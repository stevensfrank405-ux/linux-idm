"""Generate the Windows icon from the same geometric mark used by the native window."""
import struct
from pathlib import Path
size = 64
pixels = bytearray()
for y in reversed(range(size)):
    for x in range(size):
        arrow = (27 <= x < 37 and 12 <= y < 40) or (32 <= y < 49 and y - 17 <= x <= 81 - y) or (15 <= x < 49 and 51 <= y < 56)
        r, g, b = (139, 229, 187) if arrow else (22, 27, 34)
        pixels.extend((b, g, r, 255))
mask = bytes(((size + 31) // 32) * 4 * size)
dib = struct.pack('<IiiHHIIiiII', 40, size, size * 2, 1, 32, 0, len(pixels) + len(mask), 0, 0, 0, 0) + pixels + mask
icon = struct.pack('<HHH', 0, 1, 1) + struct.pack('<BBBBHHII', size, size, 0, 0, 1, 32, len(dib), 22) + dib
(Path(__file__).resolve().parents[1] / 'assets/opendownload.ico').write_bytes(icon)
