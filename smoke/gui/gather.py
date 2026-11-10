"""Compile the GUI smoke programs and their libraries with rucc, for the input disk of r9 boot.

Usage: python3 -I smoke/gui/gather.py RUCC TREE OBJTYPE OUT

RUCC is the rucc binary, TREE is a 9front tree (such as ~/.cache/r9/trees/front), OBJTYPE is
amd64 or arm64, and OUT is a new directory. The script compiles each file with the kencc driver
of rucc and the CFLAGS of its mkfile, and writes:

- OUT/lib/LIB/*.O, the objects of each library that the mkfile builds.
- OUT/cmd/PROG/*.O, the objects of each program of more than one file.
- OUT/one/PROG.O, the object of each program of one file.
- OUT/gui.rc and OUT/start, the guest scripts of this directory.

Then give OUT to r9 boot with --in, and smoke/gui/run.rc with --run.
"""
import concurrent.futures
import os
import re
import shutil
import subprocess
import sys

rucc, tree, objtype, out = sys.argv[1:5]
tree = os.path.abspath(os.path.expanduser(tree))
TARGET = {"amd64": "x86_64-plan9", "arm64": "aarch64-plan9"}[objtype]
O = {"amd64": "6", "arm64": "7"}[objtype]
SRC = os.path.join(tree, "sys/src")

# The libraries, with the source directories of each. A directory named DIR/port gives the
# names that DIR/OBJTYPE does not replace, as the reduce scripts of the tree do.
LIBS = {
    "libc": ["libc/port", "libc/9sys", "libc/fmt", "libc/" + objtype],
    "libdraw": ["libdraw"],
    "libmemdraw": ["libmemdraw"],
    "libmemlayer": ["libmemlayer"],
    "libframe": ["libframe"],
    "libthread": ["libthread"],
    "libplumb": ["libplumb"],
    "libcomplete": ["libcomplete"],
    "libbio": ["libbio"],
    "libregexp": ["libregexp"],
    "libsec": ["libsec/port", "libsec/" + objtype],
    "libmp": ["libmp/port", "libmp/" + objtype],
    "libauth": ["libauth"],
    "libauthsrv": ["libauthsrv"],
    "libip": ["libip"],
    "libcontrol": ["libcontrol"],
    "libflate": ["libflate"],
}

# The programs of more than one file: the directory, the variables that name the objects, and
# the extra names.
CMDS = {
    "rio": ("cmd/rio", ["OFILES"], []),
    "acme": ("cmd/acme", ["OFILES"], []),
    "samterm": ("cmd/samterm", ["OFILES"], []),
    "vncs": ("cmd/vnc", ["OFILES", "SOFILES"], ["vncs"]),
}

# The files that need a file that the build makes. The guest keeps the object of the image.
KEEP = {"libauthsrv/authpak": "msqrt.mpc, which mpc makes from msqrt.mp"}

# The programs of one file.
ONE = ["cmd/clock", "cmd/colors", "cmd/stats", "cmd/winwatch", "cmd/paint", "games/catclock"]


def read(path, depth=0):
    """The lines of a mkfile, with each relative `<file` include read in its place."""
    try:
        text = open(path, errors="replace").read().replace("\\\n", " ")
    except OSError:
        return []
    lines = []
    for line in text.splitlines():
        m = re.match(r"^<\s*([^|\s]+)\s*$", line)
        if m and not m.group(1).startswith("/") and "$" not in m.group(1) and depth < 4:
            lines += read(os.path.join(os.path.dirname(path), m.group(1)), depth + 1)
        else:
            lines.append(line)
    return lines


def variables(directory):
    """The simple variables of the mkfile of `directory`, with $O and $objtype set."""
    env = {"objtype": objtype, "O": O, "CFLAGS": ""}
    for line in read(os.path.join(SRC, directory, "mkfile")):
        m = re.match(r"^(\w+)\s*=\s*(.*)$", line)
        if not m:
            continue
        value = re.sub(r"`\{[^}]*\}", "", m.group(2))
        env[m.group(1)] = re.sub(r"\$\{?(\w+)\}?", lambda v: env.get(v.group(1), ""), value)
    return env


