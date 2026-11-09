//! Raw tar disks: the way files go into a guest and come out of it.
//!
//! A guest has no network share with the host. So `r9 boot --in` writes a tar archive to a raw
//! disk file and attaches it after the root disk, and the guest reads it with `tar xf` on the
//! disk's `data` file. `r9 boot --collect` attaches a blank raw disk, the guest writes a tar
//! archive to it with `tar cf`, and the host unpacks it after the boot. A tar archive ends with
//! zero blocks, so the rest of the disk does not matter.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The size of a disk sector. Disk files are padded to a whole number of sectors.
const SECTOR: u64 = 512;

/// Room after the archive, so that the end-of-archive blocks are always on the disk.
const SLACK: u64 = 1 << 20;

/// Make the raw disk `out` from `input`, which is a directory or a tar file. A directory is
/// archived in the ustar format, with no extended attributes, so that the tar of 9front reads
/// it. Return the size of the disk.
pub fn pack(input: &Path, out: &Path) -> Result<u64, String> {
    if input.is_dir() {
        let status = Command::new("tar")
            .env("COPYFILE_DISABLE", "1")
            .arg("--format=ustar")
            .arg("-cf")
            .arg(out)
            .arg("-C")
            .arg(input)
            .arg(".")
            .status()
            .map_err(|e| format!("cannot run tar: {e}"))?;
        if !status.success() {
            return Err(format!("tar of {} failed", input.display()));
        }
    } else if input.is_file() {
        std::fs::copy(input, out).map_err(|e| format!("{}: {e}", input.display()))?;
    } else {
        return Err(format!("{} is not a directory or a file", input.display()));
    }
    let len = std::fs::metadata(out)
        .map_err(|e| format!("{}: {e}", out.display()))?
        .len();
    let size = round_up(len + SLACK);
    set_len(out, size)?;
    Ok(size)
}

/// Make the blank raw disk `out` of `mib` MiB. The file is sparse on the host.
pub fn blank(out: &Path, mib: u64) -> Result<(), String> {
    if mib == 0 {
        return Err("a collect disk needs 1 MiB or more".into());
    }
    std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    set_len(out, mib << 20)
}

/// Unpack the tar archive at the start of the raw disk `disk` into `dir`. A disk that the guest
/// did not write starts with a zero block, and gives an empty directory.
pub fn unpack(disk: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if starts_with_zero_block(disk)? {
        return Ok(());
    }
    let status = Command::new("tar")
        .arg("-xf")
        .arg(disk)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("tar could not unpack {}", disk.display()))
    }
}

/// The file names of the disks of a boot, in the work directory.
#[must_use]
pub fn in_disk(work: &Path) -> PathBuf {
    work.join("in.tar")
}

/// See [`in_disk`].
#[must_use]
pub fn collect_disk(work: &Path) -> PathBuf {
    work.join("collect.tar")
}

fn round_up(n: u64) -> u64 {
    n.div_ceil(SECTOR) * SECTOR
}

fn set_len(path: &Path, size: u64) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.set_len(size))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn starts_with_zero_block(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let mut block = [0u8; 512];
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let n = f
        .read(&mut block)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(block[..n].iter().all(|&b| b == 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("r9-disk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_directory_goes_in_and_comes_out() {
        let d = temp("roundtrip");
        let src = d.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/a.c"), "int a;\n").unwrap();
        let disk = d.join("in.tar");
        let size = pack(&src, &disk).unwrap();
        assert_eq!(size % SECTOR, 0);
        assert!(size > SLACK);
        let out = d.join("out");
        unpack(&disk, &out).unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join("sub/a.c")).unwrap(),
            "int a;\n"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_blank_disk_unpacks_to_nothing() {
        let d = temp("blank");
        let disk = d.join("collect.tar");
        blank(&disk, 2).unwrap();
        assert_eq!(std::fs::metadata(&disk).unwrap().len(), 2 << 20);
        unpack(&disk, &d.join("out")).unwrap();
        assert_eq!(std::fs::read_dir(d.join("out")).unwrap().count(), 0);
        assert!(blank(&disk, 0).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
