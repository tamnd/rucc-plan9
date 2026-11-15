//! The ACPI tables of the amd64 machine (plan 10.4). `archacpi.c` of the tree finds the RSDP in
//! the BIOS area, and reads the processors and the I/O APIC from the MADT. The tables are the
//! least that it accepts: an RSDP, an RSDT and an XSDT, a FADT, a DSDT with no AML, and the MADT.
//! The machine has no HPET table, so the kernel uses the PIT and the TSC.
//!
//! The MADT has no interrupt source override, so ISA interrupt N is input N of the I/O APIC, as
//! in the default routing of KVM.

/// The guest physical address of the RSDP, at the start of the BIOS area that the kernel scans.
pub const RSDP: u64 = 0xe_0000;
/// The guest physical address of the local APICs.
pub const LAPIC: u64 = 0xfee0_0000;
/// The guest physical address of the I/O APIC.
pub const IOAPIC: u64 = 0xfec0_0000;
/// The most processors. The MADT has an entry of 8 bytes for each.
pub const MAX_CPUS: u32 = 64;

/// The offsets of the tables from `RSDP`.
const RSDT: usize = 0x40;
const XSDT: usize = 0x80;
const FADT: usize = 0x100;
const DSDT: usize = 0x180;
const MADT: usize = 0x200;

const HEADER: usize = 36;
const OEM: &[u8; 6] = b"R9 9H ";

fn put(b: &mut [u8], at: usize, v: &[u8]) {
    b[at..at + v.len()].copy_from_slice(v);
}

fn sum(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |s, &x| s.wrapping_add(x))
}

/// Write the header of a table of `len` bytes at `at`. The checksum is set by `seal`.
fn header(b: &mut [u8], at: usize, sig: [u8; 4], len: usize, rev: u8) {
    put(b, at, &sig);
    put(b, at + 4, &(len as u32).to_le_bytes());
    b[at + 8] = rev;
    put(b, at + 10, OEM);
    put(b, at + 16, b"9H      ");
    put(b, at + 24, &1u32.to_le_bytes());
    put(b, at + 28, b"R9  ");
    put(b, at + 32, &1u32.to_le_bytes());
}

/// Set the checksum at `at + check` so that the `len` bytes at `at` sum to 0.
fn seal(b: &mut [u8], at: usize, len: usize, check: usize) {
    b[at + check] = 0;
    b[at + check] = 0u8.wrapping_sub(sum(&b[at..at + len]));
}

