//! The disks of the 9h kernel (`sdh9.c`, plan 10.5). Each unit is a host file. A read or a write
//! is one hypercall, which does the transfer before it returns. A transfer is at most 64 KiB, so
//! the call is short, and the kernel needs no interrupt for the disk.

use std::fs::File;
use std::os::unix::fs::FileExt;

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The most units of the device. `sdh9.c` has the same number.
pub const MAX_UNITS: usize = 4;

struct Unit {
    file: File,
    sectors: u64,
    readonly: bool,
}

/// The disk device.
#[derive(Default)]
pub struct H9Disk {
    units: Vec<Unit>,
}

impl H9Disk {
    /// A device with no units.
    #[must_use]
    pub fn new() -> H9Disk {
        H9Disk::default()
    }

    /// Add the host file `file` as the next unit. Its length must be a whole number of sectors.
    pub fn add(&mut self, file: File, readonly: bool) -> Result<usize, String> {
        if self.units.len() == MAX_UNITS {
            return Err(format!("the 9h machine has at most {MAX_UNITS} disks"));
        }
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len % h9::SD_SECTOR != 0 {
            return Err(format!(
                "the disk has {len} bytes, which is not a whole number of {}-byte sectors",
                h9::SD_SECTOR
            ));
        }
        self.units.push(Unit {
            file,
            sectors: len / h9::SD_SECTOR,
            readonly,
        });
        Ok(self.units.len() - 1)
    }

    fn transfer(&mut self, x: &[u64; 6], ram: &Ram, write: bool) -> u64 {
        let Some(u) = usize::try_from(x[1]).ok().and_then(|i| self.units.get(i)) else {
            return h9::NOT_SUPPORTED;
        };
        let (lba, pa) = (x[2], x[4]);
        let n = x[3].min(h9::SD_MAX);
        if lba > u.sectors || n > u.sectors - lba || (write && u.readonly) {
            return h9::IO_ERROR;
        }
        let off = lba * h9::SD_SECTOR;
        let mut b = vec![0; (n * h9::SD_SECTOR) as usize];
        if write {
            if !ram.read(pa, &mut b) {
                return h9::BAD_ADDRESS;
            }
            if u.file.write_all_at(&b, off).is_err() {
                return h9::IO_ERROR;
            }
        } else {
            if ram.offset(pa, b.len()).is_none() {
                return h9::BAD_ADDRESS;
            }
            if u.file.read_exact_at(&mut b, off).is_err() {
                return h9::IO_ERROR;
            }
            if !ram.write(pa, &b) {
                return h9::BAD_ADDRESS;
            }
        }
        n
    }
}

impl Device for H9Disk {
    fn name(&self) -> &'static str {
        "h9disk"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        let unit = usize::try_from(x[1]).ok().and_then(|i| self.units.get(i));
        Some(match x[0] as u32 {
            h9::SD_INFO => unit.map_or(h9::NOT_SUPPORTED, |u| u.sectors),
            h9::SD_READ => self.transfer(x, ram, false),
            h9::SD_WRITE => self.transfer(x, ram, true),
            h9::SD_FLUSH => match unit {
                None => h9::NOT_SUPPORTED,
                Some(u) if u.readonly || u.file.sync_data().is_ok() => 0,
                Some(_) => h9::IO_ERROR,
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::H9Disk;
    use crate::{Bus, h9};
    use h9_vm::Ram;
    use std::fs::OpenOptions;

    fn disk(name: &str, len: usize) -> std::fs::File {
        let p = std::env::temp_dir().join(format!("h9disk-{}-{name}", std::process::id()));
        std::fs::write(&p, (0..len).map(|i| (i / 512) as u8).collect::<Vec<u8>>()).unwrap();
        let f = OpenOptions::new().read(true).write(true).open(&p).unwrap();
        std::fs::remove_file(&p).unwrap();
        f
    }

    #[test]
    fn the_disk_reads_and_writes_sectors() {
        let ram = Ram::new(0x4000_0000, 0x4_0000).unwrap();
        let mut d = H9Disk::new();
        assert_eq!(d.add(disk("a", 200 * 512), false), Ok(0));
        assert_eq!(d.add(disk("b", 4 * 512), true), Ok(1));
        assert!(d.add(disk("c", 513), false).is_err());
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(d));
        let mut call =
            |f: u32, a: [u64; 4]| bus.call(&[u64::from(f), a[0], a[1], a[2], a[3], 0], &ram);
        assert_eq!(call(h9::SD_INFO, [0; 4]), 200);
        assert_eq!(call(h9::SD_INFO, [1, 0, 0, 0]), 4);
        assert_eq!(call(h9::SD_INFO, [2, 0, 0, 0]), h9::NOT_SUPPORTED);

        assert_eq!(call(h9::SD_READ, [0, 3, 2, 0x4000_0000]), 2);
        let mut b = [0u8; 1024];
        assert!(ram.read(0x4000_0000, &mut b));
        assert!(b[..512].iter().all(|&c| c == 3) && b[512..].iter().all(|&c| c == 4));

        assert_eq!(call(h9::SD_READ, [0, 0, 1000, 0x4000_0000]), h9::SD_MAX);
        assert_eq!(call(h9::SD_READ, [0, 199, 2, 0x4000_0000]), h9::IO_ERROR);
        assert_eq!(call(h9::SD_READ, [0, 0, 1, 0x5000_0000]), h9::BAD_ADDRESS);

        assert!(ram.write(0x4000_2000, &[0xee; 512]));
        assert_eq!(call(h9::SD_WRITE, [0, 7, 1, 0x4000_2000]), 1);
        assert_eq!(call(h9::SD_FLUSH, [0; 4]), 0);
        assert_eq!(call(h9::SD_READ, [0, 7, 1, 0x4000_3000]), 1);
        let mut c = [0u8; 512];
        assert!(ram.read(0x4000_3000, &mut c));
        assert!(c.iter().all(|&x| x == 0xee));

        assert_eq!(call(h9::SD_WRITE, [1, 0, 1, 0x4000_2000]), h9::IO_ERROR);
        assert_eq!(call(h9::SD_FLUSH, [1, 0, 0, 0]), 0);
    }
}
