//! The screen of the 9h machine (`h9screen.c`, plan 10.5 and 11). The framebuffer is in the RAM
//! of the guest. `H9screeninfo` gives the size of the screen, `H9screenset` gives the monitor the
//! address of the framebuffer, and `H9screenflush` gives a rectangle that changed. The monitor
//! copies each such rectangle from the RAM to its own copy of the screen, as it will copy it to
//! the host window. With no window, the copy is what `--snap` writes at the end of a session.
//!
//! The only pixel format is `XRGB32` of `draw.h`: a little-endian word of 0x00RRGGBB for each
//! pixel.

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The channel descriptor of `XRGB32`: x8r8g8b8.
pub const XRGB32: u64 = 0x6808_1828;
/// The largest width or height.
pub const MAX: u32 = 8192;

/// The framebuffer that the guest gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frame {
    pa: u64,
}

/// The screen.
pub struct H9Screen {
    width: u32,
    height: u32,
    frame: Option<Frame>,
    /// The copy of the screen, one 0x00RRGGBB word for each pixel.
    pixels: Vec<u32>,
    /// The number of flushes.
    pub flushes: u64,
}

impl H9Screen {
    /// A screen of `width` by `height` pixels.
    #[must_use]
    pub fn new(width: u32, height: u32) -> H9Screen {
        H9Screen {
            width,
            height,
            frame: None,
            pixels: vec![0; width as usize * height as usize],
            flushes: 0,
        }
    }

    /// The size of the screen.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// True when the guest gave a framebuffer.
    #[must_use]
    pub fn on(&self) -> bool {
        self.frame.is_some()
    }

    /// The pixel at (x, y), 0x00RRGGBB.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }

    /// The number of different colours on the screen.
    #[must_use]
    pub fn colours(&self) -> usize {
        let mut c: Vec<u32> = self.pixels.clone();
        c.sort_unstable();
        c.dedup();
        c.len()
    }

    /// The screen as a PNG file.
    #[must_use]
    pub fn png(&self) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(self.pixels.len() * 3);
        for &p in &self.pixels {
            rgb.extend_from_slice(&p.to_be_bytes()[1..]);
        }
        crate::png::rgb(self.width, self.height, &rgb)
    }

    fn set(&mut self, x: &[u64; 6], ram: &Ram) -> u64 {
        if x[2] != u64::from(self.width) || x[3] != u64::from(self.height) || x[4] != XRGB32 {
            return h9::IO_ERROR;
        }
        let len = u64::from(self.width) * u64::from(self.height) * 4;
        let mut probe = [0u8; 1];
        if !ram.read(x[1], &mut probe) || !ram.read(x[1] + len - 1, &mut probe) {
            return h9::BAD_ADDRESS;
        }
        self.frame = Some(Frame { pa: x[1] });
        self.copy(0, 0, self.width, self.height, ram);
        0
    }

    fn copy(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, ram: &Ram) {
        let Some(f) = self.frame else { return };
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let n = (x1 - x0) as usize;
        let mut row = vec![0u8; n * 4];
        for y in y0..y1 {
            let at = y * self.width + x0;
            if !ram.read(f.pa + u64::from(at) * 4, &mut row) {
                return;
            }
            let dst = &mut self.pixels[at as usize..at as usize + n];
            for (d, s) in dst.iter_mut().zip(row.as_chunks::<4>().0) {
                *d = u32::from_le_bytes([s[0], s[1], s[2], 0]);
            }
        }
    }
}

/// The x and the y of a point of `H9screenflush`, x | y<<16.
fn point(v: u64) -> (u32, u32) {
    ((v & 0xffff) as u32, ((v >> 16) & 0xffff) as u32)
}

impl Device for H9Screen {
    fn name(&self) -> &'static str {
        "h9screen"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        Some(match x[0] as u32 {
            h9::SCREEN_INFO => u64::from(self.width) | u64::from(self.height) << 16,
            h9::SCREEN_SET => self.set(x, ram),
            h9::SCREEN_FLUSH => {
                let (x0, y0) = point(x[1]);
                let (x1, y1) = point(x[2]);
                self.copy(x0, y0, x1, y1, ram);
                self.flushes += 1;
                0
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{H9Screen, XRGB32};
    use crate::{Bus, h9};
    use h9_vm::Ram;

    const FB: u64 = 0x4000_0000;

    #[test]
    fn the_monitor_copies_each_flushed_rectangle() {
        let ram = Ram::new(FB, 0x1_0000).unwrap();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Screen::new(16, 8)));
        let mut call =
            |f: u32, a: u64, b: u64, c: u64, d: u64| bus.call(&[u64::from(f), a, b, c, d, 0], &ram);
        assert_eq!(call(h9::SCREEN_INFO, 0, 0, 0, 0), 16 | 8 << 16);
        assert_eq!(call(h9::SCREEN_SET, FB, 16, 8, 0x1234), h9::IO_ERROR);
        assert_eq!(call(h9::SCREEN_SET, FB, 15, 8, XRGB32), h9::IO_ERROR);
        assert_eq!(
            call(h9::SCREEN_SET, FB + 0x1_0000 - 16, 16, 8, XRGB32),
            h9::BAD_ADDRESS
        );
        assert!(ram.write(FB, &[0x11, 0x22, 0x33, 0xff]));
        assert_eq!(call(h9::SCREEN_SET, FB, 16, 8, XRGB32), 0);
        // A pixel that is not flushed stays as it was.
        assert!(ram.write(FB + (2 * 16 + 3) * 4, &[1, 2, 3, 0]));
        assert!(ram.write(FB + (5 * 16 + 9) * 4, &[4, 5, 6, 0]));
        assert_eq!(call(h9::SCREEN_FLUSH, 3 | 2 << 16, 4 | 3 << 16, 0, 0), 0);
        assert_eq!(call(h9::SCREEN_FLUSH, 0, 0x64 | 0x64 << 16, 0, 0), 0);
        let s = bus.hyper_device::<H9Screen>().unwrap();
        assert!(s.on());
        assert_eq!(s.pixel(0, 0), 0x0033_2211);
        assert_eq!(s.pixel(3, 2), 0x0003_0201);
        assert_eq!(s.pixel(9, 5), 0x0006_0504);
        assert_eq!(s.colours(), 4);
        assert_eq!(s.flushes, 2);
        let png = s.png();
        assert!(png.starts_with(b"\x89PNG"));
    }

    #[test]
    fn a_flush_before_the_framebuffer_does_nothing() {
        let ram = Ram::new(FB, 0x1_0000).unwrap();
        let mut s = H9Screen::new(4, 4);
        assert_eq!(
            crate::Device::call(
                &mut s,
                &[u64::from(h9::SCREEN_FLUSH), 0, 4 | 4 << 16, 0, 0, 0],
                &ram
            ),
            Some(0)
        );
        assert!(!s.on());
        assert_eq!(s.colours(), 1);
    }
}
