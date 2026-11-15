//! The PCI configuration ports of the amd64 machine (plan 10.4), with no device on the bus. The
//! machine has no PCI device, but `pcicfginit` of `pc/pcipc.c` reads and writes the ports
//! 0xcf8 to 0xcff at each boot. With mechanism 1 there, the kernel scans the bus, reads all ones
//! as the vendor ID of each slot, and finds no device. With no device at the ports, it would try
//! mechanism 2 and then the PCI BIOS.
//!
//! The address register at 0xcf8 keeps a write of 4 bytes and gives it back. Bits 30 to 24 are
//! always 0, as the specification says. A write of fewer bytes at 0xcf8 to 0xcfb does not change
//! the register, as on a real host bridge, where it passes through as a normal I/O cycle. The data
//! register at 0xcfc reads all ones and drops each write.

use crate::Device;

/// The first port of the device. The bus has the ports at `crate::PORTS` plus the port.
pub const PORT: u64 = 0xcf8;
/// The number of ports.
pub const LEN: u64 = 8;

/// The bits of the address register that are always 0.
const RESERVED: u32 = 0x7f00_0000;

/// The PCI configuration ports with no device on the bus.
#[derive(Default)]
pub struct PciNone {
    address: u32,
}

impl PciNone {
    /// The ports, with 0 in the address register.
    #[must_use]
    pub fn new() -> PciNone {
        PciNone::default()
    }
}

fn ones(size: u8) -> u64 {
    if size >= 8 {
        u64::MAX
    } else {
        (1u64 << (8 * u32::from(size))) - 1
    }
}

impl Device for PciNone {
    fn name(&self) -> &'static str {
        "pci"
    }

    fn read(&mut self, off: u64, size: u8) -> u64 {
        if off == 0 && size == 4 {
            u64::from(self.address)
        } else {
            ones(size)
        }
    }

    fn write(&mut self, off: u64, size: u8, value: u64) {
        if off == 0 && size == 4 {
            self.address = value as u32 & !RESERVED;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PciNone;
    use crate::Device;

    /// The steps of `pcicfginit` that find mechanism 1.
    #[test]
    fn pcicfginit_finds_mechanism_1() {
        let mut p = PciNone::new();
        let n = p.read(0, 4);
        assert_eq!(n & 0x7f00_0000, 0);
        p.write(0, 4, 0x8000_0000);
        p.write(3, 1, 0);
        assert_ne!(p.read(0, 4) & 0x8000_0000, 0);
        p.write(0, 4, n);
        assert_eq!(p.read(0, 4), n);
    }

    #[test]
    fn each_slot_has_no_device() {
        let mut p = PciNone::new();
        p.write(0, 4, 0x8000_0000);
        assert_eq!(p.read(4, 4), 0xffff_ffff);
        assert_eq!(p.read(4, 2), 0xffff);
        assert_eq!(p.read(6, 1), 0xff);
        p.write(4, 4, 0);
        assert_eq!(p.read(4, 4), 0xffff_ffff);
    }

    #[test]
    fn the_reserved_bits_stay_0() {
        let mut p = PciNone::new();
        p.write(0, 4, 0xffff_ffff);
        assert_eq!(p.read(0, 4), 0x80ff_ffff);
    }
}
