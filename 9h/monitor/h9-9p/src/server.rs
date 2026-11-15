//! The answers of the server, from the host file system (plan 10.5, the host's files).
//!
//! A fid holds the path of its file as elements below the root, so `..` at the root stays at
//! the root, and no walk leaves the directory. Names are the host names as they are. A name
//! with `/` cannot be walked or made. On a host that does not tell case apart, a create or a
//! rename to a name that differs only by case from a name in the directory is refused, so the
//! two names of the tree that differ only by case do not become one file.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use crate::msg::{self, DMDIR, IOHDRSZ, NOFID, ORCLOSE, OTRUNC, QTDIR, Qid, R, Stat, T};

/// The largest message that the server takes.
const MAXMSIZE: u32 = 64 * 1024 + IOHDRSZ;

/// What the server serves.
#[derive(Debug, Clone)]
pub struct Options {
    /// The host directory that is the root.
    pub root: PathBuf,
    /// The owner, group and last writer of each file, the user of the session.
    pub user: String,
    /// Refuse every change.
    pub readonly: bool,
}

enum Opened {
    File(File),
    Dir {
        /// The `stat` entries of the directory, one after the other.
        buf: Vec<u8>,
        /// The offsets in `buf` where an entry starts, and its end.
        starts: Vec<usize>,
        /// The offset after the last read.
        next: u64,
    },
}

struct Fid {
    elems: Vec<String>,
    qid: Qid,
    open: Option<Opened>,
    read: bool,
    write: bool,
    rclose: bool,
}

/// A 9P2000 server over a host directory.
pub struct Server {
    o: Options,
    msize: u32,
    fids: HashMap<u32, Fid>,
}

fn ename(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "file does not exist".into(),
        io::ErrorKind::PermissionDenied => "permission denied".into(),
        io::ErrorKind::AlreadyExists => "file already exists".into(),
        io::ErrorKind::DirectoryNotEmpty => "directory not empty".into(),
        io::ErrorKind::IsADirectory => "file is a directory".into(),
        io::ErrorKind::NotADirectory => "not a directory".into(),
        _ => e.to_string(),
    }
}

fn qid_of(m: &std::fs::Metadata) -> Qid {
    Qid {
        ty: if m.is_dir() { QTDIR } else { 0 },
        vers: (m.mtime().cast_unsigned() as u32)
            ^ (m.mtime_nsec().cast_unsigned() as u32)
            ^ (m.size() as u32),
        path: m.ino() ^ (m.dev() << 48),
    }
}

fn good_name(n: &str) -> bool {
    !n.is_empty() && n != "." && n != ".." && !n.contains(['/', '\0'])
}

impl Server {
    /// A server for the options.
    #[must_use]
    pub fn new(o: Options) -> Server {
        Server {
            o,
            msize: MAXMSIZE,
            fids: HashMap::new(),
        }
    }

    fn path(&self, elems: &[String]) -> PathBuf {
        elems.iter().fold(self.o.root.clone(), |p, e| p.join(e))
    }

