//! The devices of the 9h monitor, and the bus that sends each access of the guest to one of them
//! (plan 10.5 and 10.8).
//!
//! The 9h kernel uses hypercalls for its devices. Until that kernel exists, the monitor runs the
//! `qemu` kernel of the tree as its first test, and that kernel needs a PL011 for its console and
//! a PL031 for its clock. These two are here as bring-up devices. An access to an address that
//! no device has reads as all ones and is logged, as an empty slot on a real bus (plan 10.4).
//! With `strict`, such an access stops the session instead (plan 10.10).

pub mod h9;
pub mod h9cons;
pub mod h9disk;
pub mod pl011;
pub mod pl031;
pub mod psci;

use std::collections::BTreeMap;

use h9_vm::Ram;

/// A device that the guest reads and writes as memory.
pub trait Device: Send + std::any::Any {
    /// The name, for the log.
    fn name(&self) -> &'static str;
    /// A load of `size` bytes at the offset `off` in the device.
    fn read(&mut self, off: u64, size: u8) -> u64;
    /// A store of `size` bytes at the offset `off` in the device.
    fn write(&mut self, off: u64, size: u8, value: u64);
    /// The shared peripheral interrupt of the device and its level now, if it has one.
    fn irq(&self) -> Option<(u32, bool)> {
        None
    }
    /// A hypercall with the registers X0 to X5. The result goes in X0. None when the number is
    /// not one of this device.
    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        let _ = (x, ram);
        None
    }
}

struct Slot {
    base: u64,
    len: u64,
    dev: Box<dyn Device>,
}

/// The devices of a machine, by guest physical address.
#[derive(Default)]
pub struct Bus {
    slots: Vec<Slot>,
    /// The devices that have hypercalls and no addresses.
    hyper: Vec<Box<dyn Device>>,
    /// Stop at an access that no device has, instead of reading all ones.
    pub strict: bool,
    unknown: BTreeMap<u64, u64>,
    unknown_calls: Vec<u64>,
}

impl Bus {
    /// A bus with no devices.
    #[must_use]
    pub fn new() -> Bus {
        Bus::default()
    }

    /// Put a device at the guest physical addresses `base` to `base + len`.
    pub fn add(&mut self, base: u64, len: u64, dev: Box<dyn Device>) {
        assert!(
            self.slots
                .iter()
                .all(|s| base + len <= s.base || s.base + s.len <= base),
            "h9-dev: {} overlaps another device",
            dev.name()
        );
        self.slots.push(Slot { base, len, dev });
    }

    /// Add a device that the guest uses only through hypercalls.
    pub fn add_hyper(&mut self, dev: Box<dyn Device>) {
        self.hyper.push(dev);
    }

