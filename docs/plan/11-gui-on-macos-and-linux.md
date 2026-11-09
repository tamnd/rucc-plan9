# The GUI on macOS and Linux

The GUI claim of 02.7 has two parts:
- **The programs work.** Every program in the GUI census runs unmodified under `9h` and passes its script. Document 12 tests that, and none of it depends on a host window: the scripts run headless.
- **The session is usable on the host.** A person can use `rio`, `acme`, `sam`, `page` and `doom` on a Mac or a Linux desktop as they would on a 9front machine, with the host's keyboard, mouse, clipboard, sound and display. That is this document, and 11.9 checks it.

All of this lives in the `9h` monitor's `h9-gui` crate (10.8). Nothing here runs in the guest except the drivers of 10.2.

## 11.1 Modes

**Screen mode** is the claim. A `9h` session is one host window. The Plan 9 screen is that window's content, `rio` runs inside it, and resizing the window resizes the Plan 9 screen. drawterm works the same way, and it is the model.

**Rootless mode** shows each `rio` window as its own host window. It is not in the claim. It is open question 4, and 11.7 explains why it is hard.

## 11.2 Back ends

| Host | Back end | Derived from | Presentation |
|---|---|---|---|
| macOS 15+ (arm64) | AppKit, an `NSWindow` with a layer-backed view | drawterm `gui-cocoa` | the framebuffer is copied into an `IOSurface` and set as the layer's contents |
| Linux, Wayland | `xdg-shell`, `wl_shm`, `xdg-decoration`, `wp_viewporter`, `zwp_primary_selection` | drawterm `gui-wl` | double-buffered `wl_shm` `XRGB8888`, with damage per flushed rectangle |
| Linux, X11 | Xlib with MIT-SHM | drawterm `gui-x11` | `XShmPutImage` per flushed rectangle |
| any | headless | none | no window; the framebuffer is read only by the test hooks (10.8) |

drawterm is MIT licensed. Where `h9-gui` ports its logic from C to Rust, as for the key tables, the modifier handling and the Wayland clipboard dance, the module says so. The monitor links no drawterm code, because drawterm's back ends call into its own kernel (`absmousetrack`, `kbdkey`, `screenputs`). `9h` has the real kernel in the guest and only needs the host half.

**Choice at run time.** `9h` uses Wayland if `WAYLAND_DISPLAY` is set and X11 if `DISPLAY` is set, and headless otherwise or with `--headless`. Both Linux back ends are built into one binary, and the Wayland and X11 libraries are loaded with `dlopen` so the binary runs on a host with neither.

## 11.3 Pixels

**Format.** `h9screen.c` allocates the screen as `XRGB32` (`x8r8g8b8`), so in memory it is `B, G, R, X` in byte order. That is `WL_SHM_FORMAT_XRGB8888`, X11's 24-bit TrueColor on little-endian hosts, and `kCVPixelFormatType_32BGRA` on macOS with the alpha ignored. No pixel is converted.

**Flushing.** `devdraw` calls `flushmemscreen(r)` after each drawing batch. `h9screen.c` turns that into a `screen.flush(r)` hypercall. The monitor does not copy the pixels on the call. It adds `r` to a damage region and returns. The presentation thread copies the damaged region from guest memory to the host buffer once per display refresh (`CVDisplayLink`, `wl_surface.frame`, or a 60 Hz timer on X11). So a program that flushes 10,000 times a second costs 10,000 cheap hypercalls and 60 copies.

**Tearing.** The copy reads guest memory while the guest may be drawing the next frame. A frame can show part of the next batch, as on real hardware with no vsync. That only affects what a person sees. Tests read `/dev/wsys/N/window` inside the guest, never the host buffer (12.6).

**Size and resize.** The Plan 9 screen has the window's size in pixels. When the host resizes the window, the monitor raises the screen's resize interrupt. `h9screen.c` then reallocates the framebuffer and calls `deletescreenimage` and `resetscreenimage` as `pc/screen.c` does after a mode change, and port's `mouseresize` tells `rio`, which redraws.

**HiDPI.** On a Retina display, one point is 2×2 device pixels. `--scale` decides how Plan 9 pixels map to them:
- `--scale=1` (the default on a 1× display) gives one Plan 9 pixel per device pixel;
- `--scale=2` (the default on a 2× display) gives each Plan 9 pixel 2×2 device pixels, nearest-neighbour, so 9front's bitmap fonts are their normal size;
- `--scale=device` gives one Plan 9 pixel per device pixel on a 2× display. Text is sharp but small, unless the user picks a large font, as on a 4K panel running 9front natively.

