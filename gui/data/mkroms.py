"""Make the ROM files of gui/data/rom for the scripts of the emulators.

Run it with python3 -I gui/data/mkroms.py. Each ROM is a small test program. It draws a fixed picture and then loops, so that the picture is the same in each frame. No ROM has code or data of a game or of a system ROM. The output is the same each time.
"""

import os
import struct
import sys


def assemble(org, prog):
    """Assemble prog at the address org and give the bytes.

    prog is a list. A string that ends in ':' is a label. A string is an opcode of hex bytes. ('rel', L) is the 8-bit offset to the label L from the end of the offset, and ('abs', L) and ('abs16be', L) are the 16-bit address of L. ('rel16be', L) is the 16-bit offset to L from the start of the offset, as the dbra of the 68000 has. ('relm68k', L) is the 8-bit offset of bra.s and bcc.s of the 68000.
    """
    labels = {}
    for pass_ in (0, 1):
        out = bytearray()
        for item in prog:
            pc = org + len(out)
            if isinstance(item, str) and item.endswith(":"):
                labels[item[:-1]] = pc
            elif isinstance(item, str):
                out += bytes.fromhex(item)
            elif isinstance(item, (bytes, bytearray)):
                out += item
            else:
                kind, label = item
                target = labels.get(label, pc) if pass_ == 0 else labels[label]
                if kind == "rel":
                    d = target - (pc + 1)
                    if pass_ == 1 and not -128 <= d <= 127:
                        raise ValueError(f"branch to {label} is too far")
                    out += struct.pack("b", d if pass_ == 1 else 0)
                elif kind == "abs":
                    out += struct.pack("<H", target & 0xFFFF)
                elif kind == "abs16be":
                    out += struct.pack(">H", target & 0xFFFF)
                elif kind == "rel16be":
                    out += struct.pack(">h", target - pc if pass_ == 1 else 0)
                elif kind == "relm68k":
                    # The 8-bit offset of bra.s and bcc.s, from the end of the opcode word.
                    d = target - (pc + 1)
                    out += struct.pack("b", d if pass_ == 1 else 0)
                else:
                    raise ValueError(kind)
    return bytes(out), labels


def a2600():
    """An Atari 2600 ROM of 4 KB. The background colour changes each 16 lines."""
    prog = [
        "start:",
        "78", "d8", "a2ff", "9a",  # sei, cld, ldx #$ff, txs
        "a900", "a200",  # lda #0, ldx #0
        "clear:", "9500", "ca", "d0", ("rel", "clear"),  # sta 0,x; dex; bne
        "frame:",
        "a902", "8501", "8500",  # lda #2; sta VBLANK; sta VSYNC
        "8502", "8502", "8502",  # sta WSYNC 3 times
        "a900", "8500",  # lda #0; sta VSYNC
        "a225",  # ldx #37
        "vb:", "8502", "ca", "d0", ("rel", "vb"),
        "a900", "8501",  # lda #0; sta VBLANK
        "a2c0",  # ldx #192
        "pic:", "8502", "8a", "29f0", "0908", "8509", "ca", "d0", ("rel", "pic"),
        "a902", "8501",  # lda #2; sta VBLANK
        "a21e",  # ldx #30
        "os:", "8502", "ca", "d0", ("rel", "os"),
        "4c", ("abs", "frame"),
    ]
    code, labels = assemble(0xF000, prog)
    rom = bytearray(code.ljust(0x1000, b"\0"))
    struct.pack_into("<HH", rom, 0xFFC, labels["start"], labels["start"])
    return bytes(rom)


