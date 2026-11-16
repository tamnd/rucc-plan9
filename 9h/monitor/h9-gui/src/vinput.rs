//! `9h vinput`: a virtual keyboard and a virtual pointer on the Wayland compositor, for the
//! check of the Wayland back end in `r9 hostcheck` (plan 11.9). The keyboard uses
//! `zwp_virtual_keyboard_v1` with the US layout, and the pointer uses `zwlr_virtual_pointer_v1`.
//! The compositor gives their events to the window that has the focus, as it does for a real
//! keyboard and mouse. One process keeps the two devices for the whole check, because the
//! compositor forgets the keys and the buttons of a device when the device goes.
//!
//! The process reads one step on each line of its standard input:
//!
//! - `key CODE 1` and `key CODE 0`: the Linux key code goes down or up;
//! - `mods MASK`: the modifiers that are down, Shift 1 and Control 4. The compositor does not
//!   compute them from the keys of a virtual keyboard;
//! - `move X Y W H`: the pointer goes to (X, Y) in a layout of W by H pixels;
//! - `button CODE 1` and `button CODE 0`: the Linux button code goes down or up;
//! - `wheel N`: the wheel turns N steps, down when N is more than 0.
//!
//! It writes `ok` when the devices are ready, and `ok` for each step after the compositor did
//! it. For a bad step, it writes `error: ` and the problem.

use crate::wayland::Args;

/// The keymap of the virtual keyboard. The compositor reads it with xkbcommon, which finds the
/// parts of the US layout in its files.
pub const KEYMAP: &str = "xkb_keymap {\n\
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n\
    xkb_types { include \"complete\" };\n\
    xkb_compat { include \"complete\" };\n\
    xkb_symbols { include \"pc+us+inet(evdev)\" };\n\
};\n";

/// The requests of `zwp_virtual_keyboard_v1`.
const KB_KEY: u16 = 1;
const KB_MODIFIERS: u16 = 2;

/// The requests of `zwlr_virtual_pointer_v1`.
const PTR_MOTION_ABSOLUTE: u16 = 1;
const PTR_BUTTON: u16 = 2;
const PTR_AXIS: u16 = 3;
const PTR_FRAME: u16 = 4;
const PTR_AXIS_SOURCE: u16 = 5;
const PTR_AXIS_DISCRETE: u16 = 7;

/// The length of one step of the wheel, as libinput gives it.
const WHEEL_STEP: i32 = 15;

/// The devices of a connection: the keyboard, the pointer and the version of the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Devices {
    /// The object of the virtual keyboard.
    pub keyboard: u32,
    /// The object of the virtual pointer.
    pub pointer: u32,
    /// The version of the pointer. Version 2 has `axis_discrete`.
    pub version: u32,
}

/// The requests of one step, at the time `ms`.
///
/// # Errors
///
/// When the step is not one of the steps of the module.
pub fn requests(line: &str, d: Devices, ms: u32) -> Result<Vec<Vec<u8>>, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let num = |i: usize| -> Result<u32, String> {
        let w = words
            .get(i)
            .ok_or_else(|| format!("{line}: a number is missing"))?;
        w.parse().map_err(|_| format!("{line}: bad number {w}"))
    };
    let state = |i: usize| -> Result<u32, String> {
        match num(i)? {
            s @ (0 | 1) => Ok(s),
            s => Err(format!("{line}: the state {s} is not 0 or 1")),
        }
    };
    let frame = Args::default().on(d.pointer, PTR_FRAME);
    Ok(match words.first().copied().unwrap_or("") {
        "key" => vec![
            Args::default()
                .u(ms)
                .u(num(1)?)
                .u(state(2)?)
                .on(d.keyboard, KB_KEY),
        ],
        "mods" => vec![
            Args::default()
                .u(num(1)?)
                .u(0)
                .u(0)
                .u(0)
                .on(d.keyboard, KB_MODIFIERS),
        ],
        "move" => vec![
            Args::default()
                .u(ms)
                .u(num(1)?)
                .u(num(2)?)
                .u(num(3)?)
                .u(num(4)?)
                .on(d.pointer, PTR_MOTION_ABSOLUTE),
            frame,
        ],
        "button" => vec![
            Args::default()
                .u(ms)
                .u(num(1)?)
                .u(state(2)?)
                .on(d.pointer, PTR_BUTTON),
            frame,
        ],
        "wheel" => {
            let w = words.get(1).copied().unwrap_or("");
            let n: i32 = w
                .parse()
                .map_err(|_| format!("{line}: bad number of steps {w}"))?;
            let value = n.saturating_mul(WHEEL_STEP).saturating_mul(256);
            let mut r = Vec::new();
            if d.version >= 2 {
                // The source 0 is the wheel, and the axis 0 is the vertical axis.
                r.push(Args::default().u(0).on(d.pointer, PTR_AXIS_SOURCE));
                r.push(
                    Args::default()
                        .u(ms)
                        .u(0)
                        .i(value)
                        .i(n)
                        .on(d.pointer, PTR_AXIS_DISCRETE),
                );
            } else {
                r.push(Args::default().u(ms).u(0).i(value).on(d.pointer, PTR_AXIS));
            }
            r.push(frame);
            r
        }
        w => {
            return Err(format!(
                "{w}: the steps are key, mods, move, button and wheel"
            ));
        }
    })
}

