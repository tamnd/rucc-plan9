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

**Status.** The AppKit back end is in `h9-gui/src/cocoa.rs`. It calls the Objective-C runtime directly and needs no crate. The first version copies the screen into a `CGImage` once for each frame that changed, at most 60 times a second, and sets the image as the contents of the layer of the view. The layer scales the image with the nearest pixel, so on a 2x display each Plan 9 pixel is 2x2 device pixels. The window does not resize the Plan 9 screen yet. The `IOSurface`, the resize and the key-up events come later. The X11 back end is in `h9-gui/src/x11.rs`. It does not use Xlib. It speaks the X11 protocol on the socket of `DISPLAY` (a Unix socket for `:N`, or TCP port 6000+N on a host), with the `MIT-MAGIC-COOKIE-1` of `XAUTHORITY` or `~/.Xauthority`. Thus it needs no library, and there is nothing to load with `dlopen`. It needs a 24-bit `TrueColor` root visual with 32-bit little-endian pixels, which is the screen format, so no pixel is converted. The window has the size of the screen, and its size hints tell the window manager not to resize it. Each frame that changed goes to the window with `PutImage`, in strips of rows that each fit in one request, at most 60 times a second. MIT-SHM comes later. The keys go through the keysyms of `GetKeyboardMapping`, and buttons 1, 2 and 3 and the wheel go to `/dev/mouse` as they are. A close of the window ends the session, as on macOS. CI runs a test that draws in a window of Xvfb and reads the pixels back. The Wayland back end is in `h9-gui/src/wayland.rs`. It does not use libwayland. It speaks the Wayland protocol on the socket of `WAYLAND_DISPLAY` in `XDG_RUNTIME_DIR`, and it sends the file of the shared memory with `SCM_RIGHTS`. It binds `wl_compositor`, `wl_shm`, `xdg_wm_base` and `wl_seat`. Each frame that changed goes to one of two `XRGB8888` buffers in one `wl_shm` pool, at most 60 times a second. The keys go through the keymap that the compositor sends, which the module reads for the levels of Shift and AltGr. The monitor makes the key repeat itself with the rate of `repeat_info`. The window asks for decorations from the server with `xdg-decoration` when the compositor has it. GNOME does not have it, so on GNOME the window has no title bar, and `H9_GUI=x11` uses XWayland. The scale is 1, and the clipboard and the pointer lock come later. CI runs a test that draws in a window of sway on a headless output and reads the pixels back with grim.

drawterm is MIT licensed. Where `h9-gui` ports its logic from C to Rust, as for the key tables, the modifier handling and the Wayland clipboard dance, the module says so. The monitor links no drawterm code, because drawterm's back ends call into its own kernel (`absmousetrack`, `kbdkey`, `screenputs`). `9h` has the real kernel in the guest and only needs the host half.

**Choice at run time.** `9h` uses Wayland if `WAYLAND_DISPLAY` is set and X11 if `DISPLAY` is set, and headless otherwise or with `--headless`. Both Linux back ends are built into one binary. They speak the protocols on the sockets and load no library, so the binary runs on a host with neither. `H9_GUI=wayland` or `H9_GUI=x11` sets the back end.

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

`kbdfs` reads `/dev/kbd` if the kernel provides one, and `/dev/scancode` otherwise (`kbdfs.c:1664`). `devh9kbd.c` provides `#b/kbd`, which carries messages of `r` or `R` and one UTF-8 rune, each NUL-terminated: key down and key up (`kbdfs.c:543`). Before this device, typed text came on the console only, and `kbdfs` gave `c` messages only. Programs that read the `k` and `K` messages of `/dev/kbd`, such as `doom`, did not get the keys. This way:
- the **host's layout and input method** produce the runes, so a French or Japanese user types as on any other host program, and `kbdfs` does not need a `kbmap`;
- programs that read key-up events, such as `doom` and the emulators, still get them.

The monitor's key table maps:
- host modifiers to `Kshift`, `Kctl` and `Kalt`;
- AltGr to `Kaltgr`;
- macOS Command and the Linux Super key to `Kmod4`;
- arrows, Home, End, Page Up/Down, Insert, Delete, Escape and F1–F12 to their `keyboard.h` runes.

The host window gives a key down when a key goes down and a key up when it goes up, so a program sees a key that the user holds. A repeat of the host gives one more key down and no key up. `kbdfs` types the rune again but does not send one more `k` message. Text from an input method, which has no key of its own, gives a key down and a key up for each rune. The `keys` step of a script also gives a key down and a key up for each rune. When the window loses the keyboard focus, each key and modifier that is down goes up. While `#b/kbd` is not open, a key down types its rune on the console, and a key up and a modifier key do nothing.

On X11, the auto-repeat of the server gives a key release and a key press with the same time. The window waits 2 milliseconds after a release for that press, and it gives the pair as one more key down. The code is `Held` in `h9-gui/src/lib.rs`, which both back ends use.

The monitor remembers which rune each physical key produced when it went down. It sends the key-up for that rune, not for whatever the key would produce now, so a key-up is never lost when a modifier changes in between.

