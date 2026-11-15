//! The messages of 9P2000, as `intro(5)` gives them. All numbers are little-endian. A string is
//! a 2-byte count and UTF-8 bytes.

/// The version that the server speaks.
pub const VERSION: &str = "9P2000";
/// The tag of `Tversion`.
pub const NOTAG: u16 = 0xffff;
/// The fid that means none.
pub const NOFID: u32 = 0xffff_ffff;
/// The bytes of a `Tread` or `Twrite` header, which `iounit` leaves for them.
pub const IOHDRSZ: u32 = 24;

/// The qid type of a directory.
pub const QTDIR: u8 = 0x80;
/// The mode bit of a directory.
pub const DMDIR: u32 = 0x8000_0000;

/// The open mode for reading.
pub const OREAD: u8 = 0;
/// The open mode for writing.
pub const OWRITE: u8 = 1;
/// The open mode for reading and writing.
pub const ORDWR: u8 = 2;
/// The open mode for executing.
pub const OEXEC: u8 = 3;
/// The open flag that truncates the file.
pub const OTRUNC: u8 = 0x10;
/// The open flag that removes the file when the fid is clunked.
pub const ORCLOSE: u8 = 0x40;

/// The identity of a file on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Qid {
    /// `QTDIR` for a directory, else 0.
    pub ty: u8,
    /// The version, which changes when the file changes.
    pub vers: u32,
    /// The number of the file, unique on the server.
    pub path: u64,
}

/// The `stat` of a file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stat {
    /// The type of the kernel device. The server sets 0.
    pub ty: u16,
    /// The instance of the device. The server sets 0.
    pub dev: u32,
    /// The qid.
    pub qid: Qid,
    /// The permission bits, with `DMDIR` for a directory.
    pub mode: u32,
    /// The time of the last read, in seconds.
    pub atime: u32,
    /// The time of the last write, in seconds.
    pub mtime: u32,
    /// The length in bytes, 0 for a directory.
    pub length: u64,
    /// The last element of the path.
    pub name: String,
    /// The owner.
    pub uid: String,
    /// The group.
    pub gid: String,
    /// The user who changed the file last.
    pub muid: String,
}

/// A message from the client. The fields have the names of `intro(5)`.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum T {
    /// `Tversion msize version`.
    Version { msize: u32, version: String },
    /// `Tauth afid uname aname`.
    Auth {
        afid: u32,
        uname: String,
        aname: String,
    },
    /// `Tattach fid afid uname aname`.
    Attach {
        fid: u32,
        afid: u32,
        uname: String,
        aname: String,
    },
    /// `Tflush oldtag`.
    Flush { oldtag: u16 },
    /// `Twalk fid newfid wname...`.
    Walk {
        fid: u32,
        newfid: u32,
        names: Vec<String>,
    },
    /// `Topen fid mode`.
    Open { fid: u32, mode: u8 },
    /// `Tcreate fid name perm mode`.
    Create {
        fid: u32,
        name: String,
        perm: u32,
        mode: u8,
    },
    /// `Tread fid offset count`.
    Read { fid: u32, offset: u64, count: u32 },
    /// `Twrite fid offset data`.
    Write {
        fid: u32,
        offset: u64,
        data: Vec<u8>,
    },
    /// `Tclunk fid`.
    Clunk { fid: u32 },
    /// `Tremove fid`.
    Remove { fid: u32 },
    /// `Tstat fid`.
    Stat { fid: u32 },
    /// `Twstat fid stat`.
    Wstat { fid: u32, stat: Stat },
}

