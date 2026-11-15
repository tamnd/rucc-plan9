//! A PL031 real time clock, read by `devrtc.c` of the `qemu` kernel. It is a bring-up device
//! (plan 10.5 gives the 9h kernel its time in the device tree). The guest can set the time,
//! which then keeps its offset from the clock of the host.

use std::time::{SystemTime, UNIX_EPOCH};

const DR: u64 = 0x00;
const MR: u64 = 0x04;
const LR: u64 = 0x08;
const CR: u64 = 0x0c;
const IMSC: u64 = 0x10;
const RIS: u64 = 0x14;

const ID: [u32; 8] = [0x31, 0x10, 0x14, 0x00, 0x0d, 0xf0, 0x05, 0xb1];

/// A PL031 that counts the seconds of the host clock.
#[derive(Default)]
pub struct Pl031 {
    offset: i64,
    mr: u32,
    imsc: u32,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().cast_signed())
}

impl Pl031 {
    /// A clock with the time of the host.
    #[must_use]
    pub fn new() -> Pl031 {
        Pl031::default()
    }
}

impl super::Device for Pl031 {
    fn name(&self) -> &'static str {
        "pl031"
    }

    fn read(&mut self, off: u64, _: u8) -> u64 {
        u64::from(match off {
            DR => (now() + self.offset).cast_unsigned() as u32,
            MR => self.mr,
            CR => 1,
            IMSC => self.imsc,
            RIS => 0,
            0xfe0..=0xffc => ID[((off - 0xfe0) / 4) as usize],
            _ => 0,
        })
    }

    fn write(&mut self, off: u64, _: u8, value: u64) {
        match off {
            LR => self.offset = i64::from(value as u32) - now(),
            MR => self.mr = value as u32,
            IMSC => self.imsc = value as u32 & 1,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Pl031;
    use crate::Device;

    #[test]
    fn the_clock_has_the_time_that_the_guest_set() {
        let mut c = Pl031::new();
        let t = c.read(0, 4);
        assert!(t > 1_700_000_000, "the host time is {t}");
        c.write(8, 4, 1000);
        assert!((1000..1003).contains(&c.read(0, 4)));
        assert_eq!(c.read(0xfe0, 4), 0x31);
    }
}