    fn stat_of(&self, elems: &[String], m: &std::fs::Metadata) -> Stat {
        let secs = |t: io::Result<std::time::SystemTime>| {
            t.ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as u32)
        };
        Stat {
            ty: 0,
            dev: 0,
            qid: qid_of(m),
            mode: (m.mode() & 0o777) | if m.is_dir() { DMDIR } else { 0 },
            atime: secs(m.accessed()),
            mtime: secs(m.modified()),
            length: if m.is_dir() { 0 } else { m.len() },
            name: elems.last().cloned().unwrap_or_else(|| "/".into()),
            uid: self.o.user.clone(),
            gid: self.o.user.clone(),
            muid: self.o.user.clone(),
        }
    }

    /// The `iounit` of an open file.
    fn iounit(&self) -> u32 {
        self.msize - IOHDRSZ
    }

    /// Answer one message, with its 4-byte size at the start. The reply has its size too.
    pub fn handle(&mut self, req: &[u8]) -> Vec<u8> {
        match msg::read_t(req) {
            Ok((tag, t)) => {
                let r = self.answer(t).unwrap_or_else(|ename| R::Error { ename });
                msg::write_r(tag, &r)
            }
            Err(ename) => {
                let tag = req
                    .get(5..7)
                    .map_or(msg::NOTAG, |b| u16::from_le_bytes([b[0], b[1]]));
                msg::write_r(tag, &R::Error { ename })
            }
        }
    }

    /// Answer the messages of a stream until it ends.
    pub fn serve(&mut self, mut s: impl Read + Write) -> io::Result<()> {
        loop {
            let mut size = [0u8; 4];
            match s.read_exact(&mut size) {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                r => r?,
            }
            let n = u32::from_le_bytes(size);
            if !(7..=MAXMSIZE).contains(&n) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("a 9P message has the size {n}"),
                ));
            }
            let mut m = vec![0u8; n as usize];
            m[..4].copy_from_slice(&size);
            s.read_exact(&mut m[4..])?;
            s.write_all(&self.handle(&m))?;
            s.flush()?;
        }
    }

    fn fid(&mut self, fid: u32) -> Result<&mut Fid, String> {
        self.fids.get_mut(&fid).ok_or_else(|| "unknown fid".into())
    }

    fn answer(&mut self, t: T) -> Result<R, String> {
        match t {
            T::Version { msize, version } => {
                self.fids.clear();
                self.msize = msize.clamp(256, MAXMSIZE);
                let version = if version.starts_with(msg::VERSION) {
                    msg::VERSION
                } else {
                    "unknown"
                };
                Ok(R::Version {
                    msize: self.msize,
                    version: version.into(),
                })
            }
            T::Auth { .. } => Err("authentication is not required".into()),
            T::Attach { fid, afid, .. } => {
                if afid != NOFID {
                    return Err("authentication is not required".into());
                }
                if self.fids.contains_key(&fid) {
                    return Err("fid in use".into());
                }
                let m = std::fs::metadata(&self.o.root).map_err(|e| ename(&e))?;
                let qid = qid_of(&m);
                self.fids.insert(
                    fid,
                    Fid {
                        elems: Vec::new(),
                        qid,
                        open: None,
                        read: false,
                        write: false,
                        rclose: false,
                    },
                );
                Ok(R::Attach { qid })
            }
            T::Flush { .. } => Ok(R::Flush),
            T::Walk { fid, newfid, names } => self.walk(fid, newfid, &names),
            T::Open { fid, mode } => self.open(fid, mode),
            T::Create {
                fid,
                name,
                perm,
                mode,
            } => self.create(fid, &name, perm, mode),
            T::Read { fid, offset, count } => self.read(fid, offset, count),
            T::Write { fid, offset, data } => {
                let iounit = self.iounit() as usize;
                let f = self.fid(fid)?;
                match &f.open {
                    Some(Opened::File(file)) if f.write => {
                        let data = &data[..data.len().min(iounit)];
                        file.write_all_at(data, offset).map_err(|e| ename(&e))?;
                        Ok(R::Write {
                            count: data.len() as u32,
                        })
                    }
                    Some(Opened::Dir { .. }) => Err("file is a directory".into()),
                    Some(Opened::File(_)) => Err("the file is not open for writing".into()),
                    None => Err("fid is not open".into()),
                }
            }
            T::Clunk { fid } => {
                let f = self.fids.remove(&fid).ok_or("unknown fid")?;
                if f.rclose {
                    let p = self.path(&f.elems);
                    drop(f);
                    let _ = std::fs::remove_file(&p).or_else(|_| std::fs::remove_dir(&p));
                }
                Ok(R::Clunk)
            }
            T::Remove { fid } => {
                let f = self.fids.remove(&fid).ok_or("unknown fid")?;
                if self.o.readonly {
                    return Err("permission denied".into());
                }
                if f.elems.is_empty() {
                    return Err("the root cannot be removed".into());
                }
                let p = self.path(&f.elems);
                drop(f);
                let r = if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
                    std::fs::remove_dir(&p)
                } else {
                    std::fs::remove_file(&p)
                };
                r.map_err(|e| ename(&e))?;
                Ok(R::Remove)
            }
            T::Stat { fid } => {
                let elems = self.fid(fid)?.elems.clone();
                let m = std::fs::metadata(self.path(&elems)).map_err(|e| ename(&e))?;
                Ok(R::Stat {
                    stat: self.stat_of(&elems, &m),
                })
            }
            T::Wstat { fid, stat } => self.wstat(fid, &stat),
        }
    }

    fn walk(&mut self, fid: u32, newfid: u32, names: &[String]) -> Result<R, String> {
        let f = self.fids.get(&fid).ok_or("unknown fid")?;
        if f.open.is_some() {
            return Err("fid is open".into());
        }
        if newfid != fid && self.fids.contains_key(&newfid) {
            return Err("fid in use".into());
        }
        let mut elems = f.elems.clone();
        let mut qid = f.qid;
        let mut qids = Vec::new();
        for (i, n) in names.iter().enumerate() {
            if qid.ty & QTDIR == 0 {
                if i == 0 {
                    return Err("not a directory".into());
                }
                break;
            }
            let mut next = elems.clone();
            if n == ".." {
                next.pop();
            } else if good_name(n) {
                next.push(n.clone());
            } else if i == 0 {
                return Err(format!("{n:?} is not a name that a walk can take"));
            } else {
                break;
            }
            match std::fs::metadata(self.path(&next)) {
                Ok(m) => {
                    qid = qid_of(&m);
                    qids.push(qid);
                    elems = next;
                }
                Err(e) if i == 0 => return Err(ename(&e)),
                Err(_) => break,
            }
        }
        if qids.len() == names.len() {
            self.fids.insert(
                newfid,
                Fid {
                    elems,
                    qid,
                    open: None,
                    read: false,
                    write: false,
                    rclose: false,
                },
            );
        }
        Ok(R::Walk { qids })
    }

    fn dir_entries(&self, elems: &[String]) -> io::Result<Opened> {
        let mut names: Vec<String> = std::fs::read_dir(self.path(elems))?
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| good_name(n))
            .collect();
        names.sort();
        let mut buf = Vec::new();
        let mut starts = Vec::new();
        for n in names {
            let mut child = elems.to_vec();
            child.push(n);
            // A link to nothing is not in the listing, as it cannot be walked to.
            if let Ok(m) = std::fs::metadata(self.path(&child)) {
                starts.push(buf.len());
                buf.extend_from_slice(&msg::stat_bytes(&self.stat_of(&child, &m)));
            }
        }
        starts.push(buf.len());
        Ok(Opened::Dir {
            buf,
            starts,
            next: 0,
        })
    }

    fn open_file(&self, elems: &[String], mode: u8, new: Option<u32>) -> Result<Opened, String> {
        let acc = mode & 3;
        let write = acc == msg::OWRITE || acc == msg::ORDWR;
        let mut o = OpenOptions::new();
        o.read(acc != msg::OWRITE).write(write);
        if mode & OTRUNC != 0 {
            o.write(true).truncate(true);
        }
        if let Some(perm) = new {
            o.write(true).create_new(true);
            std::os::unix::fs::OpenOptionsExt::mode(&mut o, perm & 0o777);
        }
        o.open(self.path(elems))
            .map(Opened::File)
            .map_err(|e| ename(&e))
    }

    fn open(&mut self, fid: u32, mode: u8) -> Result<R, String> {
        let iounit = self.iounit();
        let readonly = self.o.readonly;
        let f = self.fids.get(&fid).ok_or("unknown fid")?;
        if f.open.is_some() {
            return Err("fid is open".into());
        }
        let acc = mode & 3;
        let write = acc == msg::OWRITE || acc == msg::ORDWR || mode & OTRUNC != 0;
        if readonly && (write || mode & ORCLOSE != 0) {
            return Err("permission denied".into());
        }
        let elems = f.elems.clone();
        let m = std::fs::metadata(self.path(&elems)).map_err(|e| ename(&e))?;
        let opened = if m.is_dir() {
            if write {
                return Err("file is a directory".into());
            }
            self.dir_entries(&elems).map_err(|e| ename(&e))?
        } else {
            self.open_file(&elems, mode, None)?
        };
        let qid = qid_of(&m);
        let f = self.fid(fid)?;
        f.qid = qid;
        f.open = Some(opened);
        f.read = acc != msg::OWRITE;
        f.write = acc == msg::OWRITE || acc == msg::ORDWR;
        f.rclose = mode & ORCLOSE != 0;
        Ok(R::Open { qid, iounit })
    }

    /// Refuse a new name that differs only by case from a name in the directory, when the
    /// host does not tell case apart.
    fn case_check(dir: &Path, name: &str) -> Result<(), String> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Ok(());
        };
        for e in rd.filter_map(Result::ok) {
            let other = e.file_name();
            let other = other.to_string_lossy();
            if other != name && other.eq_ignore_ascii_case(name) {
                let probe = dir.join(name);
                if std::fs::symlink_metadata(&probe).is_ok() {
                    return Err(format!(
                        "{name} is the same host file as {other}, because the host does not tell case apart"
                    ));
                }
            }
        }
        Ok(())
    }

    fn create(&mut self, fid: u32, name: &str, perm: u32, mode: u8) -> Result<R, String> {
        let iounit = self.iounit();
        if self.o.readonly {
            return Err("permission denied".into());
        }
        if !good_name(name) {
            return Err(format!("{name:?} is not a name that a create can make"));
        }
        let f = self.fids.get(&fid).ok_or("unknown fid")?;
        if f.open.is_some() {
            return Err("fid is open".into());
        }
        if f.qid.ty & QTDIR == 0 {
            return Err("not a directory".into());
        }
        let dir = self.path(&f.elems);
        let mut elems = f.elems.clone();
        elems.push(name.to_string());
        Self::case_check(&dir, name)?;
        let dperm = std::fs::metadata(&dir).map_err(|e| ename(&e))?.mode() & 0o777;
        let opened = if perm & DMDIR != 0 {
            if mode & 3 != msg::OREAD && mode & 3 != msg::OEXEC {
                return Err("file is a directory".into());
            }
            let p = self.path(&elems);
            std::fs::create_dir(&p).map_err(|e| ename(&e))?;
            let bits = perm & (!0o777 | dperm) & 0o777;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(bits));
            self.dir_entries(&elems).map_err(|e| ename(&e))?
        } else {
            let bits = perm & (!0o666 | (dperm & 0o666));
            self.open_file(&elems, mode, Some(bits))?
        };
        let m = std::fs::metadata(self.path(&elems)).map_err(|e| ename(&e))?;
        let qid = qid_of(&m);
        let acc = mode & 3;
        let f = self.fid(fid)?;
        f.elems = elems;
        f.qid = qid;
        f.open = Some(opened);
        f.read = acc != msg::OWRITE;
        f.write = acc == msg::OWRITE || acc == msg::ORDWR;
        f.rclose = mode & ORCLOSE != 0;
        Ok(R::Create { qid, iounit })
    }

    fn read(&mut self, fid: u32, offset: u64, count: u32) -> Result<R, String> {
        let count = count.min(self.iounit()) as usize;
        let f = self.fid(fid)?;
        match &mut f.open {
            None => Err("fid is not open".into()),
            Some(Opened::File(_)) if !f.read => Err("the file is not open for reading".into()),
            Some(Opened::File(file)) => {
                let mut data = vec![0u8; count];
                let mut got = 0;
                while got < count {
                    match file.read_at(&mut data[got..], offset + got as u64) {
                        Ok(0) => break,
                        Ok(n) => got += n,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(e) => return Err(ename(&e)),
                    }
                }
                data.truncate(got);
                Ok(R::Read { data })
            }
            Some(Opened::Dir { buf, starts, next }) => {
                if offset != 0 && offset != *next {
                    return Err(
                        "the offset of a directory read is not where the last read ended".into(),
                    );
                }
                let from = offset as usize;
                let Ok(i) = starts.binary_search(&from) else {
                    return Err(
                        "the offset of a directory read is not where the last read ended".into(),
                    );
                };
                let mut to = from;
                for &s in &starts[i + 1..] {
                    if s - from > count {
                        break;
                    }
                    to = s;
                }
                if to == from && from < buf.len() {
                    return Err("the count of the read is too small for a directory entry".into());
                }
                *next = to as u64;
                Ok(R::Read {
                    data: buf[from..to].to_vec(),
                })
            }
        }
    }

    fn wstat(&mut self, fid: u32, st: &Stat) -> Result<R, String> {
        if self.o.readonly {
            return Err("permission denied".into());
        }
        let elems = self.fids.get(&fid).ok_or("unknown fid")?.elems.clone();
        let p = self.path(&elems);
        let m = std::fs::metadata(&p).map_err(|e| ename(&e))?;
        let user = &self.o.user;
        if (!st.uid.is_empty() && st.uid != *user) || (!st.gid.is_empty() && st.gid != *user) {
            return Err("the owner and the group of a host file cannot change".into());
        }
        if st.mode != !0 && (st.mode & DMDIR != 0) != m.is_dir() {
            return Err("the directory bit cannot change".into());
        }
        if st.length != !0 && st.length != m.len() {
            if m.is_dir() {
                return Err("file is a directory".into());
            }
            OpenOptions::new()
                .write(true)
                .open(&p)
                .and_then(|f| f.set_len(st.length))
                .map_err(|e| ename(&e))?;
        }
        if st.mode != !0 {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(st.mode & 0o777))
                .map_err(|e| ename(&e))?;
        }
        if st.mtime != !0 {
            let t = UNIX_EPOCH + Duration::from_secs(u64::from(st.mtime));
            File::open(&p)
                .and_then(|f| f.set_modified(t))
                .map_err(|e| ename(&e))?;
        }
        if !st.name.is_empty() && elems.last() != Some(&st.name) {
            if elems.is_empty() {
                return Err("the root cannot be renamed".into());
            }
            if !good_name(&st.name) {
                return Err(format!(
                    "{:?} is not a name that a rename can make",
                    st.name
                ));
            }
            let dir = p.parent().unwrap_or(&self.o.root).to_path_buf();
            Self::case_check(&dir, &st.name)?;
            let to = dir.join(&st.name);
            if std::fs::symlink_metadata(&to).is_ok() {
                return Err("file already exists".into());
            }
            std::fs::rename(&p, &to).map_err(|e| ename(&e))?;
            let mut new = elems.clone();
            if let Some(last) = new.last_mut() {
                last.clone_from(&st.name);
            }
            for f in self.fids.values_mut() {
                if f.elems.starts_with(&elems) {
                    f.elems.splice(..elems.len(), new.iter().cloned());
                }
            }
        }
        Ok(R::Wstat)
    }
}

