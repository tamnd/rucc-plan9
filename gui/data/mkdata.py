"""Make the image files of gui/data for the GUI scripts.

Run it with python3 -I gui/data/mkdata.py. The files are small and the output is the same each time.
"""

import os
import struct
import sys

W, H = 96, 48
# The colour bars: white, yellow, cyan, green, magenta, red, blue and black.
BARS = [(255, 255, 255), (255, 255, 0), (0, 255, 255), (0, 255, 0), (255, 0, 255), (255, 0, 0), (0, 0, 255), (0, 0, 0)]


def rgb(x, y):
    """The colour of the pixel x, y: bars at the top, and a grey ramp in the bottom quarter."""
    if y >= H * 3 // 4:
        v = x * 255 // (W - 1)
        return (v, v, v)
    return BARS[x * len(BARS) // W]


def ycbcr(c):
    """The 10-bit Rec. 601 values of the colour c."""
    r, g, b = c
    y = 16 + (65.738 * r + 129.057 * g + 25.064 * b) / 256
    cb = 128 + (-37.945 * r - 74.494 * g + 112.439 * b) / 256
    cr = 128 + (112.439 * r - 94.154 * g - 18.285 * b) / 256
    return [int(round(v * 4)) for v in (y, cb, cr)]


def uyvy(y):
    """The 10-bit values Cb Y Cr Y of the line y, two pixels for each four values."""
    out = []
    for x in range(0, W, 2):
        y0, cb, cr = ycbcr(rgb(x, y))
        y1 = ycbcr(rgb(x + 1, y))[0]
        out += [cb, y0, cr, y1]
    return out


def tga():
    """An uncompressed 24-bit TGA file with the first line at the top."""
    head = struct.pack("<BBBHHBHHHHBB", 0, 0, 2, 0, 0, 0, 0, 0, W, H, 24, 0x20)
    px = bytearray()
    for y in range(H):
        for x in range(W):
            r, g, b = rgb(x, y)
            px += bytes((b, g, r))
    return head + bytes(px)


def yuv():
    """An 8-bit UYVY file, as yuv reads it."""
    return bytes(v >> 2 for y in range(H) for v in uyvy(y))


def v210():
    """A v210 file: three 10-bit values in each 32-bit word, 128 bytes for each 48 pixels."""
    out = bytearray()
    for y in range(H):
        vals = uyvy(y)
        line = bytearray()
        for i in range(0, len(vals), 3):
            t = vals[i] | vals[i + 1] << 10 | vals[i + 2] << 20
            line += struct.pack("<I", t)
        out += line + bytes(128 * ((W + 47) // 48) - len(line))
    return bytes(out)


def main():
    d = os.path.dirname(os.path.abspath(sys.argv[0]))
    files = {
        "bars.tga": tga(),
        "bars.yuv": yuv(),
        "bars.v210": v210(),
        # yuv and v210 find the size of the image in /lib/video.specs by the size of the file.
        # The fields 3, 5 and 7 are the pixels of a line, the lines and the more lines.
        "lib/video.specs": f"# name x pixels {W} lines {H} more 0\nbars x pixels {W} lines {H} more 0\n".encode(),
    }
    for name, data in files.items():
        with open(os.path.join(d, name), "wb") as f:
            f.write(data)


main()