/// A message from the server. The fields have the names of `intro(5)`.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum R {
    /// `Rversion msize version`.
    Version { msize: u32, version: String },
    /// `Rauth aqid`.
    Auth { aqid: Qid },
    /// `Rattach qid`.
    Attach { qid: Qid },
    /// `Rerror ename`.
    Error { ename: String },
    /// `Rflush`.
    Flush,
    /// `Rwalk qid...`.
    Walk { qids: Vec<Qid> },
    /// `Ropen qid iounit`.
    Open { qid: Qid, iounit: u32 },
    /// `Rcreate qid iounit`.
    Create { qid: Qid, iounit: u32 },
    /// `Rread data`.
    Read { data: Vec<u8> },
    /// `Rwrite count`.
    Write { count: u32 },
    /// `Rclunk`.
    Clunk,
    /// `Rremove`.
    Remove,
    /// `Rstat stat`.
    Stat { stat: Stat },
    /// `Rwstat`.
    Wstat,
}

/// The bytes of a message as it is built.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(0xffff);
        self.u16(n as u16);
        self.0.extend_from_slice(&b[..n]);
    }
    fn qid(&mut self, q: &Qid) {
        self.u8(q.ty);
        self.u32(q.vers);
        self.u64(q.path);
    }
    fn stat(&mut self, s: &Stat) {
        let at = self.0.len();
        self.u16(0);
        self.u16(s.ty);
        self.u32(s.dev);
        self.qid(&s.qid);
        self.u32(s.mode);
        self.u32(s.atime);
        self.u32(s.mtime);
        self.u64(s.length);
        self.str(&s.name);
        self.str(&s.uid);
        self.str(&s.gid);
        self.str(&s.muid);
        let n = (self.0.len() - at - 2) as u16;
        self.0[at..at + 2].copy_from_slice(&n.to_le_bytes());
    }
    fn begin(&mut self, ty: u8, tag: u16) {
        self.u32(0);
        self.u8(ty);
        self.u16(tag);
    }
    fn end(mut self) -> Vec<u8> {
        let n = self.0.len() as u32;
        self.0[0..4].copy_from_slice(&n.to_le_bytes());
        self.0
    }
}

/// The bytes of one `stat`, with its 2-byte size, as a directory read gives it.
#[must_use]
pub fn stat_bytes(s: &Stat) -> Vec<u8> {
    let mut w = Writer::default();
    w.stat(s);
    w.0
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or("the message is shorter than its fields")?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from(self.u32()?) | u64::from(self.u32()?) << 32)
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "a string is not UTF-8".into())
    }
    fn qid(&mut self) -> Result<Qid, String> {
        Ok(Qid {
            ty: self.u8()?,
            vers: self.u32()?,
            path: self.u64()?,
        })
    }
    fn stat(&mut self) -> Result<Stat, String> {
        let _size = self.u16()?;
        Ok(Stat {
            ty: self.u16()?,
            dev: self.u32()?,
            qid: self.qid()?,
            mode: self.u32()?,
            atime: self.u32()?,
            mtime: self.u32()?,
            length: self.u64()?,
            name: self.str()?,
            uid: self.str()?,
            gid: self.str()?,
            muid: self.str()?,
        })
    }
}

/// Read one T-message, which has its 4-byte size at the start. The result is the tag and the
/// message.
pub fn read_t(b: &[u8]) -> Result<(u16, T), String> {
    let mut r = Reader { b, at: 4 };
    let ty = r.u8()?;
    let tag = r.u16()?;
    let t = match ty {
        100 => T::Version {
            msize: r.u32()?,
            version: r.str()?,
        },
        102 => T::Auth {
            afid: r.u32()?,
            uname: r.str()?,
            aname: r.str()?,
        },
        104 => T::Attach {
            fid: r.u32()?,
            afid: r.u32()?,
            uname: r.str()?,
            aname: r.str()?,
        },
        108 => T::Flush { oldtag: r.u16()? },
        110 => {
            let fid = r.u32()?;
            let newfid = r.u32()?;
            let n = r.u16()?;
            let names = (0..n).map(|_| r.str()).collect::<Result<_, _>>()?;
            T::Walk { fid, newfid, names }
        }
        112 => T::Open {
            fid: r.u32()?,
            mode: r.u8()?,
        },
        114 => T::Create {
            fid: r.u32()?,
            name: r.str()?,
            perm: r.u32()?,
            mode: r.u8()?,
        },
        116 => T::Read {
            fid: r.u32()?,
            offset: r.u64()?,
            count: r.u32()?,
        },
        118 => {
            let fid = r.u32()?;
            let offset = r.u64()?;
            let n = r.u32()? as usize;
            T::Write {
                fid,
                offset,
                data: r.take(n)?.to_vec(),
            }
        }
        120 => T::Clunk { fid: r.u32()? },
        122 => T::Remove { fid: r.u32()? },
        124 => T::Stat { fid: r.u32()? },
        126 => {
            let fid = r.u32()?;
            let _n = r.u16()?;
            T::Wstat {
                fid,
                stat: r.stat()?,
            }
        }
        t => return Err(format!("the message type {t} is not a T-message of 9P2000")),
    };
    Ok((tag, t))
}