#[cfg(test)]
mod tests {
    use super::{Options, Server};
    use crate::msg::{self, DMDIR, NOFID, ORDWR, OREAD, OTRUNC, OWRITE, QTDIR, R, Stat, T};
    use std::path::PathBuf;

    struct Client {
        s: Server,
        root: PathBuf,
    }

    impl Client {
        fn new(name: &str, readonly: bool) -> Client {
            let root = std::env::temp_dir().join(format!("h9-9p-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("lib")).unwrap();
            std::fs::write(root.join("lib/profile"), "bind -a $home/bin/rc /bin\n").unwrap();
            std::fs::write(root.join("b"), "").unwrap();
            let mut c = Client {
                s: Server::new(Options {
                    root: root.clone(),
                    user: "glenda".into(),
                    readonly,
                }),
                root,
            };
            assert_eq!(
                c.rpc(T::Version {
                    msize: 8192,
                    version: "9P2000".into()
                }),
                R::Version {
                    msize: 8192,
                    version: "9P2000".into()
                }
            );
            let r = c.rpc(T::Attach {
                fid: 0,
                afid: NOFID,
                uname: "glenda".into(),
                aname: String::new(),
            });
            assert!(matches!(r, R::Attach { qid } if qid.ty == QTDIR), "{r:?}");
            c
        }

        #[allow(clippy::needless_pass_by_value)]
        fn rpc(&mut self, t: T) -> R {
            let reply = self.s.handle(&msg::write_t(1, &t));
            let (tag, r) = msg::read_r(&reply).unwrap();
            assert_eq!(tag, 1);
            r
        }

        fn walk(&mut self, fid: u32, newfid: u32, path: &str) -> R {
            self.rpc(T::Walk {
                fid,
                newfid,
                names: path
                    .split('/')
                    .filter(|n| !n.is_empty())
                    .map(String::from)
                    .collect(),
            })
        }

        fn err(&mut self, t: T) -> String {
            match self.rpc(t) {
                R::Error { ename } => ename,
                r => panic!("{r:?} is not an error"),
            }
        }
    }

    impl Drop for Client {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn walk_open_read_and_list() {
        let mut c = Client::new("read", false);
        assert!(matches!(c.walk(0, 1, "lib/profile"), R::Walk { qids } if qids.len() == 2));
        assert!(matches!(
            c.rpc(T::Open {
                fid: 1,
                mode: OREAD
            }),
            R::Open { iounit: 8168, .. }
        ));
        assert_eq!(
            c.rpc(T::Read {
                fid: 1,
                offset: 5,
                count: 2
            }),
            R::Read {
                data: b"-a".to_vec()
            }
        );
        assert_eq!(
            c.err(T::Write {
                fid: 1,
                offset: 0,
                data: vec![1]
            }),
            "the file is not open for writing"
        );
        assert_eq!(c.rpc(T::Clunk { fid: 1 }), R::Clunk);

        // A walk that stops part way gives the qids it found, and makes no fid.
        assert!(matches!(c.walk(0, 2, "lib/none"), R::Walk { qids } if qids.len() == 1));
        assert_eq!(c.err(T::Stat { fid: 2 }), "unknown fid");
        assert_eq!(
            c.err(T::Walk {
                fid: 0,
                newfid: 2,
                names: vec!["none".into()]
            }),
            "file does not exist"
        );
        // `..` at the root stays at the root.
        assert!(matches!(c.walk(0, 3, "../../lib"), R::Walk { qids } if qids.len() == 3));
        assert!(
            matches!(c.rpc(T::Stat { fid: 3 }), R::Stat { stat } if stat.name == "lib" && stat.mode & DMDIR != 0)
        );
        assert_eq!(c.rpc(T::Clunk { fid: 3 }), R::Clunk);

        assert!(matches!(c.walk(0, 4, ""), R::Walk { qids } if qids.is_empty()));
        assert!(matches!(
            c.rpc(T::Open {
                fid: 4,
                mode: OREAD
            }),
            R::Open { .. }
        ));
        let R::Read { data } = c.rpc(T::Read {
            fid: 4,
            offset: 0,
            count: 8000,
        }) else {
            panic!()
        };
        let names: Vec<String> = msg::read_stats(&data)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["b", "lib"]);
        let end = data.len() as u64;
        assert_eq!(
            c.rpc(T::Read {
                fid: 4,
                offset: end,
                count: 8000
            }),
            R::Read { data: vec![] }
        );
        assert!(
            c.err(T::Read {
                fid: 4,
                offset: 3,
                count: 8000
            })
            .contains("offset")
        );
        // A count that holds one entry gives one entry.
        let R::Read { data } = c.rpc(T::Read {
            fid: 4,
            offset: 0,
            count: 80,
        }) else {
            panic!()
        };
        assert_eq!(msg::read_stats(&data).unwrap().len(), 1);
        assert!(
            c.err(T::Read {
                fid: 4,
                offset: data.len() as u64,
                count: 10
            })
            .contains("too small")
        );
        assert_eq!(
            c.err(T::Open {
                fid: 4,
                mode: OREAD
            }),
            "fid is open"
        );
        assert_eq!(
            c.err(T::Walk {
                fid: 4,
                newfid: 5,
                names: vec![]
            }),
            "fid is open"
        );
        assert!(
            c.err(T::Walk {
                fid: 0,
                newfid: 5,
                names: vec!["a/b".into()]
            })
            .contains("not a name")
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn create_write_rename_and_remove() {
        let mut c = Client::new("write", false);
        assert!(matches!(c.walk(0, 1, "lib"), R::Walk { .. }));
        assert!(matches!(
            c.rpc(T::Create { fid: 1, name: "new".into(), perm: 0o644, mode: ORDWR }),
            R::Create { qid, .. } if qid.ty == 0
        ));
        assert_eq!(
            c.rpc(T::Write {
                fid: 1,
                offset: 0,
                data: b"hello world".to_vec()
            }),
            R::Write { count: 11 }
        );
        assert_eq!(
            c.rpc(T::Read {
                fid: 1,
                offset: 6,
                count: 100
            }),
            R::Read {
                data: b"world".to_vec()
            }
        );
        assert_eq!(c.rpc(T::Clunk { fid: 1 }), R::Clunk);
        assert_eq!(
            std::fs::read(c.root.join("lib/new")).unwrap(),
            b"hello world"
        );

        assert!(matches!(c.walk(0, 2, "lib/new"), R::Walk { .. }));
        let keep = Stat {
            mode: !0,
            atime: !0,
            mtime: !0,
            length: !0,
            qid: msg::Qid {
                ty: !0,
                vers: !0,
                path: !0,
            },
            ty: !0,
            dev: !0,
            ..Stat::default()
        };
        let rename = Stat {
            name: "old".into(),
            length: 5,
            ..keep.clone()
        };
        assert_eq!(
            c.rpc(T::Wstat {
                fid: 2,
                stat: rename
            }),
            R::Wstat
        );
        assert_eq!(std::fs::read(c.root.join("lib/old")).unwrap(), b"hello");
        assert!(
            matches!(c.rpc(T::Stat { fid: 2 }), R::Stat { stat } if stat.name == "old" && stat.length == 5 && stat.uid == "glenda")
        );
        let chown = Stat {
            uid: "bootes".into(),
            ..keep.clone()
        };
        assert!(
            c.err(T::Wstat {
                fid: 2,
                stat: chown
            })
            .contains("owner")
        );
        let mode = Stat {
            mode: 0o600,
            mtime: 1_000_000,
            ..keep
        };
        assert_eq!(c.rpc(T::Wstat { fid: 2, stat: mode }), R::Wstat);
        assert!(
            matches!(c.rpc(T::Stat { fid: 2 }), R::Stat { stat } if stat.mode == 0o600 && stat.mtime == 1_000_000)
        );
        assert!(matches!(
            c.rpc(T::Open {
                fid: 2,
                mode: OWRITE | OTRUNC
            }),
            R::Open { .. }
        ));
        assert_eq!(std::fs::read(c.root.join("lib/old")).unwrap(), b"");
        assert_eq!(c.rpc(T::Remove { fid: 2 }), R::Remove);
        assert!(!c.root.join("lib/old").exists());

        assert!(matches!(c.walk(0, 3, ""), R::Walk { .. }));
        assert!(matches!(
            c.rpc(T::Create { fid: 3, name: "d".into(), perm: DMDIR | 0o755, mode: OREAD }),
            R::Create { qid, .. } if qid.ty == QTDIR
        ));
        assert!(c.root.join("d").is_dir());
        assert_eq!(c.rpc(T::Remove { fid: 3 }), R::Remove);
        assert!(!c.root.join("d").exists());

        assert!(matches!(c.walk(0, 4, ""), R::Walk { .. }));
        assert_eq!(
            c.err(T::Create {
                fid: 4,
                name: "b".into(),
                perm: 0o644,
                mode: OWRITE
            }),
            "file already exists"
        );
        assert_eq!(c.err(T::Remove { fid: 0 }), "the root cannot be removed");
    }

    #[test]
    fn a_readonly_server_refuses_changes() {
        let mut c = Client::new("ro", true);
        assert!(matches!(c.walk(0, 1, "b"), R::Walk { .. }));
        assert_eq!(
            c.err(T::Open {
                fid: 1,
                mode: OWRITE
            }),
            "permission denied"
        );
        assert!(matches!(
            c.rpc(T::Open {
                fid: 1,
                mode: OREAD
            }),
            R::Open { .. }
        ));
        assert!(matches!(c.walk(0, 2, ""), R::Walk { .. }));
        assert_eq!(
            c.err(T::Create {
                fid: 2,
                name: "x".into(),
                perm: 0o644,
                mode: OWRITE
            }),
            "permission denied"
        );
        assert_eq!(c.err(T::Remove { fid: 2 }), "permission denied");
        assert!(c.root.join("b").exists());
    }

    #[test]
    fn a_stream_is_served_until_it_ends() {
        let root = std::env::temp_dir().join(format!("h9-9p-{}-stream", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut input = msg::write_t(
            1,
            &T::Version {
                msize: 1 << 20,
                version: "9P2000.u".into(),
            },
        );
        input.extend(msg::write_t(
            2,
            &T::Attach {
                fid: 0,
                afid: NOFID,
                uname: "u".into(),
                aname: String::new(),
            },
        ));
        let mut s = Server::new(Options {
            root: root.clone(),
            user: "u".into(),
            readonly: false,
        });
        let mut io = Duplex {
            input: std::io::Cursor::new(input),
            output: Vec::new(),
        };
        s.serve(&mut io).unwrap();
        let first = u32::from_le_bytes(io.output[..4].try_into().unwrap()) as usize;
        assert_eq!(
            msg::read_r(&io.output[..first]).unwrap(),
            (
                1,
                R::Version {
                    msize: 64 * 1024 + 24,
                    version: "9P2000".into()
                }
            )
        );
        assert!(matches!(
            msg::read_r(&io.output[first..]).unwrap(),
            (2, R::Attach { .. })
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    struct Duplex {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }
    impl std::io::Read for Duplex {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(b)
        }
    }
    impl std::io::Write for Duplex {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