def nes():
    """An iNES ROM with mapper 0: 16 KB of program and 8 KB of tiles."""
    prog = [
        "reset:",
        "78", "d8", "a240", "8e1740", "a2ff", "9a", "e8",  # sei cld; ldx #$40; stx $4017; ldx #$ff; txs; inx
        "8e0020", "8e0120", "8e1040",  # stx $2000 $2001 $4010
        "w1:", "2c0220", "10", ("rel", "w1"),
        "w2:", "2c0220", "10", ("rel", "w2"),
        # The palette, 32 bytes at $3f00.
        "ad0220", "a93f", "8d0620", "a900", "8d0620",
        "a200", "pal:", "bd", ("abs", "paldata"), "8d0720", "e8", "e020", "d0", ("rel", "pal"),
        # The name table at $2000: tile (x + y/4) & 3.
        "a920", "8d0620", "a900", "8d0620",
        "a000",
        "row:", "a200",
        "col:", "8a", "4a", "4a", "8501", "98", "4a", "4a", "18", "6501", "2903", "8d0720",
        "e8", "e020", "d0", ("rel", "col"),
        "c8", "c01e", "d0", ("rel", "row"),
        # The attribute table: a different palette in each square of 32 by 32 pixels.
        "a200", "att:", "8a", "8d0720", "e8", "e040", "d0", ("rel", "att"),
        "a900", "8d0520", "8d0520", "8d0020",  # scroll 0, 0
        "a90a", "8d0120",  # show the background
        "loop:", "4c", ("abs", "loop"),
        "nmi:", "40",  # rti
        "paldata:",
        bytes([0x0F, 0x01, 0x11, 0x21, 0x0F, 0x06, 0x16, 0x26, 0x0F, 0x09, 0x19, 0x29, 0x0F, 0x04, 0x14, 0x24] * 2),
    ]
    code, labels = assemble(0xC000, prog)
    prg = bytearray(code.ljust(0x4000, b"\0"))
    struct.pack_into("<HHH", prg, 0x3FFA, labels["nmi"], labels["reset"], labels["nmi"])
    chr_ = bytearray(0x2000)
    for t in range(4):
        lo = 0xFF if t & 1 else 0
        hi = 0xFF if t & 2 else 0
        for row in range(8):
            chr_[t * 16 + row] = lo
            chr_[t * 16 + 8 + row] = hi
    head = b"NES\x1a" + bytes([1, 1, 0, 0]) + bytes(8)
    return head + bytes(prg) + bytes(chr_)


def c64():
    """The three ROMs of the c64 emulator: kernal.bin, basic.bin and crom.bin.

    The kernal sets the colours of the screen and fills the screen with the 8 characters of crom.bin, each in its colour. There is no BASIC, so basic.bin has zeros only.
    """
    fill = []
    for page in (0x04, 0x05, 0x06, 0x07):
        fill += ["9d00" + f"{page:02x}"]  # sta $pp00,x
    color = []
    for page in (0xD8, 0xD9, 0xDA, 0xDB):
        color += ["9d00" + f"{page:02x}"]
    prog = [
        "reset:",
        "78", "d8", "a2ff", "9a",
        "a92f", "8500", "a937", "8501",  # the processor port: I/O at $d000
        "a91b", "8d11d0", "a908", "8d16d0", "a914", "8d18d0",  # VIC: text, 40 columns, screen $0400
        "a903", "8d02dd", "8d00dd",  # VIC bank 0
        "a906", "8d20d0", "a900", "8d21d0",  # border blue, background black
        "a200",
        "fill:", "8a", "2907", *fill, "8a", "4a", "4a", "4a", "2907", "0908", *color,
        "e8", "d0", ("rel", "fill"),
        "loop:", "4c", ("abs", "loop"),
        "nmi:", "40",
    ]
    code, labels = assemble(0xE000, prog)
    kernal = bytearray(code.ljust(0x2000, b"\0"))
    struct.pack_into("<HHH", kernal, 0x1FFA, labels["nmi"], labels["reset"], labels["nmi"])
    glyphs = [
        [0xFF] * 8,  # a full square
        [0xAA, 0x55] * 4,  # a checker
        [0xFF, 0x00] * 4,  # lines across
        [0xCC] * 8,  # lines down
        [0x80 >> i for i in range(8)],  # a diagonal
        [0xFF, 0x81, 0x81, 0x81, 0x81, 0x81, 0x81, 0xFF],  # a frame
        [0x00, 0x00, 0x18, 0x3C, 0x3C, 0x18, 0x00, 0x00],  # a dot
        [0x18, 0x18, 0x18, 0xFF, 0xFF, 0x18, 0x18, 0x18],  # a cross
    ]
    crom = bytearray(0x1000)
    for i, g in enumerate(glyphs):
        crom[i * 8 : i * 8 + 8] = bytes(g)
    return {"kernal.bin": bytes(kernal), "basic.bin": bytes(0x2000), "crom.bin": bytes(crom)}