#[cfg(target_os = "linux")]
pub use conn::run;

#[cfg(target_os = "linux")]
mod conn {
    use super::{Devices, KEYMAP, requests};
    use crate::wayland::{Args, Event, socket_path, split};
    use std::io::{BufRead, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    /// A connection to the compositor, with blocking reads.
    struct Conn {
        sock: UnixStream,
        buf: Vec<u8>,
        next_id: u32,
        registry: u32,
        /// The globals: the name, the interface and the version.
        globals: Vec<(u32, String, u32)>,
    }

    impl Conn {
        fn id(&mut self) -> u32 {
            let id = self.next_id;
            self.next_id += 1;
            id
        }

        fn send(&self, m: &[u8]) -> Result<(), String> {
            (&self.sock)
                .write_all(m)
                .map_err(|e| format!("cannot send to the compositor: {e}"))
        }

        fn event(&mut self, e: &Event) -> Result<(), String> {
            let mut r = e.reader();
            match (e.id, e.op) {
                (1, 0) => {
                    let object = r.u();
                    let code = r.u();
                    let text = r.s();
                    Err(format!(
                        "the compositor gave the error {code} on the object {object}: {text}"
                    ))
                }
                (id, 0) if id == self.registry => {
                    let name = r.u();
                    let iface = r.s();
                    let version = r.u();
                    self.globals.push((name, iface, version));
                    Ok(())
                }
                _ => Ok(()),
            }
        }

        /// Wait until the compositor did each request before this one.
        fn roundtrip(&mut self) -> Result<(), String> {
            let cb = self.id();
            self.send(&Args::default().u(cb).on(1, 0))?;
            let mut chunk = [0u8; 4096];
            loop {
                for e in split(&mut self.buf) {
                    if e.id == cb {
                        return Ok(());
                    }
                    self.event(&e)?;
                }
                let n = (&self.sock)
                    .read(&mut chunk)
                    .map_err(|e| format!("cannot read from the compositor: {e}"))?;
                if n == 0 {
                    return Err("the compositor closed the connection".into());
                }
                self.buf.extend_from_slice(&chunk[..n]);
            }
        }

        fn global(&self, iface: &str) -> Result<(u32, u32), String> {
            self.globals
                .iter()
                .find(|g| g.1 == iface)
                .map(|g| (g.0, g.2))
                .ok_or_else(|| format!("the compositor has no {iface}"))
        }

        fn bind(&mut self, iface: &str, max: u32) -> Result<(u32, u32), String> {
            let (name, version) = self.global(iface)?;
            let id = self.id();
            let v = version.min(max);
            self.send(
                &Args::default()
                    .u(name)
                    .s(iface)
                    .u(v)
                    .u(id)
                    .on(self.registry, 0),
            )?;
            Ok((id, v))
        }
    }

    /// Give the keymap to the keyboard `kb`. The keymap goes in a file that has no name, as
    /// the protocol gives a keymap by its file descriptor.
    fn keymap(c: &Conn, kb: u32) -> Result<(), String> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, Into::into);
        let path = dir.join(format!("9h-vinput-{}.xkb", std::process::id()));
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = std::fs::remove_file(&path);
        let mut text = KEYMAP.as_bytes().to_vec();
        text.push(0);
        f.write_all(&text)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let size = u32::try_from(text.len()).unwrap_or(u32::MAX);
        // The format 1 is the text of an XKB keymap.
        let m = Args::default().u(1).u(size).on(kb, 0);
        crate::wayland::send_fd(&c.sock, &m, f.as_raw_fd())
    }

    /// Make the devices, and do the steps of `input` until its end. Write the answers to `out`.
    ///
    /// # Errors
    ///
    /// When the compositor cannot be reached, has no virtual devices, or closes the connection.
    pub fn run(input: impl BufRead, mut out: impl Write) -> Result<(), String> {
        let display = std::env::var("WAYLAND_DISPLAY").ok();
        let runtime = std::env::var("XDG_RUNTIME_DIR").ok();
        let path = socket_path(display.as_deref(), runtime.as_deref())
            .ok_or("set WAYLAND_DISPLAY and XDG_RUNTIME_DIR")?;
        let sock = UnixStream::connect(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut c = Conn {
            sock,
            buf: Vec::new(),
            next_id: 2,
            registry: 0,
            globals: Vec::new(),
        };
        c.registry = c.id();
        c.send(&Args::default().u(c.registry).on(1, 1))?;
        c.roundtrip()?;
        let (seat, _) = c.bind("wl_seat", 1)?;
        let (kbm, _) = c.bind("zwp_virtual_keyboard_manager_v1", 1)?;
        let (ptm, version) = c.bind("zwlr_virtual_pointer_manager_v1", 2)?;
        let keyboard = c.id();
        c.send(&Args::default().u(seat).u(keyboard).on(kbm, 0))?;
        keymap(&c, keyboard)?;
        let pointer = c.id();
        c.send(&Args::default().u(seat).u(pointer).on(ptm, 0))?;
        c.roundtrip()?;
        let d = Devices {
            keyboard,
            pointer,
            version,
        };
        let say = |out: &mut dyn Write, s: &str| -> Result<(), String> {
            writeln!(out, "{s}")
                .and_then(|()| out.flush())
                .map_err(|e| e.to_string())
        };
        say(&mut out, "ok")?;
        let start = Instant::now();
        for line in input.lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            let ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
            match requests(&line, d, ms) {
                Ok(rs) => {
                    for r in rs {
                        c.send(&r)?;
                    }
                    c.roundtrip()?;
                    say(&mut out, "ok")?;
                }
                Err(e) => say(&mut out, &format!("error: {e}"))?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Devices = Devices {
        keyboard: 5,
        pointer: 6,
        version: 2,
    };

    fn words(m: &[u8]) -> Vec<u32> {
        m.chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect()
    }

    #[test]
    fn a_key_is_one_request_of_the_keyboard() {
        let r = requests("key 30 1", D, 7).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(words(&r[0]), vec![5, 20 << 16 | 1, 7, 30, 1]);
        assert_eq!(
            words(&requests("mods 5", D, 0).unwrap()[0])[2..],
            [5, 0, 0, 0]
        );
    }

    #[test]
    fn the_pointer_steps_end_with_a_frame() {
        let r = requests("move 10 20 1280 1024", D, 3).unwrap();
        assert_eq!(words(&r[0]), vec![6, 28 << 16 | 1, 3, 10, 20, 1280, 1024]);
        assert_eq!(words(&r[1]), vec![6, 8 << 16 | 4]);
        let r = requests("button 272 0", D, 3).unwrap();
        assert_eq!(words(&r[0])[2..], [3, 272, 0]);
        let r = requests("wheel -1", D, 3).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(
            words(&r[1])[2..],
            [3, 0, (-15 * 256i32).cast_unsigned(), u32::MAX]
        );
        let v1 = Devices { version: 1, ..D };
        assert_eq!(requests("wheel 1", v1, 3).unwrap().len(), 2);
    }

    #[test]
    fn a_bad_step_is_an_error() {
        assert!(requests("key 30 2", D, 0).is_err());
        assert!(requests("key 30", D, 0).is_err());
        assert!(requests("jump 1", D, 0).is_err());
        assert!(requests("wheel up", D, 0).is_err());
    }
}
