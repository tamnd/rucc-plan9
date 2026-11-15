//! The console of the 9h kernel (`uarth9.c` and `uarti8250h9.c`, plan 10.5). Output is a hypercall. The bytes that the
//! host types wait here, and the interrupt of the device is high while one waits.

use std::collections::VecDeque;
use std::io::Write;

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The longest write of one `H9consputs` call. A longer call writes this much and returns it, and
/// `uarth9.c` writes the rest in its next call.
const MAX_PUTS: u64 = 64 * 1024;

/// The console device.
pub struct H9Cons {
    spi: u32,
    input: VecDeque<u8>,
    out: Box<dyn Write + Send>,
}

impl H9Cons {
    /// A console with the interrupt `spi` that writes to `out`.
    #[must_use]
    pub fn new(spi: u32, out: Box<dyn Write + Send>) -> H9Cons {
        H9Cons {
            spi,
            input: VecDeque::new(),
            out,
        }
    }

    /// A byte that the host typed.
    pub fn push(&mut self, b: u8) {
        self.input.push_back(b);
    }

    fn put(&mut self, b: &[u8]) {
        // The console of the host is gone when this fails, and the guest cannot do anything
        // about it.
        let _ = self.out.write_all(b).and_then(|()| self.out.flush());
    }
}

impl Device for H9Cons {
    fn name(&self) -> &'static str {
        "h9cons"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn irq(&self) -> Option<(u32, bool)> {
        Some((self.spi, !self.input.is_empty()))
    }

    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        Some(match x[0] as u32 {
            h9::CONS_PUTC => {
                self.put(&[x[1] as u8]);
                0
            }
            h9::CONS_PUTS => {
                let n = x[2].min(MAX_PUTS);
                let mut b = vec![0; n as usize];
                if !ram.read(x[1], &mut b) {
                    return Some(h9::BAD_ADDRESS);
                }
                self.put(&b);
                n
            }
            h9::CONS_GETC => self.input.pop_front().map_or(u64::MAX, u64::from),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::H9Cons;
    use crate::{Bus, h9};
    use h9_vm::Ram;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Out(Arc<Mutex<Vec<u8>>>);
    impl Write for Out {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_console_writes_reads_and_raises_its_interrupt() {
        let ram = Ram::new(0x4000_0000, 0x1_0000).unwrap();
        assert!(ram.write(0x4000_1000, b"Plan 9\n"));
        let out = Out::default();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Cons::new(h9::CONS_SPI, Box::new(out.clone()))));
        let call =
            |bus: &mut Bus, f: u32, a: u64, b: u64| bus.call(&[u64::from(f), a, b, 0, 0, 0], &ram);
        assert_eq!(call(&mut bus, h9::VERSION, 0, 0), h9::REVISION);
        assert_eq!(call(&mut bus, h9::CONS_PUTS, 0x4000_1000, 7), 7);
        assert_eq!(call(&mut bus, h9::CONS_PUTC, u64::from(b'%'), 0), 0);
        assert_eq!(out.0.lock().unwrap().as_slice(), b"Plan 9\n%");
        assert_eq!(
            call(&mut bus, h9::CONS_PUTS, 0x5000_0000, 1),
            h9::BAD_ADDRESS
        );
        assert_eq!(bus.irqs(), vec![(h9::CONS_SPI, false)]);
        bus.hyper_device::<H9Cons>().unwrap().push(b'l');
        assert_eq!(bus.irqs(), vec![(h9::CONS_SPI, true)]);
        assert_eq!(call(&mut bus, h9::CONS_GETC, 0, 0), u64::from(b'l'));
        assert_eq!(call(&mut bus, h9::CONS_GETC, 0, 0), u64::MAX);
        assert_eq!(bus.irqs(), vec![(h9::CONS_SPI, false)]);
        assert_eq!(call(&mut bus, 0xc600_00ff, 0, 0), h9::NOT_SUPPORTED);
    }
}
