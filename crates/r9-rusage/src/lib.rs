//! Run a child process and give its CPU time and its peak resident memory.
//!
//! The standard library waits for a child with `waitpid`, which gives the exit status and
//! nothing more. Plan 13.5 needs the user and system CPU time of each compiler process, and its
//! peak resident memory, so this crate waits with `wait4`, which also fills a `struct rusage`.
//! The CPU time of the child is then its own time, and not the time of other children that run
//! at the same time.
//!
//! The crate works on Linux and macOS with 64-bit pointers. On these hosts, `struct rusage` is
//! two `struct timeval` and 14 `long` fields. A `struct timeval` is 16 bytes on both, but the
//! microseconds are an `int` with 4 bytes of padding on macOS. `ru_maxrss` is in KiB on Linux
//! and in bytes on macOS.

use std::io::{self, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// What the kernel says about one child that ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// How the child ended.
    pub status: ExitStatus,
    /// The CPU time in user mode.
    pub user: Duration,
    /// The CPU time in the kernel.
    pub sys: Duration,
    /// The peak resident memory, in KiB.
    pub max_rss_kib: u64,
}

impl Usage {
    /// The user and the system CPU time together.
    #[must_use]
    pub fn cpu(&self) -> Duration {
        self.user + self.sys
    }
}

/// The usage of a child and what it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The usage from `wait4`.
    pub usage: Usage,
    /// The time from the call of [`wait`] until the child ended.
    pub wall: Duration,
    /// The standard output, when it was a pipe.
    pub stdout: Vec<u8>,
    /// The standard error, when it was a pipe.
    pub stderr: Vec<u8>,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct Timeval {
    sec: i64,
    usec: i32,
    pad: i32,
}

#[cfg(not(target_os = "macos"))]
#[repr(C)]
#[derive(Default)]
struct Timeval {
    sec: i64,
    usec: i64,
}

impl Timeval {
    fn duration(&self) -> Duration {
        let sec = u64::try_from(self.sec).unwrap_or(0);
        let usec = u32::try_from(self.usec).unwrap_or(0);
        Duration::from_secs(sec) + Duration::from_micros(u64::from(usec))
    }
}

#[repr(C)]
#[derive(Default)]
struct Rusage {
    utime: Timeval,
    stime: Timeval,
    maxrss: i64,
    rest: [i64; 13],
}

unsafe extern "C" {
    fn wait4(pid: i32, status: *mut i32, options: i32, rusage: *mut Rusage) -> i32;
}

const _: () = assert!(std::mem::size_of::<Rusage>() == 144);

/// Read a pipe to its end in a new thread.
fn drain(pipe: Option<impl Read + Send + 'static>) -> Option<thread::JoinHandle<Vec<u8>>> {
    pipe.map(|mut p| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = p.read_to_end(&mut bytes);
            bytes
        })
    })
}

/// Wait for a child that was spawned, with its standard output and standard error read to the
/// end when they are pipes. The child is reaped here, so the caller must not wait for it again.
pub fn wait(mut child: Child) -> io::Result<Output> {
    let start = Instant::now();
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let pid = i32::try_from(child.id()).map_err(io::Error::other)?;
    let mut status = 0;
    let mut ru = Rusage::default();
    loop {
        // SAFETY: `status` and `ru` are valid for writes, and `Rusage` has the layout of
        // `struct rusage` on the hosts that this crate supports. The pid is the child of this
        // process, and the standard library has not reaped it.
        let r = unsafe { wait4(pid, &raw mut status, 0, &raw mut ru) };
        if r == pid {
            break;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    let wall = start.elapsed();
    let maxrss = u64::try_from(ru.maxrss).unwrap_or(0);
    let max_rss_kib = if cfg!(target_os = "macos") {
        maxrss / 1024
    } else {
        maxrss
    };
    Ok(Output {
        usage: Usage {
            status: ExitStatus::from_raw(status),
            user: ru.utime.duration(),
            sys: ru.stime.duration(),
            max_rss_kib,
        },
        wall,
        stdout: out.and_then(|h| h.join().ok()).unwrap_or_default(),
        stderr: err.and_then(|h| h.join().ok()).unwrap_or_default(),
    })
}

/// Run a command with no standard input, read its standard output and standard error, and give
/// its usage.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    wait(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_child_has_cpu_time() {
        let out = output(Command::new("sh").args([
            "-c",
            "i=0; while [ $i -lt 200000 ]; do i=$((i+1)); done; echo done; echo err >&2; exit 3",
        ]))
        .unwrap();
        assert_eq!(out.usage.status.code(), Some(3));
        assert_eq!(out.stdout, b"done\n");
        assert_eq!(out.stderr, b"err\n");
        assert!(
            out.usage.cpu() > Duration::from_millis(10),
            "{:?}",
            out.usage
        );
        assert!(out.usage.cpu() <= out.wall + Duration::from_millis(50));
        assert!(out.usage.max_rss_kib > 100, "{:?}", out.usage);
    }

    #[test]
    fn a_child_that_sleeps_has_little_cpu_time() {
        let out = output(Command::new("sleep").arg("0.3")).unwrap();
        assert!(out.usage.status.success());
        assert!(out.wall >= Duration::from_millis(250));
        assert!(
            out.usage.cpu() < Duration::from_millis(100),
            "{:?}",
            out.usage
        );
    }
}
