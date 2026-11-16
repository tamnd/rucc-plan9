//! The keyboard of the 9h kernel (`devh9kbd.c`). The kernel serves `#b/kbd`, and `kbdfs` reads
//! it in place of `/dev/scancode`. Each event is one rune and a direction, key down or key up, so
//! `kbdfs` gives the `k` and `K` messages of `/dev/kbd` to programs such as `games/doom` and the
//! emulators, which act on keys that are held. The interrupt of the device is high while an event
//! waits.
//!
//! The kernel tells the device when `#b/kbd` is open. Typed text goes to the keyboard only while
//! it is open, and to the console at other times, for example at the prompts of the boot before
//! `kbdfs` starts.

use std::collections::VecDeque;

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The most events that wait.
const MAX: usize = 1024;

/// The keyboard device.
pub struct H9Kbd {
    irq: u32,
    open: bool,
    events: VecDeque<(char, bool)>,
    /// The first bytes of a rune that the last text did not complete.
    partial: Vec<u8>,
}

impl H9Kbd {
    /// A keyboard with the interrupt `irq`.
    #[must_use]
    pub fn new(irq: u32) -> H9Kbd {
        H9Kbd {
            irq,
            open: false,
            events: VecDeque::new(),
            partial: Vec::new(),
        }
    }

    /// True while the kernel has `#b/kbd` open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// A key down, `down` true, or a key up of the rune `r`. When `MAX` events wait, the event is
    /// dropped.
    pub fn push(&mut self, r: char, down: bool) {
        if self.events.len() < MAX {
            self.events.push_back((r, down));
        }
    }

    /// Typed text: a key down and a key up for each rune of the UTF-8 text `b`. A rune can start
    /// at the end of one text and end in the next text. Bytes that are not UTF-8 are dropped.
    pub fn type_in(&mut self, b: &[u8]) {
        let mut text = std::mem::take(&mut self.partial);
        text.extend_from_slice(b);
        let mut s = &text[..];
        while !s.is_empty() {
            let (n, skip) = match std::str::from_utf8(s) {
                Ok(_) => (s.len(), Some(0)),
                Err(e) => (e.valid_up_to(), e.error_len()),
            };
            for r in std::str::from_utf8(&s[..n]).unwrap_or_default().chars() {
                self.push(r, true);
                self.push(r, false);
            }
            let Some(skip) = skip else {
                // The text ends in the first bytes of a rune.
                self.partial = s[n..].to_vec();
                break;
            };
            s = &s[n + skip..];
        }
    }

    /// The number of events that the guest did not read yet.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.events.len()
    }
}

impl Device for H9Kbd {
    fn name(&self) -> &'static str {
        "h9kbd"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn irq(&self) -> Option<(u32, bool)> {
        Some((self.irq, !self.events.is_empty()))
    }

    fn call(&mut self, x: &[u64; 6], _: &Ram) -> Option<u64> {
        match x[0] as u32 {
            h9::KBD_GET => Some(
                self.events
                    .pop_front()
                    .map_or(u64::MAX, |(r, down)| u64::from(r) | u64::from(down) << 32),
            ),
            h9::KBD_OPEN => {
                self.open = x[1] != 0;
                if !self.open {
                    self.events.clear();
                }
                Some(0)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{H9Kbd, MAX};
    use crate::{Bus, h9};
    use h9_vm::Ram;

    fn call(bus: &mut Bus, ram: &Ram, n: u32, a: u64) -> u64 {
        bus.call(&[u64::from(n), a, 0, 0, 0, 0], ram)
    }

    #[test]
    fn the_keyboard_gives_a_down_and_an_up_for_each_rune() {
        let ram = Ram::new(0x4000_0000, 0x1000).unwrap();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Kbd::new(h9::KBD_SPI)));
        assert_eq!(bus.irqs(), vec![(h9::KBD_SPI, false)]);
        assert_eq!(call(&mut bus, &ram, h9::KBD_GET, 0), u64::MAX);
        assert!(!bus.hyper_device::<H9Kbd>().unwrap().is_open());
        assert_eq!(call(&mut bus, &ram, h9::KBD_OPEN, 1), 0);
        let k = bus.hyper_device::<H9Kbd>().unwrap();
        assert!(k.is_open());
        k.type_in("a\u{f00e}".as_bytes());
        assert_eq!(k.pending(), 4);
        assert_eq!(bus.irqs(), vec![(h9::KBD_SPI, true)]);
        assert_eq!(call(&mut bus, &ram, h9::KBD_GET, 0), 0x61 | 1 << 32);
        assert_eq!(call(&mut bus, &ram, h9::KBD_GET, 0), 0x61);
        assert_eq!(call(&mut bus, &ram, h9::KBD_GET, 0), 0xf00e | 1 << 32);
        assert_eq!(call(&mut bus, &ram, h9::KBD_GET, 0), 0xf00e);
        assert_eq!(call(&mut bus, &ram, h9::KBD_GET, 0), u64::MAX);
        assert_eq!(bus.irqs(), vec![(h9::KBD_SPI, false)]);
    }

    #[test]
    fn a_close_drops_the_events_that_wait() {
        let ram = Ram::new(0x4000_0000, 0x1000).unwrap();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Kbd::new(h9::KBD_SPI)));
        call(&mut bus, &ram, h9::KBD_OPEN, 1);
        bus.hyper_device::<H9Kbd>().unwrap().type_in(b"xy");
        assert_eq!(call(&mut bus, &ram, h9::KBD_OPEN, 0), 0);
        let k = bus.hyper_device::<H9Kbd>().unwrap();
        assert!(!k.is_open());
        assert_eq!(k.pending(), 0);
    }

    #[test]
    fn a_rune_can_be_in_two_texts() {
        let mut k = H9Kbd::new(h9::KBD_SPI);
        k.type_in(b"a\xef\x80");
        k.type_in(b"\x98\xffb");
        let got: Vec<(char, bool)> = k.events.iter().copied().collect();
        let want = [
            ('a', true),
            ('a', false),
            ('\u{f018}', true),
            ('\u{f018}', false),
            ('b', true),
            ('b', false),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn the_queue_has_a_limit() {
        let mut k = H9Kbd::new(h9::KBD_SPI);
        for _ in 0..MAX {
            k.push('a', true);
        }
        k.push('b', true);
        assert_eq!(k.pending(), MAX);
    }
}
