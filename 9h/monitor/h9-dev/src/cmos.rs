//! The CMOS clock of the amd64 machine (plan 10.4), at the ports 0x70 (index) and 0x71 (data).
//! `devrtc.c` and `rtctime` of the tree read the time from it in BCD. The clock gives the UTC time
//! of the host, and the other bytes are a RAM of 128 bytes that the guest can write.

use crate::Device;

/// The first port of the device. The bus has the ports at `crate::PORTS` plus the port.
pub const PORT: u64 = 0x70;

const STATUS_A: usize = 0x0a;
const STATUS_B: usize = 0x0b;
const STATUS_C: usize = 0x0c;
const STATUS_D: usize = 0x0d;
const CENTURY: usize = 0x32;

/// The CMOS device.
pub struct Cmos {
    index: usize,
    nvram: [u8; 128],
    /// A fixed time in seconds from 1970, for the tests. None gives the time of the host.
    fixed: Option<u64>,
}

impl Default for Cmos {
    fn default() -> Self {
        Cmos::new()
    }
}

fn bcd(v: u64) -> u8 {
    (((v / 10 % 10) << 4) | (v % 10)) as u8
}

/// The year, month (1 to 12) and day (1 to 31) of the day `days` from 1970-01-01.
fn civil(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1).cast_unsigned();
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }).cast_unsigned();
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

impl Cmos {
    /// A clock with the time of the host.
    #[must_use]
    pub fn new() -> Cmos {
        Cmos {
            index: 0,
            nvram: [0; 128],
            fixed: None,
        }
    }

    /// A clock that stays at `secs` seconds from 1970.
    #[must_use]
    pub fn at(secs: u64) -> Cmos {
        Cmos {
            fixed: Some(secs),
            ..Cmos::new()
        }
    }

    fn now(&self) -> u64 {
        self.fixed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        })
    }

    fn register(&self, r: usize) -> u8 {
        let t = self.now();
        let days = (t / 86_400).cast_signed();
        let (year, month, day) = civil(days);
        let year = year.unsigned_abs();
        match r {
            0x00 => bcd(t % 60),
            0x02 => bcd(t / 60 % 60),
            0x04 => bcd(t / 3600 % 24),
            // 1 is Sunday. 1970-01-01 was a Thursday.
            0x06 => bcd((days + 4).rem_euclid(7) as u64 + 1),
            0x07 => bcd(day),
            0x08 => bcd(month),
            0x09 => bcd(year % 100),
            CENTURY => bcd(year / 100),
            // No update in progress, and the usual divider and rate.
            STATUS_A => 0x26,
            // 24-hour time in BCD, and no interrupts.
            STATUS_B => 0x02,
            STATUS_C => 0,
            // The battery is good.
            STATUS_D => 0x80,
            _ => self.nvram[r],
        }
    }
}

impl Device for Cmos {
    fn name(&self) -> &'static str {
        "cmos"
    }

    fn read(&mut self, off: u64, _: u8) -> u64 {
        match off {
            1 => u64::from(self.register(self.index)),
            _ => 0xff,
        }
    }

    fn write(&mut self, off: u64, _: u8, value: u64) {
        match off {
            // Bit 7 of the index masks the NMI, which the machine does not have.
            0 => self.index = (value & 0x7f) as usize,
            1 if self.index > STATUS_D && self.index != CENTURY => {
                self.nvram[self.index] = value as u8;
            }
            // A write to the clock or to a status register is dropped: the clock is the host's.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cmos, civil};
    use crate::Device;

    fn reg(c: &mut Cmos, r: u64) -> u64 {
        c.write(0, 1, r);
        c.read(1, 1)
    }

    #[test]
    fn the_clock_reads_in_bcd() {
        // 2026-11-15 13:45:56 UTC, a Sunday.
        let mut c = Cmos::at(1_794_750_356);
        let got: Vec<u64> = [0x00, 0x02, 0x04, 0x06, 0x07, 0x08, 0x09, 0x32]
            .iter()
            .map(|&r| reg(&mut c, r))
            .collect();
        assert_eq!(got, [0x56, 0x45, 0x13, 0x01, 0x15, 0x11, 0x26, 0x20]);
        assert_eq!(reg(&mut c, 0x0a) & 0x80, 0, "no update in progress");
        assert_eq!(reg(&mut c, 0x0b) & 0x04, 0, "BCD, not binary");
        assert_eq!(reg(&mut c, 0x0d), 0x80);
        assert_eq!(
            reg(&mut c, 0x8d),
            0x80,
            "bit 7 of the index is the NMI mask"
        );
        c.write(0, 1, 0x40);
        c.write(1, 1, 0x5a);
        assert_eq!(reg(&mut c, 0x40), 0x5a);
        c.write(0, 1, 0x00);
        c.write(1, 1, 0x99);
        assert_eq!(reg(&mut c, 0x00), 0x56);
        assert_eq!(c.read(0, 1), 0xff);
    }

    #[test]
    fn the_days_are_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(20_772), (2026, 11, 15));
        assert_eq!(civil(-1), (1969, 12, 31));
    }
}
