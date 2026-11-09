//! A reader of the tar archives that a guest writes to its collect disk.
//!
//! The host must not unpack a tree from 9front to read it, because 25 paths of the tree differ
//! only by case, and the file system of macOS keeps one of each pair (plan 11.8). This reader
//! gives the names and the places of the files in the archive, and the host reads each file
//! from the archive itself.
//!
//! It reads ustar headers with a prefix, pax headers with a `path` record, and GNU long names.
//! The archive ends at two zero blocks. A disk that holds no end is full, and the archive on it
//! is not complete.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const BLOCK: u64 = 512;
const BLOCK_LEN: usize = 512;

/// One regular file in an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path, with no leading `/` or `./`.
    pub name: String,
    /// The offset of the contents in the archive.
    pub offset: u64,
    /// The size of the contents.
    pub size: u64,
}

/// The files of an archive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Archive {
    /// The regular files, in the order of the archive.
    pub files: Vec<Entry>,
    /// The length of the archive up to and with its two zero blocks.
    pub end: u64,
}

fn octal(field: &[u8]) -> Result<u64, String> {
    let text: String = field
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect();
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).map_err(|_| format!("bad octal field {text:?}"))
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

fn clean(name: &str) -> String {
    let mut n = name.trim_start_matches('/');
    while let Some(rest) = n.strip_prefix("./") {
        n = rest;
    }
    n.to_string()
}

/// The `path` record of a pax header.
fn pax_path(data: &[u8]) -> Option<String> {
    let mut rest = data;
    while !rest.is_empty() {
        let space = rest.iter().position(|&b| b == b' ')?;
        let len: usize = std::str::from_utf8(&rest[..space]).ok()?.parse().ok()?;
        if len <= space || len > rest.len() {
            return None;
        }
        let record = &rest[space + 1..len];
        let record = record.strip_suffix(b"\n").unwrap_or(record);
        if let Some(path) = record.strip_prefix(b"path=") {
            return Some(String::from_utf8_lossy(path).into_owned());
        }
        rest = &rest[len..];
    }
    None
}

/// Read the list of files of the archive in `r`, which is `len` bytes long. Fail when the
/// archive has no end before `len`, as on a full disk.
pub fn read<R: Read + Seek>(r: &mut R, len: u64) -> Result<Archive, String> {
    let mut a = Archive::default();
    let mut pos = 0u64;
    let mut long_name: Option<String> = None;
    let mut zeros = 0;
    let mut header = [0u8; BLOCK_LEN];
    while pos + BLOCK <= len {
        r.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        r.read_exact(&mut header).map_err(|e| e.to_string())?;
        pos += BLOCK;
        if header.iter().all(|&b| b == 0) {
            zeros += 1;
            if zeros == 2 {
                a.end = pos;
                return Ok(a);
            }
            continue;
        }
        zeros = 0;
        let size = octal(&header[124..136])?;
        let data = pos;
        pos += size.div_ceil(BLOCK) * BLOCK;
        if pos > len {
            break;
        }
        match header[156] {
            b'x' | b'L' => {
                let mut buf = vec![0u8; usize::try_from(size).map_err(|e| e.to_string())?];
                r.seek(SeekFrom::Start(data)).map_err(|e| e.to_string())?;
                r.read_exact(&mut buf).map_err(|e| e.to_string())?;
                long_name = if header[156] == b'x' {
                    pax_path(&buf)
                } else {
                    Some(text(&buf))
                };
            }
            b'0' | 0 => {
                let name = long_name.take().unwrap_or_else(|| {
                    let prefix = text(&header[345..500]);
                    let name = text(&header[0..100]);
                    if header[257..262] == *b"ustar" && !prefix.is_empty() {
                        format!("{prefix}/{name}")
                    } else {
                        name
                    }
                });
                a.files.push(Entry {
                    name: clean(&name),
                    offset: data,
                    size,
                });
            }
            _ => long_name = None,
        }
    }
    Err("the archive has no end; the collect disk is full, so give a larger --collect".into())
}

/// Read the list of files of the archive file at `path`.
pub fn list(path: &Path) -> Result<Archive, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = f
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    read(&mut f, len).map_err(|e| format!("{}: {e}", path.display()))
}

/// The contents of `e` in the archive file `f`.
pub fn contents(f: &mut std::fs::File, e: &Entry) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; usize::try_from(e.size).map_err(|x| x.to_string())?];
    f.seek(SeekFrom::Start(e.offset))
        .and_then(|_| f.read_exact(&mut buf))
        .map_err(|x| format!("{}: {x}", e.name))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn header(name: &str, size: u64, kind: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        let s = format!("{size:011o}");
        h[124..135].copy_from_slice(s.as_bytes());
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h
    }

    fn file(name: &str, data: &[u8], kind: u8) -> Vec<u8> {
        let mut v = header(name, data.len() as u64, kind);
        let mut d = data.to_vec();
        d.resize(data.len().div_ceil(512) * 512, 0);
        v.extend(d);
        v
    }

    #[test]
    fn names_of_every_kind_are_read() {
        let mut t = Vec::new();
        t.extend(file("/sys/src/a.c", b"int a;\n", b'0'));
        t.extend(header("sys/src/", 0, b'5'));
        t.extend(file("ignored", b"19 path=sys/src/C1\n", b'x'));
        t.extend(file("C1", b"upper", b'0'));
        t.extend(file("././L", b"sys/src/c1", b'L'));
        t.extend(file("c1", b"lower", b'0'));
        t.extend(vec![0u8; 1024]);
        let end = t.len() as u64;
        t.extend(vec![0u8; 4096]);
        let len = t.len() as u64;
        let a = read(&mut Cursor::new(&t), len).unwrap();
        let names: Vec<&str> = a.files.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sys/src/a.c", "sys/src/C1", "sys/src/c1"]);
        assert_eq!(a.end, end);
        let c1 = &a.files[2];
        let at = usize::try_from(c1.offset).unwrap();
        assert_eq!(&t[at..at + 5], b"lower");
    }

    #[test]
    fn an_archive_with_no_end_is_full() {
        let mut t = file("a", b"x", b'0');
        t.extend(header("b", 4096, b'0'));
        let len = t.len() as u64;
        let e = read(&mut Cursor::new(&t), len).unwrap_err();
        assert!(e.contains("collect disk is full"), "{e}");
    }
}
