//! The multiboot loader of the amd64 machine (plan 10.4). `9pc64` has a multiboot header with
//! the load addresses of the a.out (bit 16 of its flags), so the loader needs no ELF reader. The
//! monitor copies the kernel to its load address, writes the information block, and starts
//! vCPU 0 in 32-bit protected mode at the entry with `EAX = BOOT_MAGIC` and `EBX` at the block.
//!
//! The block and what it points to are at `INFO`, between the bootstrap page of the application
//! processors (0x7000) and the IDT (0x10000) of the pc64 kernel. `multibootargs` of the tree's
//! `pc/bootargs.c` reads the memory map, the command line and the first module, the plan9.ini,
//! before the kernel uses that memory.

use crate::kernel::Image;

/// The magic number of the header in the kernel.
pub const HEADER_MAGIC: u32 = 0x1BAD_B002;
/// The value of `EAX` at the entry.
pub const BOOT_MAGIC: u32 = 0x2BAD_B002;
/// The guest physical address of the information block.
pub const INFO: u64 = 0x8000;
/// The space for the block, the memory map, the module, the command line and the plan9.ini.
pub const INFO_SIZE: usize = 0x8000;
/// The flag of the header that gives the load addresses.
const LOAD_ADDRESSES: u32 = 1 << 16;
/// The offsets in the space at `INFO`.
const MMAP: usize = 0x100;
const MODULE: usize = 0x200;
const CMDLINE: usize = 0x300;
const CMDLINE_MAX: usize = 0xd00;
const INI: usize = 0x1000;
/// The largest plan9.ini. The kernel keeps less than this in its BOOTARGS.
pub const INI_MAX: usize = INFO_SIZE - INI - 1;

/// The start of the hole of the PC below 1 MiB: VGA memory and the BIOS area.
pub const LOW_END: u64 = 0xa_0000;
/// The start of the extended BIOS data area, which the memory map reserves.
const EBDA: u64 = 0x9_fc00;
/// The BIOS area, where the ACPI tables are.
const BIOS: u64 = 0xe_0000;
/// The end of the first MiB.
const HIGH: u64 = 0x10_0000;

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Read a multiboot kernel. The header must be in the first 8 KiB of the file, aligned to 4
/// bytes, and must give the load addresses.
pub fn load(bytes: &[u8]) -> Result<Image, String> {
    let limit = bytes.len().min(8192);
    let at = (0..limit.saturating_sub(31))
        .step_by(4)
        .find(|&i| {
            le32(bytes, i) == HEADER_MAGIC
                && HEADER_MAGIC
                    .wrapping_add(le32(bytes, i + 4))
                    .wrapping_add(le32(bytes, i + 8))
                    == 0
        })
        .ok_or("the kernel has no multiboot header in its first 8 KiB")?;
    let flags = le32(bytes, at + 4);
    if flags & LOAD_ADDRESSES == 0 {
        return Err(
            "the multiboot header gives no load addresses, and the loader reads no ELF".into(),
        );
    }
    let [header, load, load_end, bss_end, entry] =
        [12, 16, 20, 24, 28].map(|o| u64::from(le32(bytes, at + o)));
    let start = header
        .checked_sub(load)
        .and_then(|d| (at as u64).checked_sub(d))
        .ok_or("the multiboot header is before the start of the file")? as usize;
    let len = if load_end == 0 {
        bytes.len() - start
    } else {
        load_end
            .checked_sub(load)
            .ok_or("the load end of the multiboot header is before the load address")?
            as usize
    };
    let body = bytes
        .get(start..start + len)
        .ok_or("the kernel is shorter than its multiboot header says")?;
    let end = if bss_end == 0 {
        load + len as u64
    } else {
        bss_end
    };
    if end < load + len as u64 {
        return Err("the bss end of the multiboot header is before the load end".into());
    }
    if !(load..end).contains(&entry) {
        return Err(format!("the entry {entry:#x} is not in the kernel"));
    }
    if load < HIGH {
        return Err(format!(
            "the kernel loads at {load:#x}, below 1 MiB, where the loader and the BIOS area are"
        ));
    }
    Ok(Image {
        segments: vec![(load, body.to_vec())],
        entry,
        end,
    })
}

/// A memory type of the memory map: usable RAM.
pub const USABLE: u32 = 1;
/// A memory type of the memory map: reserved.
pub const RESERVED: u32 = 2;

/// The memory map of a machine with `mem` bytes of RAM from address 0: the RAM below the hole of
/// the PC, the EBDA and the BIOS area, which are reserved, and the RAM from 1 MiB.
#[must_use]
pub fn memory_map(mem: u64) -> Vec<(u64, u64, u32)> {
    vec![
        (0, EBDA, USABLE),
        (EBDA, LOW_END - EBDA, RESERVED),
        (BIOS, HIGH - BIOS, RESERVED),
        (HIGH, mem.saturating_sub(HIGH), USABLE),
    ]
}

