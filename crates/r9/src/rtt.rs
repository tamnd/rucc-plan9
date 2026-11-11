//! `r9 rtt`: the cost of 9P round trips from a guest to a host process (open question 10).
//!
//! `r9 rtt` listens on a TCP port of the host loopback, which QEMU user networking reaches as
//! 10.0.2.2. In the guest, `r9rtt` dials it, sends the line `9p rtt`, and runs `exportfs -r /` on
//! the connection. `r9 rtt` then sends 9P requests as rucc does when it reads a header with
//! `--sysroot=9p:`. One sequence is `walk`, `open`, `read` and `clunk` of one file, which is four
//! round trips. `r9 rtt` times each sequence and prints the median, the 90th percentile and the
//! mean.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

const TVERSION: u8 = 100;
const TATTACH: u8 = 104;
const RERROR: u8 = 107;
const TWALK: u8 = 110;
const TOPEN: u8 = 112;
const TREAD: u8 = 116;
const TCLUNK: u8 = 120;
const NOTAG: u16 = 0xffff;
const NOFID: u32 = 0xffff_ffff;

/// A 9P message as it is built.
#[derive(Debug, Default)]
pub struct Msg(Vec<u8>);

impl Msg {
    /// A new message of a type with a tag.
    #[must_use]
    pub fn new(kind: u8, tag: u16) -> Self {
        let mut m = Self(vec![0, 0, 0, 0, kind]);
        m.u16(tag);
        m
    }

    /// Add a 16-bit number.
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// Add a 32-bit number.
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// Add a 64-bit number.
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// Add a string, with its length first.
    pub fn str(&mut self, s: &str) -> &mut Self {
        let n = u16::try_from(s.len()).unwrap_or(u16::MAX);
        self.u16(n);
        self.0.extend_from_slice(&s.as_bytes()[..usize::from(n)]);
        self
    }

    /// The bytes, with the size set.
    #[must_use]
    pub fn bytes(mut self) -> Vec<u8> {
        let n = u32::try_from(self.0.len()).unwrap_or(u32::MAX);
        self.0[..4].copy_from_slice(&n.to_le_bytes());
        self.0
    }
}

/// A 9P client on one connection.
pub struct Client<S: Read + Write> {
    conn: S,
    tag: u16,
}

impl<S: Read + Write> Client<S> {
    /// A client on a connection where the 9P messages start.
    pub fn new(conn: S) -> Self {
        Self { conn, tag: 0 }
    }

    /// Send one message and read the reply. The body of the reply comes back without the size,
    /// the type and the tag. An `Rerror` gives its text as the error.
    pub fn rpc(&mut self, m: Msg, kind: u8) -> Result<Vec<u8>, String> {
        let bytes = m.bytes();
        self.conn
            .write_all(&bytes)
            .map_err(|e| format!("write: {e}"))?;
        let mut size = [0u8; 4];
        self.conn
            .read_exact(&mut size)
            .map_err(|e| format!("read: {e}"))?;
        let n = u32::from_le_bytes(size) as usize;
        if !(7..=1 << 24).contains(&n) {
            return Err(format!("a reply of {n} bytes"));
        }
        let mut body = vec![0u8; n - 4];
        self.conn
            .read_exact(&mut body)
            .map_err(|e| format!("read: {e}"))?;
        if body[0] == RERROR {
            let len = usize::from(u16::from_le_bytes([body[3], body[4]]));
            let text = String::from_utf8_lossy(&body[5..(5 + len).min(body.len())]);
            return Err(format!("9P error: {text}"));
        }
        if body[0] != kind + 1 {
            return Err(format!("the reply has type {}, not {}", body[0], kind + 1));
        }
        Ok(body[3..].to_vec())
    }

    fn next_tag(&mut self) -> u16 {
        self.tag = (self.tag + 1) % NOTAG;
        self.tag
    }

    /// Agree on the version and attach fid 0 to the root.
    pub fn attach(&mut self, msize: u32) -> Result<(), String> {
        let mut m = Msg::new(TVERSION, NOTAG);
        m.u32(msize).str("9P2000");
        self.rpc(m, TVERSION)?;
        let tag = self.next_tag();
        let mut m = Msg::new(TATTACH, tag);
        m.u32(0).u32(NOFID).str("none").str("");
        self.rpc(m, TATTACH)?;
        Ok(())
    }

    /// One sequence: walk fid 0 to `path` as fid 1, open it, read up to `count` bytes, and clunk
    /// it. It gives the number of bytes read.
    pub fn sequence(&mut self, path: &[&str], count: u32) -> Result<usize, String> {
        let tag = self.next_tag();
        let mut m = Msg::new(TWALK, tag);
        m.u32(0).u32(1).u16(u16::try_from(path.len()).unwrap_or(0));
        for p in path {
            m.str(p);
        }
        let r = self.rpc(m, TWALK)?;
        if r.len() < 2 || usize::from(u16::from_le_bytes([r[0], r[1]])) != path.len() {
            return Err(format!("walk to {} failed", path.join("/")));
        }
        let tag = self.next_tag();
        let mut m = Msg::new(TOPEN, tag);
        m.u32(1);
        m.0.push(0);
        self.rpc(m, TOPEN)?;
        let tag = self.next_tag();
        let mut m = Msg::new(TREAD, tag);
        m.u32(1).u64(0).u32(count);
        let r = self.rpc(m, TREAD)?;
        let tag = self.next_tag();
        let mut m = Msg::new(TCLUNK, tag);
        m.u32(1);
        self.rpc(m, TCLUNK)?;
        Ok(r.len().saturating_sub(4))
    }
}