def snes():
    """A LoROM SNES ROM of 32 KB. BG1 in mode 0 shows bands of 4 colours."""
    prog = [
        "reset:",
        "78", "18", "fb",  # sei, clc, xce: native mode
        "c230", "a2ff1f", "9a", "a90000", "5b",  # rep #$30; ldx #$1fff; txs; lda #0; tcd
        "e220", "a980", "8d0021",  # sep #$20; lda #$80; sta $2100
        "9c0521", "a904", "8d0721", "9c0b21", "a901", "8d2c21",
        "9c0d21", "9c0d21", "9c0e21", "9c0e21",
        "a980", "8d1521",
        "9c2121", "a20000",
        "pal:", "bd", ("abs", "paldata"), "8d2221", "e8", "e00800", "d0", ("rel", "pal"),
        "c220",  # rep #$20
        "9c1621", "a20000",
        "tiles:", "bd", ("abs", "tiledata"), "8d1821", "e8", "e8", "e04000", "d0", ("rel", "tiles"),
        "a90004", "8d1621", "a20000",
        "map:", "8a", "4a", "4a", "8500", "8a", "4a", "4a", "4a", "4a", "4a", "4a", "4a", "18", "6500",
        "290300", "8d1821", "e8", "e00004", "d0", ("rel", "map"),
        "e220", "a90f", "8d0021",
        "loop:", "80", ("rel", "loop"),
        "nmi:", "40",
        "paldata:",
        struct.pack("<4H", 0x0000, 0x7C00, 0x03E0, 0x001F),
        "tiledata:",
        b"".join(struct.pack("<H", (0xFF if t & 1 else 0) | (0xFF00 if t & 2 else 0)) * 8 for t in range(4)),
    ]
    code, labels = assemble(0x8000, prog)
    rom = bytearray(code.ljust(0x8000, b"\0"))
    rom[0x7FC0:0x7FD5] = b"R9 GUI TEST".ljust(21)
    rom[0x7FD5] = 0x20  # LoROM
    rom[0x7FD6] = 0
    rom[0x7FD7] = 0x08  # 256 KB or less
    rom[0x7FD8] = 0
    for v in range(0x7FE4, 0x7FF0, 2):
        struct.pack_into("<H", rom, v, labels["nmi"])
    struct.pack_into("<HHH", rom, 0x7FFA, labels["nmi"], labels["reset"], labels["nmi"])
    return bytes(rom)


GB_LOGO = bytes([
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83,
    0x00, 0x0C, 0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E,
])


def gb():
    """A Game Boy ROM of 32 KB. The background has diagonal bands of the 4 shades."""
    prog = [
        "start:",
        "f3", "31feff",  # di; ld sp,$fffe
        "3e00", "e040",  # ld a,0; ldh (LCDC),a
        "210080",  # ld hl,$8000
    ]
    for t in range(4):
        lo = 0xFF if t & 1 else 0
        hi = 0xFF if t & 2 else 0
        prog += ["0608", f"t{t}:", f"3e{lo:02x}", "22", f"3e{hi:02x}", "22", "05", "20", ("rel", f"t{t}")]
    prog += [
        "210098", "1600",  # ld hl,$9800; ld d,0
        "row:", "1e00",
        "col:", "7b", "82", "cb3f", "cb3f", "e603", "22", "1c", "7b", "fe20", "20", ("rel", "col"),
        "14", "7a", "fe20", "20", ("rel", "row"),
        "3ee4", "e047", "3e00", "e042", "e043",
        "3e91", "e040",
        "loop:", "18", ("rel", "loop"),
    ]
    code, _ = assemble(0x150, prog)
    rom = bytearray(0x8000)
    rom[0x100:0x104] = bytes([0x00, 0xC3, 0x50, 0x01])  # nop; jp $150
    rom[0x104:0x104 + len(GB_LOGO)] = GB_LOGO
    rom[0x134:0x143] = b"R9 GUI TEST".ljust(15, b"\0")
    rom[0x147] = 0  # ROM only
    rom[0x148] = 0  # 32 KB
    rom[0x149] = 0
    rom[0x14D] = (-sum(rom[0x134:0x14D]) - (0x14D - 0x134)) & 0xFF
    rom[0x150 : 0x150 + len(code)] = code
    return bytes(rom)


def gba():
    """A GBA ROM, and a BIOS that only jumps to the ROM. The ROM fills the screen in mode 3."""
    words = [
        0xEA00002E,  # b 0xc0
    ]
    rom = bytearray(0xC0)
    struct.pack_into("<I", rom, 0, words[0])
    rom[0xA0:0xAC] = b"R9 GUI TEST\0"
    rom[0xB2] = 0x96
    code = [
        0xE3A00301,  # mov r0, #0x04000000
        0xE3A01B01,  # mov r1, #0x400
        0xE2811003,  # add r1, r1, #3
        0xE1C010B0,  # strh r1, [r0]: mode 3 and BG2
        0xE3A00406,  # mov r0, #0x06000000
        0xE3A02000,  # mov r2, #0
        0xE3A03000,  # yloop: mov r3, #0
        0xE1A041A3,  # xloop: mov r4, r3, lsr #3
        0xE1A051A2,  # mov r5, r2, lsr #3
        0xE1844285,  # orr r4, r4, r5, lsl #5
        0xE0835002,  # add r5, r3, r2
        0xE1A05225,  # mov r5, r5, lsr #4
        0xE1844505,  # orr r4, r4, r5, lsl #10
        0xE0C040B2,  # strh r4, [r0], #2
        0xE2833001,  # add r3, r3, #1
        0xE35300F0,  # cmp r3, #240
        None,  # blt xloop
        0xE2822001,  # add r2, r2, #1
        0xE35200A0,  # cmp r2, #160
        None,  # blt yloop
        0xEAFFFFFE,  # b .
    ]
    base = 0xC0
    xloop, yloop = base + 7 * 4, base + 6 * 4
    out = bytearray()
    for i, w in enumerate(code):
        pc = base + i * 4
        if w is None:
            target = xloop if i == 16 else yloop
            w = 0xBA000000 | (((target - (pc + 8)) >> 2) & 0xFFFFFF)
        out += struct.pack("<I", w)
    rom += out
    rom = bytes(rom.ljust(0x400, b"\0"))
    bios = bytearray(0x4000)
    struct.pack_into("<I", bios, 0, 0xE3A0F302)  # mov pc, #0x08000000
    return rom, bytes(bios)


