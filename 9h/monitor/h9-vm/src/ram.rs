//! The RAM of the guest. It is one host allocation, which the hypervisor maps at a guest
//! physical address. The monitor reads and writes it through the same mapping, and it checks
//! each guest address against the RAM before it uses it (plan 10.5).

use std::ptr::NonNull;

/// The RAM of the guest, at the guest physical addresses `base` to `base + len`.
pub struct Ram {
    base: u64,
    len: usize,
    ptr: NonNull<u8>,
}

// SAFETY: the mapping is owned by Ram and is only freed in drop. The guest also writes to it,
// so each access of the monitor copies bytes and keeps no reference into the memory.
unsafe impl Send for Ram {}
// SAFETY: as for Send. Each method copies through a raw pointer and makes no reference.
unsafe impl Sync for Ram {}

impl Ram {
    /// Allocate `len` bytes of zeroed RAM at the guest physical address `base`. `len` must be a
    /// multiple of the page size of the host.
    pub fn new(base: u64, len: usize) -> Result<Ram, String> {
        // SAFETY: an anonymous private mapping with no address hint has no other effect.
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_ANON | libc::MAP_PRIVATE,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED {
            return Err(format!(
                "cannot allocate {len} bytes of guest RAM: {}",
                std::io::Error::last_os_error()
            ));
        }
        let ptr = NonNull::new(p.cast::<u8>()).ok_or("mmap gave a null address")?;
        Ok(Ram { base, len, ptr })
    }

    /// The guest physical address of the first byte.
    #[must_use]
    pub fn base(&self) -> u64 {
        self.base
    }

    /// The size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the RAM has no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The host address of the first byte, for the hypervisor's map call.
    #[must_use]
    pub fn host_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// The offset of the guest physical range `pa` to `pa + n` in the RAM, or None when a byte of
    /// the range is not in the RAM.
    #[must_use]
    pub fn offset(&self, pa: u64, n: usize) -> Option<usize> {
        let off = usize::try_from(pa.checked_sub(self.base)?).ok()?;
        (off.checked_add(n)? <= self.len).then_some(off)
    }

    /// Copy the bytes at `pa` into `buf`. False when the range is not in the RAM.
    pub fn read(&self, pa: u64, buf: &mut [u8]) -> bool {
        let Some(off) = self.offset(pa, buf.len()) else {
            return false;
        };
        // SAFETY: offset checked that the range is in the mapping, and buf is a different object.
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr.as_ptr().add(off), buf.as_mut_ptr(), buf.len());
        };
        true
    }

    /// Copy `data` to `pa`. False when the range is not in the RAM.
    #[must_use]
    pub fn write(&self, pa: u64, data: &[u8]) -> bool {
        let Some(off) = self.offset(pa, data.len()) else {
            return false;
        };
        // SAFETY: offset checked that the range is in the mapping, and data is a different object.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.as_ptr().add(off), data.len());
        };
        true
    }
}

impl Drop for Ram {
    fn drop(&mut self) {
        // SAFETY: ptr and len are the mapping that new made, and nothing uses it after drop.
        unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}

#[cfg(test)]
mod tests {
    use super::Ram;

    #[test]
    fn an_access_outside_the_ram_is_refused() {
        let ram = Ram::new(0x4000_0000, 0x10000).unwrap();
        assert!(ram.write(0x4000_0000, b"plan 9"));
        let mut b = [0u8; 6];
        assert!(ram.read(0x4000_0000, &mut b));
        assert_eq!(&b, b"plan 9");
        assert!(!ram.write(0x3fff_ffff, b"x"));
        assert!(!ram.write(0x4000_fffc, b"12345"));
        assert!(ram.write(0x4000_fffb, b"12345"));
        assert!(!ram.read(u64::MAX, &mut b));
        assert_eq!(ram.offset(0x4001_0000, 0), Some(0x10000));
        assert_eq!(ram.offset(0x4001_0000, 1), None);
    }
}