/// Write one R-message with its size.
#[must_use]
pub fn write_r(tag: u16, r: &R) -> Vec<u8> {
    let mut w = Writer::default();
    match r {
        R::Version { msize, version } => {
            w.begin(101, tag);
            w.u32(*msize);
            w.str(version);
        }
        R::Auth { aqid } => {
            w.begin(103, tag);
            w.qid(aqid);
        }
        R::Attach { qid } => {
            w.begin(105, tag);
            w.qid(qid);
        }
        R::Error { ename } => {
            w.begin(107, tag);
            w.str(ename);
        }
        R::Flush => w.begin(109, tag),
        R::Walk { qids } => {
            w.begin(111, tag);
            w.u16(qids.len() as u16);
            for q in qids {
                w.qid(q);
            }
        }
        R::Open { qid, iounit } | R::Create { qid, iounit } => {
            w.begin(
                if matches!(r, R::Open { .. }) {
                    113
                } else {
                    115
                },
                tag,
            );
            w.qid(qid);
            w.u32(*iounit);
        }
        R::Read { data } => {
            w.begin(117, tag);
            w.u32(data.len() as u32);
            w.0.extend_from_slice(data);
        }
        R::Write { count } => {
            w.begin(119, tag);
            w.u32(*count);
        }
        R::Clunk => w.begin(121, tag),
        R::Remove => w.begin(123, tag),
        R::Stat { stat } => {
            w.begin(125, tag);
            let b = stat_bytes(stat);
            w.u16(b.len() as u16);
            w.0.extend_from_slice(&b);
        }
        R::Wstat => w.begin(127, tag),
    }
    w.end()
}

/// Write one T-message with its size. The server does not need it. The tests and a client do.
#[must_use]
pub fn write_t(tag: u16, t: &T) -> Vec<u8> {
    let mut w = Writer::default();
    match t {
        T::Version { msize, version } => {
            w.begin(100, tag);
            w.u32(*msize);
            w.str(version);
        }
        T::Auth { afid, uname, aname } => {
            w.begin(102, tag);
            w.u32(*afid);
            w.str(uname);
            w.str(aname);
        }
        T::Attach {
            fid,
            afid,
            uname,
            aname,
        } => {
            w.begin(104, tag);
            w.u32(*fid);
            w.u32(*afid);
            w.str(uname);
            w.str(aname);
        }
        T::Flush { oldtag } => {
            w.begin(108, tag);
            w.u16(*oldtag);
        }
        T::Walk { fid, newfid, names } => {
            w.begin(110, tag);
            w.u32(*fid);
            w.u32(*newfid);
            w.u16(names.len() as u16);
            for n in names {
                w.str(n);
            }
        }
        T::Open { fid, mode } => {
            w.begin(112, tag);
            w.u32(*fid);
            w.u8(*mode);
        }
        T::Create {
            fid,
            name,
            perm,
            mode,
        } => {
            w.begin(114, tag);
            w.u32(*fid);
            w.str(name);
            w.u32(*perm);
            w.u8(*mode);
        }
        T::Read { fid, offset, count } => {
            w.begin(116, tag);
            w.u32(*fid);
            w.u64(*offset);
            w.u32(*count);
        }
        T::Write { fid, offset, data } => {
            w.begin(118, tag);
            w.u32(*fid);
            w.u64(*offset);
            w.u32(data.len() as u32);
            w.0.extend_from_slice(data);
        }
        T::Clunk { fid } => {
            w.begin(120, tag);
            w.u32(*fid);
        }
        T::Remove { fid } => {
            w.begin(122, tag);
            w.u32(*fid);
        }
        T::Stat { fid } => {
            w.begin(124, tag);
            w.u32(*fid);
        }
        T::Wstat { fid, stat } => {
            w.begin(126, tag);
            w.u32(*fid);
            let b = stat_bytes(stat);
            w.u16(b.len() as u16);
            w.0.extend_from_slice(&b);
        }
    }
    w.end()
}