    /// The first device of type `T` that has only hypercalls.
    pub fn hyper_device<T: Device + 'static>(&mut self) -> Option<&mut T> {
        self.hyper.iter_mut().find_map(|d| {
            let d: &mut dyn std::any::Any = d.as_mut();
            d.downcast_mut::<T>()
        })
    }

    /// A hypercall that is not PSCI. The answer is in X0: the result of the device whose call it
    /// is, or `NOT_SUPPORTED` when no device has the number.
    pub fn call(&mut self, x: &[u64; 6], ram: &Ram) -> u64 {
        if x[0] as u32 == h9::VERSION {
            return h9::REVISION;
        }
        let devs = self
            .hyper
            .iter_mut()
            .chain(self.slots.iter_mut().map(|s| &mut s.dev));
        for d in devs {
            if let Some(v) = d.call(x, ram) {
                return v;
            }
        }
        if !self.unknown_calls.contains(&x[0]) {
            self.unknown_calls.push(x[0]);
            eprintln!(
                "9h: the guest made the hypercall {:#x}, which no device has",
                x[0]
            );
        }
        h9::NOT_SUPPORTED
    }

    fn find(&mut self, pa: u64) -> Option<&mut Slot> {
        self.slots
            .iter_mut()
            .find(|s| pa >= s.base && pa - s.base < s.len)
    }

    /// The device at `pa`, as its concrete type.
    pub fn device<T: Device + 'static>(&mut self, pa: u64) -> Option<&mut T> {
        let d: &mut dyn std::any::Any = self.find(pa)?.dev.as_mut();
        d.downcast_mut::<T>()
    }

    fn unknown(&mut self, pa: u64, what: &str) -> Result<(), String> {
        let n = self.unknown.entry(pa).or_insert(0);
        *n += 1;
        if self.strict {
            return Err(format!("{what} at {pa:#x}, where there is no device"));
        }
        // A probe, as the PCI scan, can touch many addresses of one empty block. Log the first
        // access to each 16 MiB block only.
        if *n == 1 && !self.unknown.range(pa & !0xff_ffff..pa).any(|_| true) {
            eprintln!(
                "9h: {what} at {pa:#x}, where there is no device (the log shows the first such access in each 16 MiB block)"
            );
        }
        Ok(())
    }

    /// A load from the guest. An address with no device reads as all ones.
    pub fn read(&mut self, pa: u64, size: u8) -> Result<u64, String> {
        if let Some(s) = self.find(pa) {
            return Ok(s.dev.read(pa - s.base, size));
        }
        self.unknown(pa, "a read")?;
        Ok(if size >= 8 {
            u64::MAX
        } else {
            (1u64 << (8 * u32::from(size))) - 1
        })
    }

    /// A store from the guest. A store to an address with no device is dropped.
    pub fn write(&mut self, pa: u64, size: u8, value: u64) -> Result<(), String> {
        if let Some(s) = self.find(pa) {
            s.dev.write(pa - s.base, size, value);
            return Ok(());
        }
        self.unknown(pa, "a write")
    }

    /// The interrupt lines of the devices and their levels.
    #[must_use]
    pub fn irqs(&self) -> Vec<(u32, bool)> {
        self.slots
            .iter()
            .map(|s| &s.dev)
            .chain(&self.hyper)
            .filter_map(|d| d.irq())
            .collect()
    }

    /// The addresses with no device that the guest used, and how many times.
    #[must_use]
    pub fn unknown_accesses(&self) -> &BTreeMap<u64, u64> {
        &self.unknown
    }
}

#[cfg(test)]
mod tests {
    use super::{Bus, Device};

    struct Reg(u64);
    impl Device for Reg {
        fn name(&self) -> &'static str {
            "reg"
        }
        fn read(&mut self, off: u64, _: u8) -> u64 {
            self.0 + off
        }
        fn write(&mut self, _: u64, _: u8, v: u64) {
            self.0 = v;
        }
    }

    #[test]
    fn an_access_goes_to_its_device_or_reads_all_ones() {
        let mut b = Bus::new();
        b.add(0x1000, 0x100, Box::new(Reg(5)));
        assert_eq!(b.read(0x1004, 4), Ok(9));
        b.write(0x1000, 4, 100).unwrap();
        assert_eq!(b.read(0x1000, 4), Ok(100));
        assert_eq!(b.read(0x2000, 4), Ok(0xffff_ffff));
        assert_eq!(b.read(0x2000, 1), Ok(0xff));
        assert_eq!(b.read(0x2000, 8), Ok(u64::MAX));
        assert_eq!(b.unknown_accesses().get(&0x2000), Some(&3));
        assert_eq!(b.device::<Reg>(0x10ff).map(|r| r.0), Some(100));
    }

    #[test]
    fn a_strict_bus_stops_at_an_unknown_address() {
        let mut b = Bus::new();
        b.strict = true;
        assert!(b.read(0x2000, 4).unwrap_err().contains("0x2000"));
        assert!(b.write(0x2000, 4, 0).is_err());
    }

    #[test]
    #[should_panic(expected = "overlaps")]
    fn two_devices_cannot_overlap() {
        let mut b = Bus::new();
        b.add(0x1000, 0x100, Box::new(Reg(0)));
        b.add(0x10f0, 0x100, Box::new(Reg(0)));
    }
}