The monitor scales mouse coordinates the same way, so `absmousetrack` always receives Plan 9 pixels.

## 11.4 Keyboard

`kbdfs` reads `/dev/kbd` if the kernel provides one, and `/dev/scancode` otherwise (`kbdfs.c:1664`). `h9kbd.c` provides `#b/kbd`, which carries messages of `r` or `R` and one UTF-8 rune, each NUL-terminated: key down and key up (`kbdfs.c:543`). This way:
- the **host's layout and input method** produce the runes, so a French or Japanese user types as on any other host program, and `kbdfs` does not need a `kbmap`;
- programs that read key-up events, such as `doom` and the emulators, still get them.

The monitor's key table maps:
- host modifiers to `Kshift`, `Kctl` and `Kalt`;
- AltGr to `Kaltgr`;
- macOS Command and the Linux Super key to `Kmod4`;
- arrows, Home, End, Page Up/Down, Insert, Delete, Escape and F1–F12 to their `keyboard.h` runes.

The monitor remembers which rune each physical key produced when it went down. It sends the key-up for that rune, not for whatever the key would produce now, so a key-up is never lost when a modifier changes in between.

**Compose.** 9front's compose sequence is `Kalt` followed by two characters, done in `kbdfs`. On macOS, Option is also the system's dead-key modifier. drawterm's rule applies: Option alone sends `Kalt` down and up, so Option-tap starts a Plan 9 compose, and Option held with a key goes through the macOS input method. `--keys=plan9` turns off the input method for users who want raw keys.

**Reserved host keys.** The host keeps Command-Q (quit `9h`), Command-H, Command-M and Command-Tab on macOS, and nothing on Linux. Everything else goes to Plan 9.

## 11.5 Mouse

**Position.** The monitor sends absolute positions, so port's `absmousetrack` sets the Plan 9 pointer where the host pointer is. There is no acceleration and no drift.

**Buttons.** These follow drawterm's rules:
- three-button mice map directly;
- on a one-button mouse or a trackpad, Option-click is button 2 and Command-click is button 3;
- the wheel is buttons 4 and 5 (`8` and `16` in `/dev/mouse`).

A trackpad's continuous scroll accumulates deltas and emits one button-4 or button-5 click for each line-height of movement. The monitor reads the line height from the host (`NSEvent.scrollingDeltaY` with `hasPreciseScrollingDeltas`, or Wayland's `axis_value120`). Without it, the monitor assumes 40 pixels.

**Chords.** `acme`'s cut, paste and snarf chords need two buttons down at once. They work with a three-button mouse. On a trackpad they work if the modifier is pressed while the first button is held, which drawterm also supports.

**Cursor.** Plan 9 cursors are 16×16 with a `set` and a `clr` mask, plus a hot spot (`Cursor` in `draw.h`). `h9screen.c` passes the cursor to the monitor with `screen.cursor`. The monitor turns it into a host cursor image: black for `set`, white for `clr`, transparent elsewhere, scaled by the HiDPI factor. The cursor is drawn by the host, so it never appears in the framebuffer and never needs masking in tests. `h9screen.c` does not link port's `swcursor.c` for the same reason.

**Warping.** Writing to `/dev/mouse` moves the pointer (`rio` does this on window placement, and `acme` does it to follow the text). `h9mouse.c` passes the move to the monitor:
- macOS uses `CGWarpMouseCursorPosition`, then re-associates the mouse with the cursor;
- X11 uses `XWarpPointer`;
- Wayland has no pointer warp. The monitor uses `zwp_pointer_constraints_v1` to lock the pointer and set a position hint, if the compositor supports it. Otherwise the warp is ignored, and 11.9 records that as a known limit of the host.

## 11.6 Audio

9front's `/dev/audio` is signed 16-bit little-endian stereo at 44,100 Hz by default. `audioh9.c` is an `audio` card for port's `devaudio`. Its output is a ring of 4 KiB buffers in guest memory. The monitor plays the ring through:
- CoreAudio (an `AudioUnit` with `kAudioUnitSubType_DefaultOutput`) on macOS;
- PipeWire, PulseAudio or ALSA on Linux, the first that is present, each loaded with `dlopen`.

When a buffer has played, the monitor raises the card's interrupt, as a DMA engine would. Latency is about 50 ms, the same as `audiohda` with its default buffering. `/dev/volume` maps to the stream's volume.

