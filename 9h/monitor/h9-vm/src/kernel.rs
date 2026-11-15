//! The kernel image. The monitor loads a Plan 9 a.out as `7l` or `6l` writes it, or the U-Boot
//! image that `aux/aout2uimage` makes from it, as the `.u` kernels of the tree.

/// The magic number of an arm64 a.out, `R_MAGIC` of `a.out.h`.
pub const R_MAGIC: u32 = 0x8000 | (4 * 28 * 28 + 7);
/// The magic number of an amd64 a.out, `S_MAGIC` of `a.out.h`.
pub const S_MAGIC: u32 = 0x8000 | (4 * 26 * 26 + 7);
/// The magic number of a U-Boot legacy image.
pub const UIMAGE_MAGIC: u32 = 0x2705_1956;

/// A kernel, ready to copy to the RAM of the guest.
#[derive(Debug, PartialEq, Eq)]
pub struct Image {
    /// The bytes to copy, each at its guest physical address.
    pub segments: Vec<(u64, Vec<u8>)>,
    /// The guest physical address of the first instruction.
    pub entry: u64,
    /// The guest physical address after the last byte of the kernel, its bss included.
    pub end: u64,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn be64(b: &[u8], at: usize) -> u64 {
    u64::from(be32(b, at)) << 32 | u64::from(be32(b, at + 4))
}

/// Read a kernel image. `kzero` is the virtual address of physical address 0 in the kernel,
/// `KZERO` of its `mem.h`, and `pgsize` is the page size of the a.out segments, as in the `Mach`
/// of libmach (64 KiB for arm64).
pub fn load(bytes: &[u8], kzero: u64, pgsize: u64) -> Result<Image, String> {
    if bytes.len() < 40 {
        return Err("the kernel is too short for a header".into());
    }
    match be32(bytes, 0) {
        UIMAGE_MAGIC => {
            let size = be32(bytes, 12) as usize;
            let load = u64::from(be32(bytes, 16));
            let entry = u64::from(be32(bytes, 20));
            let body = bytes
                .get(64..64 + size)
                .ok_or("the U-Boot image is shorter than its header says")?;
            Ok(Image {
                segments: vec![(load, body.to_vec())],
                entry,
                end: load + size as u64,
            })
        }
        R_MAGIC => {
            let text = be32(bytes, 4) as usize;
            let data = be32(bytes, 8) as usize;
            let bss = u64::from(be32(bytes, 12));
            let entry = be64(bytes, 32);
            let hdr = 40;
            let t = bytes
                .get(hdr..hdr + text)
                .ok_or("the a.out is shorter than its text")?;
            let d = bytes
                .get(hdr + text..hdr + text + data)
                .ok_or("the a.out is shorter than its data")?;
            // The text starts at the entry point, and the data on the next segment boundary.
            let tva = entry;
            let dva = (tva + t.len() as u64).next_multiple_of(pgsize);
            let pa = |va: u64| va.wrapping_sub(kzero);
            let end = pa(dva) + data as u64 + bss;
            Ok(Image {
                segments: vec![(pa(tva), t.to_vec()), (pa(dva), d.to_vec())],
                entry: pa(entry),
                end,
            })
        }
        S_MAGIC => Err(
            "the kernel is an amd64 a.out, and the amd64 machine is not in the monitor yet".into(),
        ),
        m => Err(format!(
            "the kernel has the magic number {m:#x}, which is not an arm64 a.out or a U-Boot image"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{Image, R_MAGIC, UIMAGE_MAGIC, load};

    const KZERO: u64 = 0xffff_ffff_8000_0000;

    #[test]
    fn the_magic_numbers_are_those_of_the_tree() {
        assert_eq!(R_MAGIC, 0x8c47);
        assert_eq!(super::S_MAGIC, 0x8a97);
    }

    #[test]
    fn a_uimage_loads_at_its_load_address() {
        let mut b = vec![0u8; 64];
        b[0..4].copy_from_slice(&UIMAGE_MAGIC.to_be_bytes());
        b[12..16].copy_from_slice(&4u32.to_be_bytes());
        b[16..20].copy_from_slice(&0x4010_0000u32.to_be_bytes());
        b[20..24].copy_from_slice(&0x4010_0000u32.to_be_bytes());
        b.extend_from_slice(&[1, 2, 3, 4]);
        let i = load(&b, KZERO, 0x10000).unwrap();
        assert_eq!(
            i,
            Image {
                segments: vec![(0x4010_0000, vec![1, 2, 3, 4])],
                entry: 0x4010_0000,
                end: 0x4010_0004
            }
        );
    }

    #[test]
    fn an_arm64_aout_puts_the_data_on_the_next_segment() {
        let mut b = vec![0u8; 40];
        b[0..4].copy_from_slice(&R_MAGIC.to_be_bytes());
        b[4..8].copy_from_slice(&8u32.to_be_bytes());
        b[8..12].copy_from_slice(&4u32.to_be_bytes());
        b[12..16].copy_from_slice(&0x100u32.to_be_bytes());
        b[32..40].copy_from_slice(&0xffff_ffff_c010_0000u64.to_be_bytes());
        b.extend_from_slice(&[9; 8]);
        b.extend_from_slice(&[7; 4]);
        let i = load(&b, KZERO, 0x10000).unwrap();
        assert_eq!(i.entry, 0x4010_0000);
        assert_eq!(
            i.segments,
            vec![(0x4010_0000, vec![9; 8]), (0x4011_0000, vec![7; 4])]
        );
        assert_eq!(i.end, 0x4011_0104);
    }

    #[test]
    fn a_short_or_unknown_kernel_is_refused() {
        assert!(load(&[0; 8], KZERO, 0x10000).is_err());
        assert!(
            load(&[0x7f; 64], KZERO, 0x10000)
                .unwrap_err()
                .contains("magic")
        );
    }
}
