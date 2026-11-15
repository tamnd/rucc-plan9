//! The `PM1a` control register of ACPI on the amd64 machine (plan 10.4). The FADT gives its port,
//! and the DSDT gives the package `_S5` with the sleep type `SLP_TYP`. `fshalt` runs `aux/acpi`,
//! and its `poweroff` writes `SLP_TYP` and `SLP_EN` to the register. Then the device asks to end
//! the session, and the machine turns off.
//!
//! The register keeps each other write and gives it back. `SLP_EN` always reads as 0, as the
//! specification says. The machine has no `PM1b` control register.

use crate::Device;
use h9_vm::acpi;

/// The port of the register, from the FADT. The bus has the ports at `crate::PORTS` plus the
/// port.
pub const PORT: u64 = acpi::PM1A_CNT as u64;
/// The number of ports.
pub const LEN: u64 = 2;
/// The sleep type of the state S5, which the DSDT gives in `_S5`.
pub const S5: u16 = acpi::SLP_TYP_S5 as u16;

const SLP_EN: u16 = 1 << 13;
const SLP_TYP_SHIFT: u16 = 10;
const SLP_TYP: u16 = 7 << SLP_TYP_SHIFT;

/// The `PM1a` control register.
#[derive(Default)]
pub struct AcpiPm {
    cnt: u16,
    off: bool,
}

impl AcpiPm {
    /// The register, with 0 in it.
    #[must_use]
    pub fn new() -> AcpiPm {
        AcpiPm::default()
    }
}

impl Device for AcpiPm {
    fn name(&self) -> &'static str {
        "acpi pm"
    }

    fn read(&mut self, off: u64, size: u8) -> u64 {
        let v = u64::from(self.cnt) >> (8 * off);
        if size >= 2 { v & 0xffff } else { v & 0xff }
    }

    fn write(&mut self, off: u64, size: u8, value: u64) {
        let v = match (off, size) {
            (0, 1) => (self.cnt & 0xff00) | (value as u16 & 0xff),
            (1, 1) => (self.cnt & 0x00ff) | ((value as u16 & 0xff) << 8),
            (0, _) => value as u16,
            _ => return,
        };
        if v & SLP_EN != 0 && (v & SLP_TYP) >> SLP_TYP_SHIFT == S5 {
            self.off = true;
        }
        self.cnt = v & !SLP_EN;
    }

    fn stop(&self) -> bool {
        self.off
    }
}

#[cfg(test)]
mod tests {
    use super::{AcpiPm, S5};
    use crate::Device;

    /// The write of `poweroff` in `aux/acpi.c`.
    #[test]
    fn slp_en_with_the_type_of_s5_turns_the_machine_off() {
        let mut p = AcpiPm::new();
        assert!(!p.stop());
        p.write(0, 2, u64::from((S5 << 10) & 0x1c00 | 0x2000));
        assert!(p.stop());
    }

    #[test]
    fn other_writes_do_not_turn_it_off() {
        let mut p = AcpiPm::new();
        p.write(0, 2, 1);
        assert_eq!(p.read(0, 2), 1);
        p.write(0, 2, u64::from(S5 << 10));
        assert!(!p.stop());
        p.write(0, 2, 0x2000);
        assert!(!p.stop());
        assert_eq!(p.read(0, 2), 0);
    }

    #[test]
    fn the_bytes_are_written_one_at_a_time() {
        let mut p = AcpiPm::new();
        p.write(0, 1, 0x01);
        assert!(!p.stop());
        p.write(1, 1, 0x34);
        assert!(p.stop());
        assert_eq!(p.read(0, 2), 0x1401);
        assert_eq!(p.read(1, 1), 0x14);
    }
}