/// Read one R-message, which has its 4-byte size at the start. The result is the tag and the
/// message.
pub fn read_r(b: &[u8]) -> Result<(u16, R), String> {
    let mut r = Reader { b, at: 4 };
    let ty = r.u8()?;
    let tag = r.u16()?;
    let m = match ty {
        101 => R::Version {
            msize: r.u32()?,
            version: r.str()?,
        },
        103 => R::Auth { aqid: r.qid()? },
        105 => R::Attach { qid: r.qid()? },
        107 => R::Error { ename: r.str()? },
        109 => R::Flush,
        111 => {
            let n = r.u16()?;
            R::Walk {
                qids: (0..n).map(|_| r.qid()).collect::<Result<_, _>>()?,
            }
        }
        113 | 115 => {
            let qid = r.qid()?;
            let iounit = r.u32()?;
            if ty == 113 {
                R::Open { qid, iounit }
            } else {
                R::Create { qid, iounit }
            }
        }
        117 => {
            let n = r.u32()? as usize;
            R::Read {
                data: r.take(n)?.to_vec(),
            }
        }
        119 => R::Write { count: r.u32()? },
        121 => R::Clunk,
        123 => R::Remove,
        125 => {
            let _n = r.u16()?;
            R::Stat { stat: r.stat()? }
        }
        127 => R::Wstat,
        t => {
            return Err(format!(
                "the message type {t} is not an R-message of 9P2000"
            ));
        }
    };
    Ok((tag, m))
}

/// Read the `stat` entries of the data of a directory read.
pub fn read_stats(b: &[u8]) -> Result<Vec<Stat>, String> {
    let mut r = Reader { b, at: 0 };
    let mut out = Vec::new();
    while r.at < b.len() {
        out.push(r.stat()?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Qid, R, Stat, T, read_r, read_t, write_r, write_t};

    #[test]
    fn messages_go_both_ways() {
        let stat = Stat {
            qid: Qid {
                ty: 0x80,
                vers: 3,
                path: 99,
            },
            mode: 0x8000_01ed,
            name: "lib".into(),
            uid: "glenda".into(),
            ..Stat::default()
        };
        let ts = [
            T::Version {
                msize: 8192,
                version: "9P2000".into(),
            },
            T::Walk {
                fid: 1,
                newfid: 2,
                names: vec!["usr".into(), "glenda".into()],
            },
            T::Write {
                fid: 2,
                offset: 1 << 40,
                data: b"hello".to_vec(),
            },
            T::Wstat {
                fid: 2,
                stat: stat.clone(),
            },
        ];
        for t in ts {
            let b = write_t(7, &t);
            assert_eq!(
                u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize,
                b.len()
            );
            assert_eq!(read_t(&b), Ok((7, t)));
        }
        let rs = [
            R::Error {
                ename: "file does not exist".into(),
            },
            R::Read {
                data: vec![1, 2, 3],
            },
            R::Stat { stat },
            R::Open {
                qid: Qid::default(),
                iounit: 8168,
            },
        ];
        for r in rs {
            assert_eq!(read_r(&write_r(9, &r)), Ok((9, r)));
        }
        assert!(read_t(&[11, 0, 0, 0, 99, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(read_t(&[9, 0, 0, 0, 120, 0, 0, 1]).is_err());
    }
}