**Compose.** 9front's compose sequence is `Kalt` followed by two characters, done in `kbdfs`. On macOS, Option is also the system's dead-key modifier. drawterm's rule applies: Option alone sends `Kalt` down and up when it goes up, so Option-tap starts a Plan 9 compose, and Option held with a key goes through the macOS input method. Option with a click of the mouse (button 2) or with another modifier is not a tap. On X11, Alt sends `Kalt` when it goes down, as the Alt key of a PC does on 9front. `--keys=plan9` turns off the input method for users who want raw keys.

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

9front's `/dev/audio` is signed 16-bit little-endian stereo at 44,100 Hz by default. `audioh9.c` is an `audio` card for port's `devaudio`, and the file is the same on arm64 and amd64. A write copies the samples to a buffer of 4 KiB in the kernel and gives them to the monitor with `H9audiowrite`, with the speed and the master volume. The samples wait in a queue of 256 KiB in the monitor. When the queue is full, the call takes fewer bytes, and the kernel tries again after 10 ms. As in `audiohda`, a write returns when the queue has at most `delay` samples, which `H9audiobuffered` tells. Thus the player of the host sets the pace, and the card needs no interrupt. The monitor scales the samples by the master volume. A player of the monitor takes the samples from the queue on its own thread:
- on macOS, an output queue of `AudioToolbox` on the default output device, with 4 buffers of 1024 samples, about 93 ms at 44,100 Hz;
- on Linux, the first of `pacat`, `pw-cat` and `aplay` that is on the path, which reads the samples on its standard input. A program is simpler than a client library of PipeWire, PulseAudio or ALSA, and the snarf device uses programs for the clipboard in the same way;
- with `--audio none`, a clock that drops the samples at the speed of the stream;
- with `--audio FILE`, the same clock, which writes the samples to the WAV file FILE.

The clock gives a program the same pace as a sound card, and the file holds the exact bytes that the program wrote, scaled by the volume. `r9 9h boot` and `r9 gui run` give `--audio none`, or `--audio FILE` with their `--audio`, so a test never plays on the host. When the speed changes, the player starts again at the new speed.

Recording (reading `/dev/audio`) is not supported yet. A later `--audio-in` will turn it on, because on macOS it triggers the microphone permission prompt.

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

**Status.** The first version of `r9 hostcheck` is done. It boots the kernel with `--snarf host` and with its screen in a window of the host. In the console window of `rio`, it starts the probe of `guest/probe` in a window that covers the screen. The probe is built in the guest with the compilers of the tree. It writes a line to the kernel console for each message of `/dev/kbd`, for each change of the buttons in `/dev/mouse`, and for each read and write of `/dev/snarf`. Then a driver gives the input, and `r9` compares the lines of the probe with the input. The checks are: each key of the US layout with and without Shift, a modifier change, Control, the key ups, button 1 at points near the four corners and at the centre, buttons 2 and 3, the wheel, a host copy that `rio` reads from `/dev/snarf`, and a write to `/dev/snarf` that goes to the host clipboard. The `x11` driver gives the input with XTest through `xdotool` and uses `xclip`, and the job `9h amd64` runs it on Xvfb. The `control` driver gives the input on the control socket of the monitor, so it checks only the probe, the guest and `r9`. The `wayland` driver runs on sway on a headless output, and the job `9h amd64` runs it after the `x11` driver. A headless sway has no input devices, and its seat then has no pointer, so `swaymsg seat cursor` cannot reach a window. `wtype` makes a new keyboard for each call, and the compositor forgets the keys of a keyboard when it goes, so a Shift key that is down cannot stay down between two calls. Thus `9h vinput` is one Wayland client for the whole check. It makes a virtual keyboard with `zwp_virtual_keyboard_v1` and the US keymap, and a virtual pointer with `zwlr_virtual_pointer_v1`, and it does the steps that `r9` gives on its standard input. `r9` finds the window with `swaymsg -t get_tree`, gives it the focus, and uses `wl-copy` and `wl-paste` for the clipboard. The `cocoa` driver is the default on macOS. It gives the input with the step `post` of the control socket of 9h. The window makes an `NSEvent` for each step and sends it with `[NSApp sendEvent:]`, so the event goes through the window to the view, as an event of a person does. The driver does not use `CGEventPost`, because that needs the permission of Accessibility. Each posted key event has the keyboard type of an ANSI keyboard, because the layout reads the key codes for the keyboard type, and on a JIS keyboard key 24 gives `^`. At the end, the probe writes a pattern of 32 KiB to `/dev/audio`, and `r9` looks for the same bytes in the WAV file of the monitor. The check makes no sound. It checks the audio driver of the guest, the device of the monitor and its timing, but not the audio output of the host. The snarf checks need a root with `bind -qa '#Z' /dev` in `/rc/bin/termrc.local`, as the roots of `r9 image` have. The layouts, compose, the scale 2, the warp, the `acme` chord, the resize, the audio output of the host and the quit come later.

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
