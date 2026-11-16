//! The mouse of the 9h kernel (`h9mouse.c`, plan 10.5). The events of the host window and of the
//! session script wait here, and the interrupt of the device is high while one waits. Each event
//! is absolute, in the pixels of the screen.

use std::collections::VecDeque;

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The most events that wait.
const MAX: usize = 256;

/// A mouse event: the point and the buttons, 1 for the left button, 2 for the middle, 4 for the
/// right, and 8 and 16 for the wheel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// The column of the pixel.
    pub x: u16,
    /// The row of the pixel.
    pub y: u16,
    /// The buttons that are down.
    pub buttons: u8,
}

/// The mouse device.
pub struct H9Mouse {
    spi: u32,
    events: VecDeque<Event>,
}

impl H9Mouse {
    /// A mouse with the interrupt `spi`.
    #[must_use]
    pub fn new(spi: u32) -> H9Mouse {
        H9Mouse {
            spi,
            events: VecDeque::new(),
        }
    }

    /// An event from the host. An event that only moves the mouse takes the place of the last
    /// event when that one only moved the mouse too, so a slow guest gets fewer events but each
    /// change of the buttons at its point. When `MAX` events wait, a new change is dropped.
    pub fn push(&mut self, e: Event) {
        let n = self.events.len();
        if n >= 2
            && self.events[n - 2].buttons == e.buttons
            && self.events[n - 1].buttons == e.buttons
        {
            self.events[n - 1] = e;
        } else if n < MAX {
            self.events.push_back(e);
        }
    }

    /// The number of events that the guest did not read yet.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.events.len()
    }
}

impl Device for H9Mouse {
    fn name(&self) -> &'static str {
        "h9mouse"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn irq(&self) -> Option<(u32, bool)> {
        Some((self.spi, !self.events.is_empty()))
    }

    fn call(&mut self, x: &[u64; 6], _: &Ram) -> Option<u64> {
        if x[0] as u32 != h9::MOUSE_GET {
            return None;
        }
        Some(self.events.pop_front().map_or(u64::MAX, |e| {
            u64::from(e.x) | u64::from(e.y) << 16 | u64::from(e.buttons & 0x1f) << 32
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, H9Mouse, MAX};
    use crate::{Bus, h9};
    use h9_vm::Ram;

    fn ev(x: u16, y: u16, buttons: u8) -> Event {
        Event { x, y, buttons }
    }

    #[test]
    fn the_mouse_gives_its_events_and_raises_its_interrupt() {
        let ram = Ram::new(0x4000_0000, 0x1000).unwrap();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Mouse::new(h9::MOUSE_SPI)));
        let get = |bus: &mut Bus| bus.call(&[u64::from(h9::MOUSE_GET), 0, 0, 0, 0, 0], &ram);
        assert_eq!(bus.irqs(), vec![(h9::MOUSE_SPI, false)]);
        assert_eq!(get(&mut bus), u64::MAX);
        let m = bus.hyper_device::<H9Mouse>().unwrap();
        m.push(ev(300, 200, 1));
        m.push(ev(300, 200, 0));
        assert_eq!(bus.irqs(), vec![(h9::MOUSE_SPI, true)]);
        assert_eq!(bus.hyper_device::<H9Mouse>().unwrap().pending(), 2);
        assert_eq!(get(&mut bus), 0x12c | 0xc8 << 16 | 1 << 32);
        assert_eq!(get(&mut bus), 0x12c | 0xc8 << 16);
        assert_eq!(get(&mut bus), u64::MAX);
        assert_eq!(bus.irqs(), vec![(h9::MOUSE_SPI, false)]);
    }

    #[test]
    fn a_move_takes_the_place_of_a_move_but_not_of_a_click() {
        let mut m = H9Mouse::new(h9::MOUSE_SPI);
        m.push(ev(1, 1, 0));
        m.push(ev(2, 2, 1));
        m.push(ev(3, 3, 1));
        m.push(ev(4, 4, 1));
        m.push(ev(5, 5, 0));
        assert_eq!(
            m.events.iter().copied().collect::<Vec<_>>(),
            vec![ev(1, 1, 0), ev(2, 2, 1), ev(4, 4, 1), ev(5, 5, 0)]
        );
        for i in 0..2 * MAX {
            m.push(ev(i as u16, 0, (i % 2) as u8));
        }
        assert_eq!(m.events.len(), MAX);
    }
}
