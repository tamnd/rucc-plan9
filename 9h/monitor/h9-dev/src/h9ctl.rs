//! The control requests of the 9h kernel (`#Z/ctl` of `devhost.c`, plan 10.5). A request is a
//! line of text that the kernel gives with `H9ctl`:
//! - `quit` ends the session, as `SYSTEM_OFF` does;
//! - `title TEXT` sets the title of the session: the title of the host terminal now, and that of
//!   the window when the screen exists (plan 11).

use std::io::Write;

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The device of the control requests.
pub struct H9Ctl {
    stop: bool,
    title: String,
    term: Option<Box<dyn Write + Send>>,
}

impl H9Ctl {
    /// A device that writes the title to `term` with the escape of xterm, when `term` is a
    /// terminal. None writes the title nowhere.
    #[must_use]
    pub fn new(term: Option<Box<dyn Write + Send>>) -> H9Ctl {
        H9Ctl {
            stop: false,
            title: String::new(),
            term,
        }
    }

    /// The title that the guest set last.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    fn request(&mut self, text: &str) -> Result<(), ()> {
        let text = text.trim_end_matches('\n');
        let (verb, arg) = text.split_once(' ').unwrap_or((text, ""));
        match verb {
            "quit" if arg.is_empty() => self.stop = true,
            "title" => {
                // The guest gives the text, so no control character goes to the host terminal.
                self.title = arg.chars().filter(|c| !c.is_control()).collect();
                if let Some(t) = &mut self.term {
                    let _ = write!(t, "\x1b]0;{}\x07", self.title);
                    let _ = t.flush();
                }
            }
            _ => return Err(()),
        }
        Ok(())
    }
}

impl Device for H9Ctl {
    fn name(&self) -> &'static str {
        "h9ctl"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        if x[0] as u32 != h9::CTL {
            return None;
        }
        if x[2] > h9::CTL_MAX {
            return Some(h9::IO_ERROR);
        }
        let mut b = vec![0; x[2] as usize];
        if !ram.read(x[1], &mut b) {
            return Some(h9::BAD_ADDRESS);
        }
        let ok = std::str::from_utf8(&b)
            .map_err(|_| ())
            .and_then(|t| self.request(t));
        Some(if ok.is_ok() { x[2] } else { h9::IO_ERROR })
    }

    fn stop(&self) -> bool {
        self.stop
    }
}

#[cfg(test)]
mod tests {
    use super::H9Ctl;
    use crate::{Bus, h9};
    use h9_vm::Ram;
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
    fn the_requests_are_known() {
        let ram = Ram::new(0x4000_0000, 0x1_0000).unwrap();
        let term = Sink::default();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Ctl::new(Some(Box::new(term.clone())))));
        let req = |bus: &mut Bus, t: &[u8]| {
            assert!(ram.write(0x4000_0000, t));
            bus.call(
                &[u64::from(h9::CTL), 0x4000_0000, t.len() as u64, 0, 0, 0],
                &ram,
            )
        };
        assert_eq!(req(&mut bus, b"title acme\x1b]0;x\x07 /usr/glenda\n"), 29);
        assert_eq!(
            String::from_utf8(term.0.lock().unwrap().clone()).unwrap(),
            "\x1b]0;acme]0;x /usr/glenda\x07"
        );
        assert_eq!(req(&mut bus, b"scale 2"), h9::IO_ERROR);
        assert_eq!(req(&mut bus, b"quit now"), h9::IO_ERROR);
        assert_eq!(req(&mut bus, b"\xff"), h9::IO_ERROR);
        assert!(!bus.stopping());
        assert_eq!(req(&mut bus, b"quit\n"), 5);
        assert!(bus.stopping());
        assert_eq!(
            bus.hyper_device::<H9Ctl>().unwrap().title(),
            "acme]0;x /usr/glenda"
        );
        let long = vec![b'a'; 257];
        assert_eq!(req(&mut bus, &long), h9::IO_ERROR);
    }
}