def md():
    """A Mega Drive ROM of 2 KB. Plane A shows diagonal bands of 4 colours."""
    regs = [0x8004, 0x8104, 0x8230, 0x8407, 0x8578, 0x8700, 0x8A00, 0x8B00, 0x8C81, 0x8D3F, 0x8F02, 0x9001]
    prog = [
        "start:",
        "46fc2700",  # move.w #$2700,sr
        "43f900c00004",  # lea $c00004,a1: the control port
        "41f900c00000",  # lea $c00000,a0: the data port
        "45fa", ("rel16be", "regs"),  # lea regs(pc),a2
        f"70{len(regs) - 1:02x}",  # moveq #n-1,d0
        "rl:", "329a", "51c8", ("rel16be", "rl"),  # move.w (a2)+,(a1); dbra d0,rl
        "22bcc0000000",  # move.l #$c0000000,(a1): write CRAM at 0
        "45fa", ("rel16be", "pal"), "700f",
        "pl:", "309a", "51c8", ("rel16be", "pl"),
        "22bc40200000",  # move.l #$40200000,(a1): write VRAM at $20
        "323c1111", "7403",  # move.w #$1111,d1; moveq #3,d2
        "tl:", "700f",
        "tw:", "3081", "51c8", ("rel16be", "tw"),  # move.w d1,(a0); dbra d0,tw
        "06411111", "51ca", ("rel16be", "tl"),  # add.w #$1111,d1; dbra d2,tl
        "22bc40000003",  # move.l #$40000003,(a1): write VRAM at $c000
        "7600",  # moveq #0,d3
        "yl:", "7800",
        "xl:", "3a04", "da43", "e44d", "02450003", "5245", "3085",
        "5244", "0c440040", "66", ("relm68k", "xl"),
        "5243", "0c430020", "66", ("relm68k", "yl"),
        "32bc8144",  # move.w #$8144,(a1): the display on
        "loop:", "60fe",
        "rte:", "4e73",
        "regs:", b"".join(struct.pack(">H", r) for r in regs),
        "pal:", struct.pack(">16H", 0x000, 0x00E, 0x0E0, 0xE00, 0x0EE, *([0] * 11)),
    ]
    code, labels = assemble(0x200, prog)
    rom = bytearray(0x800)
    struct.pack_into(">II", rom, 0, 0x00FFFE00, labels["start"])
    for v in range(8, 0x100, 4):
        struct.pack_into(">I", rom, v, labels["rte"])
    rom[0x100:0x110] = b"SEGA MEGA DRIVE "
    rom[0x150:0x160] = b"R9 GUI TEST".ljust(16)
    struct.pack_into(">IIII", rom, 0x1A0, 0, len(rom) - 1, 0x00FF0000, 0x00FFFFFF)
    rom[0x1F0:0x1F3] = b"JUE"
    rom[0x200 : 0x200 + len(code)] = code
    return bytes(rom)


def main():
    d = os.path.join(os.path.dirname(os.path.abspath(sys.argv[0])), "rom")
    os.makedirs(os.path.join(d, "c64"), exist_ok=True)
    gbarom, gbabios = gba()
    files = {
        "bands.a26": a2600(),
        "grid.nes": nes(),
        "bands.sfc": snes(),
        "bands.gb": gb(),
        "bands.gba": gbarom,
        "bands.md": md(),
        "gbabios.bin": gbabios,
    }
    for name, data in c64().items():
        files["c64/" + name] = data
    for name, data in files.items():
        with open(os.path.join(d, name), "wb") as f:
            f.write(data)


main()