Recording (reading `/dev/audio`) is off unless `--audio-in` is given, because on macOS it triggers the microphone permission prompt. With it, the input path is the same ring in the other direction.

## 11.7 Rootless mode (open question 4)

`rio` is the window system. Each program draws into an image that `rio` allocates for it on `rio`'s screen, through `/dev/draw` and `/dev/wsys`. The kernel sees one screen, and window boundaries exist only inside `rio`. Putting each window in its own host window needs one of two things:
- **A different window system.** This would be a Plan 9 program that serves `/dev/wsys` the way `rio` does, but tells the monitor each window's rectangle and stacking, so the host can show each as its own window. Programs would stay unmodified, but `rio` would not be running, and `rio` is itself one of the 80 programs.
- **Inferring windows from `rio`'s screen.** `rio` keeps each window in a public `devdraw` screen (`allocscreen`, `/dev/draw/new` with a screen id). In principle the monitor can read port's `devdraw` state through a debug hypercall and find each window's layer. That reads kernel internals, so it breaks when `devdraw.c` changes. The open question asks whether it can be done robustly enough to support.

Neither is planned before P11.

## 11.8 Files and case

macOS's default file system is case-insensitive, and so are some Linux mounts. The tree has 25 paths, in 12 groups, that differ only by case. Examples are `rc/bin/kill` and `rc/bin/Kill`, `sys/lib/troff/font/devutf/HB` and `Hb`, and three `charlib` names `lh`, `lH` and `LH`. None of this reaches the host:
- the build runs in the guest on gefs (05.2);
- `9h`'s root is a disk image (10.6);
- `/mnt/term` serves the user's host files and never the tree.

On the host, `r9 fetch` keeps each tree as a bare git repository. `r9 image sync` writes files into the guest straight from git objects (14.3), so no host ever has a working tree of 9front. The only exception is the GCC comparison of 13.5, which runs on server3's case-sensitive Linux file system.

## 11.9 Host checks

`r9 hostcheck` drives a real `9h` window on a real desktop, injects host input, and checks what arrives in Plan 9. The host input is injected by the OS:
- `CGEventPost` on macOS, which needs the Accessibility permission for the test's terminal;
- XTest on X11;
- Wayland's virtual-keyboard and virtual-pointer protocols, in a headless `weston` or `sway`.

A small probe program in the guest records what it receives: `/dev/kbd` runes, `/dev/mouse` events, `/dev/snarf` contents and `/dev/audio` writes. The probe is test rig, not a claim program.

| Check | macOS (M4) | Linux X11 (gpc) | Linux Wayland (gpc) |
|---|---|---|---|
| Each ASCII key, with and without Shift, gives the right rune | yes | yes | yes |
| Layouts: US, German, French AZERTY and Japanese kana produce the host's characters | yes | yes | yes |
| Key-up events match key-down events, including across a modifier change | yes | yes | yes |
| Compose: Option-tap then `'` `e` gives `é` | yes | Alt-tap | Alt-tap |
| Mouse position matches at the four corners and the centre, at scale 1 and 2 | yes | yes | yes |
| Buttons 1–3, Option- and Command-click, and the wheel | yes | yes | yes |
| `acme` chord: 1 held, 2 clicked cuts | yes | yes | yes |
| Pointer warp from `/dev/mouse` | yes | yes | if the compositor supports pointer constraints |
| Snarf: host copy, then paste in `rio`; snarf in `acme`, then paste on the host | yes | yes (CLIPBOARD) | yes |
| Resize: dragging the window resizes the `rio` screen without corruption | yes | yes | yes |
| Audio: a 1 kHz tone written to `/dev/audio` is captured at the host's output | loopback (BlackHole) | PipeWire monitor | PipeWire monitor |
| Quit: closing the window shuts the guest down cleanly (`fshalt`) | yes | yes | yes |

Every check is scripted. The run is nightly on gpc and on each `h9-gui` change on the M4. A failed check fails the GUI claim's "usable" half (02.7).

## 11.10 Checks

| Claim | Check |
|---|---|
| The host checks of 11.9 pass on each host | `r9 hostcheck --host=m4,gpc-x11,gpc-wayland` |
| No pixel is converted between the guest and the host buffer | `cargo test -p h9-gui` (golden framebuffer through each back end, compared at the host buffer) |
| A resize never shows a frame with a stale or torn screen size | `r9 hostcheck --only=resize --repeat=200` |
| The Linux binary starts on a host with neither Wayland nor X11 | `9h run --headless` in a minimal container |
