//! The snarf buffer of the 9h kernel (`#Z/snarf` of `devhost.c`, plan 10.5 and 11). The kernel
//! reads the host clipboard with `H9snarfsize` and `H9snarfget`, and sets it with `H9snarfput`.
//! The text is UTF-8, as in Plan 9.
//!
//! The clipboard of the host is a program pair: `pbpaste` and `pbcopy` on macOS, and `wl-paste`
//! and `wl-copy`, `xclip` or `xsel` on Linux. A program is simpler than the clipboard calls of
//! `AppKit` or of the display server, and a call of the guest that waits 10 ms for it is not a
//! problem, because a snarf comes from a person.

use std::io::Write;
use std::process::{Command, Stdio};

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// A clipboard.
pub trait Clipboard: Send {
    /// The text in the clipboard. Empty when it has none, or when the clipboard fails.
    fn get(&mut self) -> Vec<u8>;
    /// Put the text in the clipboard.
    fn put(&mut self, text: &[u8]);
}

/// A clipboard in the memory of the monitor, for a session with no host clipboard and for the
/// tests.
#[derive(Debug, Default)]
pub struct Memory(pub Vec<u8>);

impl Clipboard for Memory {
    fn get(&mut self) -> Vec<u8> {
        self.0.clone()
    }

    fn put(&mut self, text: &[u8]) {
        self.0 = text.to_vec();
    }
}

/// The clipboard of the host, through its programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    paste: Vec<String>,
    copy: Vec<String>,
}

fn on_path(prog: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

fn words(s: &str) -> Vec<String> {
    s.split(' ').map(String::from).collect()
}

impl Host {
    /// The clipboard programs of this host, or None when it has none.
    #[must_use]
    pub fn find() -> Option<Host> {
        let pairs: &[(&str, &str, bool)] = &[
            ("pbpaste", "pbcopy", cfg!(target_os = "macos")),
            (
                "wl-paste --no-newline",
                "wl-copy",
                std::env::var_os("WAYLAND_DISPLAY").is_some(),
            ),
            (
                "xclip -selection clipboard -o",
                "xclip -selection clipboard -i",
                std::env::var_os("DISPLAY").is_some(),
            ),
            (
                "xsel -b -o",
                "xsel -b -i",
                std::env::var_os("DISPLAY").is_some(),
            ),
        ];
        pairs.iter().find_map(|&(paste, copy, ok)| {
            let prog = paste.split(' ').next().unwrap_or(paste);
            (ok && on_path(prog)).then(|| Host {
                paste: words(paste),
                copy: words(copy),
            })
        })
    }

    fn command(argv: &[String]) -> Command {
        let mut c = Command::new(&argv[0]);
        // pbcopy and pbpaste use the encoding of the locale, and UTF-8 is that of Plan 9.
        c.args(&argv[1..]).env("LC_CTYPE", "UTF-8");
        c
    }
}

impl Clipboard for Host {
    fn get(&mut self) -> Vec<u8> {
        Host::command(&self.paste)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default()
    }

    fn put(&mut self, text: &[u8]) {
        let child = Host::command(&self.copy)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut w) = child.stdin.take() {
                let _ = w.write_all(text);
            }
            let _ = child.wait();
        }
    }
}

/// The device of the snarf buffer.
pub struct H9Snarf {
    clip: Box<dyn Clipboard>,
    buf: Vec<u8>,
}

impl H9Snarf {
    /// A device that uses the clipboard `clip`.
    #[must_use]
    pub fn new(clip: Box<dyn Clipboard>) -> H9Snarf {
        H9Snarf {
            clip,
            buf: Vec::new(),
        }
    }
}

impl Device for H9Snarf {
    fn name(&self) -> &'static str {
        "h9snarf"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        Some(match x[0] as u32 {
            h9::SNARF_SIZE => {
                self.buf = self.clip.get();
                self.buf.truncate(h9::SNARF_MAX as usize);
                self.buf.len() as u64
            }
            h9::SNARF_GET => {
                let off = (x[3] as usize).min(self.buf.len());
                let n = (x[2] as usize).min(self.buf.len() - off);
                if n > 0 && !ram.write(x[1], &self.buf[off..off + n]) {
                    return Some(h9::BAD_ADDRESS);
                }
                n as u64
            }
            h9::SNARF_PUT => {
                if x[2] > h9::SNARF_MAX {
                    return Some(h9::IO_ERROR);
                }
                let mut b = vec![0; x[2] as usize];
                if !b.is_empty() && !ram.read(x[1], &mut b) {
                    return Some(h9::BAD_ADDRESS);
                }
                self.clip.put(&b);
                self.buf = b;
                x[2]
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{H9Snarf, Host, Memory, words};
    use crate::{Bus, h9};
    use h9_vm::Ram;

    #[test]
    fn the_snarf_goes_both_ways() {
        let ram = Ram::new(0x4000_0000, 0x1_0000).unwrap();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Snarf::new(Box::new(Memory(
            "from the host ☺".into(),
        )))));
        let mut call =
            |f: u32, a: u64, b: u64, c: u64| bus.call(&[u64::from(f), a, b, c, 0, 0], &ram);
        let n = call(h9::SNARF_SIZE, 0, 0, 0);
        assert_eq!(n, "from the host ☺".len() as u64);
        assert_eq!(call(h9::SNARF_GET, 0x4000_0000, 4, 0), 4);
        assert_eq!(call(h9::SNARF_GET, 0x4000_0004, 100, 4), n - 4);
        assert_eq!(call(h9::SNARF_GET, 0x4000_0004, 100, n + 5), 0);
        let mut b = vec![0; n as usize];
        assert!(ram.read(0x4000_0000, &mut b));
        assert_eq!(b, "from the host ☺".as_bytes());
        assert_eq!(call(h9::SNARF_GET, 0x5000_0000, 4, 0), h9::BAD_ADDRESS);

        assert!(ram.write(0x4000_0100, b"from rio"));
        assert_eq!(call(h9::SNARF_PUT, 0x4000_0100, 8, 0), 8);
        assert_eq!(call(h9::SNARF_SIZE, 0, 0, 0), 8);
        assert_eq!(call(h9::SNARF_PUT, 0, h9::SNARF_MAX + 1, 0), h9::IO_ERROR);
        assert_eq!(call(h9::SNARF_PUT, 0x5000_0000, 1, 0), h9::BAD_ADDRESS);
        assert_eq!(
            call(h9::SNARF_PUT, 0, 0, 0),
            0,
            "an empty snarf needs no RAM"
        );
        let d = bus.hyper_device::<H9Snarf>().unwrap();
        assert!(d.buf.is_empty());
    }

    #[test]
    fn the_programs_split_into_words() {
        assert_eq!(words("xsel -b -o"), ["xsel", "-b", "-o"]);
        if cfg!(target_os = "macos") {
            assert_eq!(
                Host::find(),
                Some(Host {
                    paste: words("pbpaste"),
                    copy: words("pbcopy")
                })
            );
        }
    }
}