def rc_words(text):
    """Split a line as rc does: only single quotes quote, and '' in quotes is one quote."""
    words, word, quoted, have, i = [], "", False, False, 0
    while i < len(text):
        c = text[i]
        if quoted:
            if c == "'" and text[i + 1:i + 2] == "'":
                word += "'"
                i += 1
            elif c == "'":
                quoted = False
            else:
                word += c
        elif c == "'":
            quoted = have = True
        elif c in " \t":
            if have or word:
                words.append(word)
            word, have = "", False
        else:
            word += c
        i += 1
    if have or word:
        words.append(word)
    return words


def sources(directory, names=None):
    """The C files that the mkfile of `directory` builds, with no suffix.

    With no `names`, the first of OFILES, CFILES and FILES that names a C file is used. The
    CFILES of libthread names the files of each machine, but its OFILES names only one.
    """
    env = variables(directory)
    found = []
    for var in names or ["OFILES", "CFILES", "FILES"]:
        for word in env.get(var, "").split():
            base = re.sub(r"\.(c|" + O + r")$", "", word)
            if (base != word or var == "FILES") and os.path.exists(os.path.join(SRC, directory, base + ".c")):
                found.append(base)
        if found and not names:
            break
    if directory.endswith("/port"):
        arch = os.path.join(SRC, os.path.dirname(directory), objtype)
        if os.path.isdir(arch):
            over = {os.path.splitext(f)[0] for f in os.listdir(arch) if f.endswith((".c", ".s"))}
            found = [n for n in found if n not in over]
    return list(dict.fromkeys(found))


def compile_one(directory, name, dest):
    """Compile DIRECTORY/NAME.c into DEST/NAME.O. Return an error line, or None."""
    flags = [w for w in rc_words(variables(directory).get("CFLAGS", "")) if w != "-c"]
    env = dict(os.environ, objtype=objtype, INCLUDE=f"{tree}/{objtype}/include {tree}/sys/include")
    obj = os.path.join(dest, f"{name}.{O}")
    cmd = [rucc, "--driver=kencc", f"--target={TARGET}", f"--sysroot={tree}", "-FTVw", "-I."]
    cmd += flags + ["-o", obj, f"{name}.c"]
    run = subprocess.run(cmd, cwd=os.path.join(SRC, directory), env=env, capture_output=True, text=True)
    if run.returncode != 0:
        first = next((l for l in run.stderr.splitlines() if "error" in l), run.stderr.strip()[:200])
        return f"{directory}/{name}.c: {first}"
    return None


jobs = []
for lib, dirs in LIBS.items():
    dest = os.path.join(out, "lib", lib)
    os.makedirs(dest, exist_ok=True)
    for d in dirs:
        if os.path.isdir(os.path.join(SRC, d)):
            jobs += [(d, n, dest) for n in sources(d) if f"{d}/{n}" not in KEEP]
for prog, (d, names, extra) in CMDS.items():
    dest = os.path.join(out, "cmd", prog)
    os.makedirs(dest, exist_ok=True)
    jobs += [(d, n, dest) for n in sources(d, names) + extra]
os.makedirs(os.path.join(out, "one"), exist_ok=True)
jobs += [(os.path.dirname(p), os.path.basename(p), os.path.join(out, "one")) for p in ONE]

with concurrent.futures.ThreadPoolExecutor(os.cpu_count() or 4) as pool:
    errors = [e for e in pool.map(lambda j: compile_one(*j), jobs) if e]
here = os.path.dirname(os.path.abspath(__file__))
for f in ("gui.rc", "start"):
    shutil.copy(os.path.join(here, f), os.path.join(out, f))
for name, why in KEEP.items():
    print(f"keep {name}: it needs {why}")
for e in errors:
    print("FAIL", e)
print(f"compiled {len(jobs) - len(errors)} of {len(jobs)} files")
sys.exit(1 if errors else 0)