/// What the kernel gets from the loader.
#[derive(Debug, Clone, Default)]
pub struct Info<'a> {
    /// The bytes of RAM from address 0.
    pub mem: u64,
    /// The command line. The kernel takes each word as a line of plan9.ini.
    pub cmdline: &'a str,
    /// The plan9.ini, as the first module.
    pub ini: Option<&'a [u8]>,
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// The bytes to write at `INFO`: the information block of the multiboot specification and what
/// it points to.
pub fn info(i: &Info) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; INFO_SIZE];
    let base = INFO as u32;
    // Flags: memory sizes (0), command line (2), modules (3), memory map (6).
    let mut flags = 1 << 0 | 1 << 2 | 1 << 6;
    put32(&mut b, 4, (EBDA / 1024) as u32);
    put32(
        &mut b,
        8,
        (i.mem.saturating_sub(HIGH) / 1024).min(u64::from(u32::MAX)) as u32,
    );

    if i.cmdline.len() >= CMDLINE_MAX {
        return Err(format!(
            "the command line is longer than {} bytes",
            CMDLINE_MAX - 1
        ));
    }
    b[CMDLINE..CMDLINE + i.cmdline.len()].copy_from_slice(i.cmdline.as_bytes());
    put32(&mut b, 16, base + CMDLINE as u32);

    if let Some(ini) = i.ini {
        if ini.len() > INI_MAX {
            return Err(format!("the plan9.ini is longer than {INI_MAX} bytes"));
        }
        b[INI..INI + ini.len()].copy_from_slice(ini);
        flags |= 1 << 3;
        put32(&mut b, 20, 1);
        put32(&mut b, 24, base + MODULE as u32);
        put32(&mut b, MODULE, base + INI as u32);
        put32(&mut b, MODULE + 4, base + (INI + ini.len()) as u32);
    }

    let map = memory_map(i.mem);
    for (n, &(start, len, kind)) in map.iter().enumerate() {
        let e = MMAP + n * 24;
        put32(&mut b, e, 20);
        put64(&mut b, e + 4, start);
        put64(&mut b, e + 12, len);
        put32(&mut b, e + 20, kind);
    }
    put32(&mut b, 44, (map.len() * 24) as u32);
    put32(&mut b, 48, base + MMAP as u32);
    put32(&mut b, 0, flags);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kernel as `9pc64` is: some text, then the header with the load addresses, at file
    /// offset 0x40 and address 0x110040.
    fn kernel(bss: u32) -> Vec<u8> {
        let mut k = vec![0x90u8; 0x200];
        let flags = 0x0001_0007u32;
        let words = [
            HEADER_MAGIC,
            flags,
            0u32.wrapping_sub(HEADER_MAGIC.wrapping_add(flags)),
            0x11_0040,
            0x11_0000,
            0x11_0200,
            0x11_0200 + bss,
            0x11_0080,
        ];
        for (n, w) in words.iter().enumerate() {
            put32(&mut k, 0x40 + n * 4, *w);
        }
        k
    }

    #[test]
    fn the_kernel_loads_at_its_addresses() {
        let k = kernel(0x1000);
        let i = load(&k).unwrap();
        assert_eq!(i.entry, 0x11_0080);
        assert_eq!(i.end, 0x11_1200);
        assert_eq!(i.segments.len(), 1);
        assert_eq!(i.segments[0].0, 0x11_0000);
        assert_eq!(i.segments[0].1, k);

        let mut bad = k.clone();
        put32(&mut bad, 0x48, 0);
        assert!(load(&bad).unwrap_err().contains("no multiboot header"));
        let mut elf = kernel(0);
        put32(&mut elf, 0x44, 3);
        put32(&mut elf, 0x48, 0u32.wrapping_sub(HEADER_MAGIC + 3));
        assert!(load(&elf).unwrap_err().contains("no ELF"));
        assert!(load(&k[..0x100]).unwrap_err().contains("shorter"));
        let mut low = kernel(0);
        for (o, v) in [
            (0x4c, 0x8040),
            (0x50, 0x8000),
            (0x54, 0x8200),
            (0x58, 0),
            (0x5c, 0x8080),
        ] {
            put32(&mut low, o, v);
        }
        assert!(load(&low).unwrap_err().contains("below 1 MiB"));
    }

    #[test]
    fn the_block_has_the_map_and_the_ini() {
        let ini = b"console=0\nbootargs=local!/dev/sdH0/fs\n";
        let blk = info(&Info {
            mem: 512 << 20,
            cmdline: "*acpi=1",
            ini: Some(ini),
        })
        .unwrap();
        assert_eq!(le32(&blk, 0), 1 | 4 | 8 | 64);
        assert_eq!(le32(&blk, 4), 639);
        assert_eq!(le32(&blk, 8), (511 << 20) / 1024);
        let cmd = le32(&blk, 16) as usize - INFO as usize;
        assert_eq!(&blk[cmd..cmd + 8], b"*acpi=1\0");
        assert_eq!(le32(&blk, 20), 1);
        let module = le32(&blk, 24) as usize - INFO as usize;
        let (from, to) = (
            le32(&blk, module) as usize - INFO as usize,
            le32(&blk, module + 4) as usize - INFO as usize,
        );
        assert_eq!(&blk[from..to], ini);
        // The map, read as multibootargs of the tree reads it.
        let (len, at) = (
            le32(&blk, 44) as usize,
            le32(&blk, 48) as usize - INFO as usize,
        );
        let mut seen = Vec::new();
        let mut p = at;
        while p < at + len {
            let size = le32(&blk, p) as usize;
            let start = u64::from(le32(&blk, p + 4)) | u64::from(le32(&blk, p + 8)) << 32;
            let n = u64::from(le32(&blk, p + 12)) | u64::from(le32(&blk, p + 16)) << 32;
            seen.push((start, n, le32(&blk, p + 20)));
            p += size + 4;
        }
        assert_eq!(seen, memory_map(512 << 20));
        assert_eq!(seen[3], (0x10_0000, (512 << 20) - 0x10_0000, USABLE));

        assert!(
            info(&Info {
                mem: 1 << 30,
                cmdline: "",
                ini: Some(&vec![b'a'; INI_MAX + 1]),
            })
            .is_err()
        );
        let none = info(&Info {
            mem: 1 << 30,
            ..Info::default()
        })
        .unwrap();
        assert_eq!(
            le32(&none, 0) & 8,
            0,
            "no module when there is no plan9.ini"
        );
    }
}