/// The tables for `ncpu` processors, to write at `RSDP`.
pub fn tables(ncpu: u32) -> Result<Vec<u8>, String> {
    if ncpu == 0 || ncpu > MAX_CPUS {
        return Err(format!("the machine can have 1 to {MAX_CPUS} processors"));
    }
    let base = RSDP as u32;
    let madt_len = HEADER + 8 + 8 * ncpu as usize + 12;
    let mut b = vec![0u8; MADT + madt_len];

    // RSDP, revision 2, with the RSDT and the XSDT.
    put(&mut b, 0, b"RSD PTR ");
    put(&mut b, 9, OEM);
    b[15] = 2;
    put(&mut b, 16, &(base + RSDT as u32).to_le_bytes());
    put(&mut b, 20, &36u32.to_le_bytes());
    put(&mut b, 24, &u64::from(base + XSDT as u32).to_le_bytes());
    seal(&mut b, 0, 20, 8);
    seal(&mut b, 0, 36, 32);

    let entries = [FADT, MADT].map(|t| base + t as u32);
    header(&mut b, RSDT, *b"RSDT", HEADER + 4 * entries.len(), 1);
    header(&mut b, XSDT, *b"XSDT", HEADER + 8 * entries.len(), 1);
    for (n, e) in entries.iter().enumerate() {
        put(&mut b, RSDT + HEADER + 4 * n, &e.to_le_bytes());
        put(&mut b, XSDT + HEADER + 8 * n, &u64::from(*e).to_le_bytes());
    }
    seal(&mut b, RSDT, HEADER + 4 * entries.len(), 9);
    seal(&mut b, XSDT, HEADER + 8 * entries.len(), 9);

    // FADT of ACPI 1.0, 116 bytes: only the DSDT. The kernel reads it at offset 40, and the
    // X_DSDT at offset 140 only in a longer table.
    header(&mut b, FADT, *b"FACP", 116, 1);
    put(&mut b, FADT + 40, &(base + DSDT as u32).to_le_bytes());
    seal(&mut b, FADT, 116, 9);

    // DSDT with no AML.
    header(&mut b, DSDT, *b"DSDT", HEADER, 2);
    seal(&mut b, DSDT, HEADER, 9);

    // MADT: the local APIC address, PCAT_COMPAT for the 8259s, a local APIC for each processor
    // with its number as the APIC ID, and the I/O APIC with ID 0 and GSI base 0.
    header(&mut b, MADT, *b"APIC", madt_len, 3);
    put(&mut b, MADT + HEADER, &(LAPIC as u32).to_le_bytes());
    put(&mut b, MADT + HEADER + 4, &1u32.to_le_bytes());
    let mut p = MADT + HEADER + 8;
    for n in 0..ncpu {
        put(&mut b, p, &[0, 8, n as u8, n as u8]);
        put(&mut b, p + 4, &1u32.to_le_bytes());
        p += 8;
    }
    put(&mut b, p, &[1, 12, 0, 0]);
    put(&mut b, p + 4, &(IOAPIC as u32).to_le_bytes());
    put(&mut b, p + 8, &0u32.to_le_bytes());
    seal(&mut b, MADT, madt_len, 9);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le32(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// The table at the guest physical address `pa`, as `maptable` of the tree checks it.
    fn table(b: &[u8], pa: u64) -> &[u8] {
        let at = (pa - RSDP) as usize;
        let len = le32(b, at + 4) as usize;
        let t = &b[at..at + len];
        assert_eq!(
            sum(t),
            0,
            "the checksum of {}",
            String::from_utf8_lossy(&t[..4])
        );
        t
    }

    #[test]
    fn the_tables_are_what_archacpi_reads() {
        let b = tables(4).unwrap();
        assert_eq!(&b[..8], b"RSD PTR ");
        assert_eq!(sum(&b[..20]), 0);
        assert_eq!(sum(&b[..36]), 0);
        assert_eq!(b[15], 2);
        let xsdt = table(&b, u64::from_le_bytes(b[24..32].try_into().unwrap()));
        let rsdt = table(&b, u64::from(le32(&b, 16)));
        assert_eq!(&xsdt[..4], b"XSDT");
        assert_eq!(&rsdt[..4], b"RSDT");
        let mut sigs = Vec::new();
        for n in 0..2 {
            let pa = u64::from_le_bytes(xsdt[36 + 8 * n..44 + 8 * n].try_into().unwrap());
            assert_eq!(pa, u64::from(le32(rsdt, 36 + 4 * n)));
            let t = table(&b, pa);
            sigs.push(String::from_utf8_lossy(&t[..4]).to_string());
            if &t[..4] == b"FACP" {
                let dsdt = table(&b, u64::from(le32(t, 40)));
                assert_eq!(&dsdt[..4], b"DSDT");
                assert_eq!(dsdt.len(), 36);
            }
            if &t[..4] == b"APIC" {
                assert_eq!(u64::from(le32(t, 36)), LAPIC);
                let mut p = 44;
                let mut cpus = Vec::new();
                let mut ioapic = None;
                while p < t.len() {
                    match t[p] {
                        0 => cpus.push((t[p + 3], le32(t, p + 4) & 1)),
                        1 => ioapic = Some((t[p + 2], le32(t, p + 4), le32(t, p + 8))),
                        x => panic!("entry type {x}"),
                    }
                    p += t[p + 1] as usize;
                }
                assert_eq!(cpus, [(0, 1), (1, 1), (2, 1), (3, 1)]);
                assert_eq!(ioapic, Some((0, IOAPIC as u32, 0)));
            }
        }
        assert_eq!(sigs, ["FACP", "APIC"]);
        assert!(tables(0).is_err());
        assert!(tables(MAX_CPUS + 1).is_err());
        assert!(b.len() < 0x400);
    }
}
