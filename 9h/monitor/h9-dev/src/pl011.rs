//! A PL011 UART, the console of the `qemu` kernel of the tree (`uartqemu.c`). It is a bring-up
//! device: the 9h kernel has a console of hypercalls instead (plan 10.5).
//!
//! A byte that the guest writes goes out at once, so the transmit FIFO is always empty, and the
//! transmit interrupt is raised after each byte, as in QEMU. The bytes that the host types wait
//! in the receive FIFO.

use std::collections::VecDeque;
use std::io::Write;

const DR: u64 = 0x00;
const FR: u64 = 0x18;
const IBRD: u64 = 0x24;
const FBRD: u64 = 0x28;
const LCRH: u64 = 0x2c;
const CR: u64 = 0x30;
const IFLS: u64 = 0x34;
const IMSC: u64 = 0x38;
const RIS: u64 = 0x3c;
const MIS: u64 = 0x40;
const ICR: u64 = 0x44;

const RXFE: u32 = 1 << 4;
const TXFE: u32 = 1 << 7;
const RXI: u32 = 1 << 4;
const TXI: u32 = 1 << 5;
const RTI: u32 = 1 << 6;

const ID: [u32; 8] = [0x11, 0x10, 0x14, 0x00, 0x0d, 0xf0, 0x05, 0xb1];

/// A PL011 with its output going to `out`.
pub struct Pl011 {
    out: Box<dyn Write + Send>,
    rx: VecDeque<u8>,
    spi: u32,
    ris: u32,
    imsc: u32,
    ibrd: u32,
    fbrd: u32,
    lcrh: u32,
    cr: u32,
    ifls: u32,
}

impl Pl011 {
    /// A UART on the shared peripheral interrupt `spi` that writes to `out`.
    #[must_use]
    pub fn new(spi: u32, out: Box<dyn Write + Send>) -> Pl011 {
        Pl011 {
            out,
            rx: VecDeque::new(),
            spi,
            ris: 0,
            imsc: 0,
            ibrd: 0,
            fbrd: 0,
            lcrh: 0,
            cr: 0x300,
            ifls: 0x12,
        }
    }

    /// A byte from the host, for the guest to read.
    pub fn push(&mut self, b: u8) {
        self.rx.push_back(b);
        self.ris |= RXI | RTI;
    }

    /// The number of bytes from the host that the guest did not read yet.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.rx.len()
    }
}

impl super::Device for Pl011 {
    fn name(&self) -> &'static str {
        "pl011"
    }

    fn read(&mut self, off: u64, _: u8) -> u64 {
        u64::from(match off {
            DR => {
                let b = self.rx.pop_front().unwrap_or(0);
                if self.rx.is_empty() {
                    self.ris &= !(RXI | RTI);
                }
                u32::from(b)
            }
            FR => TXFE | if self.rx.is_empty() { RXFE } else { 0 },
            IBRD => self.ibrd,
            FBRD => self.fbrd,
            LCRH => self.lcrh,
            CR => self.cr,
            IFLS => self.ifls,
            IMSC => self.imsc,
            RIS => self.ris,
            MIS => self.ris & self.imsc,
            0xfe0..=0xffc => ID[((off - 0xfe0) / 4) as usize],
            _ => 0,
        })
    }

    fn write(&mut self, off: u64, _: u8, value: u64) {
        let v = value as u32;
        match off {
            DR => {
                // The console is a log, so a write error is not a fault of the guest.
                let _ = self.out.write_all(&[v as u8]);
                let _ = self.out.flush();
                self.ris |= TXI;
            }
            IBRD => self.ibrd = v,
            FBRD => self.fbrd = v,
            LCRH => self.lcrh = v,
            CR => self.cr = v,
            IFLS => self.ifls = v,
            IMSC => self.imsc = v,
            ICR => {
                self.ris &= !v;
                if !self.rx.is_empty() {
                    self.ris |= RXI;
                }
            }
            _ => {}
        }
    }

    fn irq(&self) -> Option<(u32, bool)> {
        Some((self.spi, self.ris & self.imsc != 0))
    }
}

#[cfg(test)]
mod tests {
    use super::Pl011;
    use crate::Device;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn bytes_go_out_and_come_in_with_their_interrupts() {
        let sink = Sink::default();
        let mut u = Pl011::new(33, Box::new(sink.clone()));
        assert_eq!(u.irq(), Some((33, false)));
        u.write(0x38, 4, (1 << 4 | 1 << 5) as u64);
        for b in b"Plan 9\n" {
            assert_eq!(
                u.read(0x18, 4) & 1 << 5,
                0,
                "the transmit FIFO is never full"
            );
            u.write(0, 4, u64::from(*b));
        }
        assert_eq!(&*sink.0.lock().unwrap(), b"Plan 9\n");
        assert_eq!(u.irq(), Some((33, true)));
        u.write(0x44, 4, 0x7f0);
        assert_eq!(u.irq(), Some((33, false)));
        assert_eq!(u.read(0x18, 4) & 1 << 4, 1 << 4);
        u.push(b'x');
        assert_eq!(u.irq(), Some((33, true)));
        assert_eq!(u.read(0x18, 4) & 1 << 4, 0);
        assert_eq!(u.read(0, 4), u64::from(b'x'));
        assert_eq!(u.irq(), Some((33, false)));
        assert_eq!(u.read(0xfe0, 4), 0x11);
    }
}