/// The times of the sequences, in microseconds, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Times(pub Vec<u64>);

impl Times {
    /// The value at a fraction of the sorted list.
    #[must_use]
    pub fn at(&self, f: f64) -> u64 {
        if self.0.is_empty() {
            return 0;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let i = ((self.0.len() as f64) * f) as usize;
        self.0[i.min(self.0.len() - 1)]
    }

    /// The mean.
    #[must_use]
    pub fn mean(&self) -> u64 {
        if self.0.is_empty() {
            return 0;
        }
        self.0.iter().sum::<u64>() / self.0.len() as u64
    }
}

/// Accept one guest on `port`, run `count` sequences on `file`, and give the report.
pub fn run(port: u16, count: usize, file: &str, wait: Duration) -> Result<String, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("port {port}: {e}"))?;
    eprintln!("r9 rtt: listening on port {port}");
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("listen: {e}"))?;
    let start = Instant::now();
    let conn = loop {
        match listener.accept() {
            Ok((c, _)) => break c,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > wait {
                    return Err("no guest came".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("accept: {e}")),
        }
    };
    conn.set_nonblocking(false).map_err(|e| e.to_string())?;
    conn.set_nodelay(true).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(conn.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("read: {e}"))?;
    if line.trim() != "9p rtt" {
        return Err(format!("the guest sent {line:?}, not 9p rtt"));
    }
    let mut client = Client::new(Joined { r: reader, w: conn });
    client.attach(8192 + 24)?;
    let path: Vec<&str> = file.split('/').filter(|p| !p.is_empty()).collect();
    let mut bytes = 0;
    for _ in 0..count.min(100) {
        bytes = client.sequence(&path, 8192)?;
    }
    let mut times = Vec::with_capacity(count);
    let all = Instant::now();
    for _ in 0..count {
        let t = Instant::now();
        client.sequence(&path, 8192)?;
        times.push(u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX));
    }
    let total = all.elapsed();
    times.sort_unstable();
    let t = Times(times);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{count} sequences of walk, open, read and clunk of {file} ({bytes} bytes) in {:.2}s",
        total.as_secs_f64()
    );
    let _ = writeln!(
        out,
        "sequence: median {} us, p90 {} us, p99 {} us, mean {} us",
        t.at(0.5),
        t.at(0.9),
        t.at(0.99),
        t.mean()
    );
    let _ = writeln!(
        out,
        "round trip: median {} us, mean {} us",
        t.at(0.5) / 4,
        t.mean() / 4
    );
    Ok(out)
}

/// A reader and a writer used as one connection.
struct Joined {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Read for Joined {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.r.read(buf)
    }
}

impl Write for Joined {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.w.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.w.flush()
    }
}

/// Run `r9 rtt`.
pub fn command(args: &crate::cli::Args) -> Result<String, String> {
    let port = args
        .get("port")
        .map_or(Ok(5641), str::parse)
        .map_err(|e| format!("--port: {e}"))?;
    let count = args
        .get("count")
        .map_or(Ok(10_000), str::parse)
        .map_err(|e| format!("--count: {e}"))?;
    let wait = args
        .get("wait")
        .map_or(Ok(1800), str::parse)
        .map_err(|e| format!("--wait: {e}"))?;
    run(
        port,
        count,
        args.get("file").unwrap_or("/sys/include/libc.h"),
        Duration::from_secs(wait),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_has_its_size_first() {
        let mut m = Msg::new(TVERSION, NOTAG);
        m.u32(8192).str("9P2000");
        let b = m.bytes();
        assert_eq!(b.len(), 19);
        assert_eq!(&b[..4], &19u32.to_le_bytes());
        assert_eq!(b[4], TVERSION);
        assert_eq!(&b[5..7], &[0xff, 0xff]);
        assert_eq!(&b[13..], b"9P2000");
    }

    /// A server that answers each request from a list.
    struct Canned {
        replies: std::io::Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }

    impl Read for Canned {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.replies.read(buf)
        }
    }

    impl Write for Canned {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.sent.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn an_error_reply_gives_its_text() {
        let mut m = Msg::new(RERROR, 1);
        m.str("file does not exist");
        let mut c = Client::new(Canned {
            replies: std::io::Cursor::new(m.bytes()),
            sent: Vec::new(),
        });
        let mut t = Msg::new(TCLUNK, 1);
        t.u32(1);
        assert_eq!(
            c.rpc(t, TCLUNK),
            Err("9P error: file does not exist".to_string())
        );
    }

    #[test]
    fn a_sequence_reads_the_file() {
        let mut replies = Vec::new();
        let mut w = Msg::new(TWALK + 1, 1);
        w.u16(2);
        w.0.extend_from_slice(&[0u8; 26]);
        replies.extend(w.bytes());
        let mut o = Msg::new(TOPEN + 1, 2);
        o.0.extend_from_slice(&[0u8; 17]);
        replies.extend(o.bytes());
        let mut r = Msg::new(TREAD + 1, 3);
        r.u32(5);
        r.0.extend_from_slice(b"hello");
        replies.extend(r.bytes());
        replies.extend(Msg::new(TCLUNK + 1, 4).bytes());
        let mut c = Client::new(Canned {
            replies: std::io::Cursor::new(replies),
            sent: Vec::new(),
        });
        assert_eq!(c.sequence(&["sys", "include"], 8192), Ok(5));
    }

    #[test]
    fn the_percentiles_come_from_the_sorted_times() {
        let t = Times((1..=100).collect());
        assert_eq!(t.at(0.5), 51);
        assert_eq!(t.at(0.9), 91);
        assert_eq!(t.mean(), 50);
    }
}
