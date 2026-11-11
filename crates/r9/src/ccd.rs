//! `r9 ccd`: the host side of the compiler slot (plan 05.4).
//!
//! `r9-cc` in the guest sends each call of `6c` or `7c` here and writes the reply. A call uses two
//! TCP connections from the guest, and the host needs no route into the guest:
//!
//! - The first line of the export connection is `9p ID`. After it, the guest runs `exportfs -r /`
//!   on the connection, so the rest is 9P from the name space of the build.
//! - The first line of the call connection is `call ID`, and the request follows it.
//!
//! For each call, `r9 ccd` opens a port on the loopback address that gives the export to one
//! client, and runs rucc with `--sysroot=9p:` on that port. rucc reads the sources and the headers
//! from the guest, and writes its objects to a new directory on the host. `r9 ccd` sends them back
//! in the reply, and `r9-cc` writes them in the guest. So the guest file system is the only place
//! where the build keeps its state.
//!
//! The request is lines of text, and then the bytes of standard input:
//!
//! ```text
//! cc 7c
//! objtype arm64
//! dir /sys/src/cmd/acme
//! argc 3
//! -FTVw
//! -I../common
//! acme.c
//! stdin 0
//! ```
//!
//! The reply is the status, standard error and each output file, and then `end`:
//!
//! ```text
//! status
//! stderr 0
//! file 1234 acme.7
//! <1234 bytes>
//! end
//! ```
//!
//! An empty status is success. The status `rucc failed` asks `r9-cc` to say what happened, and
//! the status of a call that could not run starts with `ccd:`.
//!
//! Before the reply, `r9 ccd` checks each `.6` file that rucc wrote with the instruction tables
//! of `6l` (plan 07.3). When `6l` cannot encode an instruction of the file, the status is
//! `objcheck failed`, standard error tells the instructions, and the directory of the call stays
//! on the host with the object in it.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The port that `r9 ccd` listens on when `--port` is not given.
pub const PORT: u16 = 5640;

/// How long a call waits for its export connection.
const EXPORT_WAIT: Duration = Duration::from_secs(30);

/// The longest line of a request.
const MAX_LINE: usize = 64 * 1024;

/// One call of the compiler, as `r9-cc` sends it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Call {
    /// The compiler name, such as `7c`.
    pub cc: String,
    /// The value of `$objtype` in the guest.
    pub objtype: String,
    /// The current directory in the guest.
    pub dir: String,
    /// The arguments after the compiler name.
    pub args: Vec<String>,
    /// The bytes of standard input. The compiler reads them when no source file is given.
    pub stdin: Vec<u8>,
}

/// What came of one call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reply {
    /// Empty for success, or else the reason of the failure.
    pub status: String,
    /// The standard error of rucc.
    pub stderr: Vec<u8>,
    /// Each output file: the name in the guest and the contents.
    pub files: Vec<(String, Vec<u8>)>,
}

/// The rucc command line for a call: the arguments, and the output files with their names in
/// the guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translated {
    /// The arguments of rucc, after the program name.
    pub args: Vec<String>,
    /// The value of `$INCLUDE` for rucc.
    pub include: String,
    /// The value of `$R9_OPT` for rucc, which adds the directory of the call to the end of the
    /// search for includes. kencc opens a name from the current directory when no include
    /// directory has it, and the current directory of rucc is not the one of the guest.
    pub options: String,
    /// For each output with `-o`, the name in the host directory and the name in the guest.
    pub renamed: Vec<(String, String)>,
}

/// The rucc target of an `objtype`.
#[must_use]
pub fn target(objtype: &str) -> Option<&'static str> {
    match objtype {
        "amd64" => Some("x86_64-plan9"),
        "arm64" => Some("aarch64-plan9"),
        "386" => Some("i386-plan9"),
        _ => None,
    }
}

/// The path `path` of the guest, made absolute from the directory `dir` and cleaned as
/// `cleanname` in the Plan 9 C library does it.
#[must_use]
pub fn absolute(dir: &str, path: &str) -> String {
    let full = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{dir}/{path}")
    };
    let mut names: Vec<&str> = Vec::new();
    for name in full.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop();
            }
            _ => names.push(name),
        }
    }
    format!("/{}", names.join("/"))
}

/// Make the rucc command line of `call`, for a 9P server of the guest at `root`, such as
/// `9p:tcp!127.0.0.1!40001`.
///
/// Each source file and each directory of `-I` becomes a path under `root`. An output of `-o`
/// is written in the directory of rucc under its base name, and goes back to the guest under
/// the name that the call gave.
pub fn translate(call: &Call, root: &str) -> Result<Translated, String> {
    let target = target(&call.objtype)
        .ok_or_else(|| format!("ccd: rucc does not compile for $objtype {}", call.objtype))?;
    let remote = |p: &str| format!("{root}{}", absolute(&call.dir, p));
    let mut args = vec![
        "--driver=kencc".to_owned(),
        format!("--target={target}"),
        format!("--sysroot={root}"),
    ];
    let mut renamed = Vec::new();
    let mut i = 0;
    while i < call.args.len() {
        let a = &call.args[i];
        i += 1;
        if a == "-I" || a == "-o" {
            let value = call
                .args
                .get(i)
                .ok_or_else(|| format!("ccd: no value after {a}"))?;
            i += 1;
            if a == "-I" {
                args.push(format!("-I{}", remote(value)));
            } else {
                let host = out_name(value);
                renamed.push((host.clone(), value.clone()));
                args.push("-o".to_owned());
                args.push(host);
            }
        } else if let Some(dir) = a.strip_prefix("-I") {
            args.push(format!("-I{}", remote(dir)));
        } else if let Some(out) = a.strip_prefix("-o") {
            let host = out_name(out);
            renamed.push((host.clone(), out.to_owned()));
            args.push(format!("-o{host}"));
        } else if a.starts_with('-') || a == "-" {
            args.push(a.clone());
        } else {
            args.push(remote(a));
        }
    }
    let include = format!(
        "{root}/{objtype}/include {root}/sys/include",
        objtype = call.objtype
    );
    let options = format!("-idirafter {root}{}", absolute("/", &call.dir));
    Ok(Translated {
        args,
        include,
        options,
        renamed,
    })
}

/// The options of `$R9_OPT` on the host, then `options`.
fn opt_with(options: &str) -> String {
    match std::env::var("R9_OPT") {
        Ok(host) if !host.trim().is_empty() => format!("{host} {options}"),
        _ => options.to_owned(),
    }
}

/// The name in the host directory of the output `name` of the guest.
fn out_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    if base.is_empty() || base == "." || base == ".." {
        "out".to_owned()
    } else {
        base.to_owned()
    }
}

/// Read one line with no newline, or `None` at the end of the stream.
fn line(r: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    r.by_ref()
        .take(MAX_LINE as u64)
        .read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.pop() != Some(b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a line is too long or not finished",
        ));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a line is not UTF-8"))
}

/// The value after `key ` on a line.
fn field(r: &mut impl BufRead, key: &str) -> io::Result<String> {
    let text = line(r)?.unwrap_or_default();
    text.strip_prefix(key)
        .and_then(|v| {
            v.strip_prefix(' ')
                .or(if v.is_empty() { Some("") } else { None })
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected `{key}`, found `{text}`"),
            )
        })
}

/// Read the request of a call, after its first line.
pub fn read_call(r: &mut impl BufRead) -> io::Result<Call> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let cc = field(r, "cc")?;
    let objtype = field(r, "objtype")?;
    let dir = field(r, "dir")?;
    let count: usize = field(r, "argc")?
        .parse()
        .map_err(|_| bad("argc is not a number"))?;
    let mut args = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        args.push(line(r)?.ok_or_else(|| bad("the arguments end early"))?);
    }
    let n: usize = field(r, "stdin")?
        .parse()
        .map_err(|_| bad("stdin is not a number"))?;
    let mut stdin = Vec::new();
    r.by_ref().take(n as u64).read_to_end(&mut stdin)?;
    if stdin.len() != n {
        return Err(bad("standard input ends early"));
    }
    Ok(Call {
        cc,
        objtype,
        dir,
        args,
        stdin,
    })
}

/// Write the request of a call, with its first line. `r9-cc` writes the same bytes.
pub fn write_call(w: &mut impl Write, id: &str, call: &Call) -> io::Result<()> {
    writeln!(w, "call {id}")?;
    writeln!(w, "cc {}", call.cc)?;
    writeln!(w, "objtype {}", call.objtype)?;
    writeln!(w, "dir {}", call.dir)?;
    writeln!(w, "argc {}", call.args.len())?;
    for a in &call.args {
        writeln!(w, "{a}")?;
    }
    writeln!(w, "stdin {}", call.stdin.len())?;
    w.write_all(&call.stdin)
}

/// Write a reply.
pub fn write_reply(w: &mut impl Write, reply: &Reply) -> io::Result<()> {
    writeln!(w, "status {}", reply.status)?;
    writeln!(w, "stderr {}", reply.stderr.len())?;
    w.write_all(&reply.stderr)?;
    for (name, bytes) in &reply.files {
        writeln!(w, "file {} {name}", bytes.len())?;
        w.write_all(bytes)?;
    }
    writeln!(w, "end")
}

/// Read a reply, as `r9-cc` reads it.
pub fn read_reply(r: &mut impl BufRead) -> io::Result<Reply> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let status = field(r, "status")?;
    let n: usize = field(r, "stderr")?
        .parse()
        .map_err(|_| bad("stderr is not a number"))?;
    let mut stderr = Vec::new();
    r.by_ref().take(n as u64).read_to_end(&mut stderr)?;
    let mut files = Vec::new();
    loop {
        let text = line(r)?.ok_or_else(|| bad("the reply ends early"))?;
        if text == "end" {
            break;
        }
        let rest = text
            .strip_prefix("file ")
            .ok_or_else(|| bad("expected `file` or `end`"))?;
        let (n, name) = rest.split_once(' ').ok_or_else(|| bad("no file name"))?;
        let n: usize = n.parse().map_err(|_| bad("the size is not a number"))?;
        let mut bytes = Vec::new();
        r.by_ref().take(n as u64).read_to_end(&mut bytes)?;
        files.push((name.to_owned(), bytes));
    }
    Ok(Reply {
        status,
        stderr,
        files,
    })
}

/// The export connections that wait for their calls, by the id of the call.
#[derive(Default)]
struct Exports {
    waiting: Mutex<HashMap<String, TcpStream>>,
    arrived: Condvar,
}

impl Exports {
    fn put(&self, id: String, stream: TcpStream) {
        let mut map = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        map.insert(id, stream);
        self.arrived.notify_all();
    }

    fn take(&self, id: &str, limit: Duration) -> Option<TcpStream> {
        let end = Instant::now() + limit;
        let mut map = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(s) = map.remove(id) {
                return Some(s);
            }
            let now = Instant::now();
            if now >= end {
                return None;
            }
            map = self
                .arrived
                .wait_timeout(map, end - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// How `r9 ccd` runs.
#[derive(Debug, Clone)]
pub struct Config {
    /// The port on the loopback address.
    pub port: u16,
    /// The rucc program.
    pub rucc: PathBuf,
    /// A directory for the outputs of each call.
    pub work: PathBuf,
    /// The file that gets the line of each call. With none, the lines go to standard output.
    pub log: Option<Arc<Mutex<std::fs::File>>>,
}

/// Copy bytes from `from` to `to` until the end of `from`, then close the write side of `to`.
fn pump(mut from: TcpStream, mut to: TcpStream) {
    let _ = io::copy(&mut from, &mut to);
    let _ = to.shutdown(Shutdown::Write);
}

/// Give the export `guest` to one client on a new loopback port. Return the port.
fn relay(guest: TcpStream) -> io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        let Ok((client, _)) = listener.accept() else {
            return;
        };
        let _ = client.set_nodelay(true);
        let _ = guest.set_nodelay(true);
        let (Ok(c2), Ok(g2)) = (client.try_clone(), guest.try_clone()) else {
            return;
        };
        let up = thread::spawn(move || pump(c2, g2));
        pump(guest, client);
        let _ = up.join();
    });
    Ok(port)
}

/// Run rucc for one call and make the reply.
fn compile(cfg: &Config, exports: &Exports, id: &str, call: &Call) -> Reply {
    let fail = |status: String| Reply {
        status,
        ..Reply::default()
    };
    let Some(guest) = exports.take(id, EXPORT_WAIT) else {
        return fail(format!("ccd: no export for call {id}"));
    };
    let shut = guest.try_clone().ok();
    let port = match relay(guest) {
        Ok(p) => p,
        Err(e) => return fail(format!("ccd: {e}")),
    };
    let root = format!("9p:tcp!127.0.0.1!{port}");
    let t = match translate(call, &root) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let dir = cfg.work.join(id.replace(['/', '.'], "_"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(format!("ccd: {}: {e}", dir.display()));
    }
    let run = Command::new(&cfg.rucc)
        .args(&t.args)
        .current_dir(&dir)
        .env("objtype", &call.objtype)
        .env("INCLUDE", &t.include)
        .env("R9_OPT", opt_with(&t.options))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut input) = child.stdin.take() {
                let bytes = call.stdin.clone();
                thread::spawn(move || {
                    let _ = input.write_all(&bytes);
                });
            }
            child.wait_with_output()
        });
    if let Some(s) = shut {
        let _ = s.shutdown(Shutdown::Both);
    }
    let reply = match run {
        Err(e) => fail(format!("ccd: {}: {e}", cfg.rucc.display())),
        Ok(out) => {
            let mut reply = Reply {
                status: if out.status.success() {
                    String::new()
                } else {
                    "rucc failed".to_owned()
                },
                stderr: out.stderr,
                files: Vec::new(),
            };
            if out.status.success() {
                match outputs(&dir, &t.renamed) {
                    Ok(files) => match check_objects(&files) {
                        None => reply.files = files,
                        Some(report) => {
                            "objcheck failed".clone_into(&mut reply.status);
                            reply.stderr.extend_from_slice(report.as_bytes());
                        }
                    },
                    Err(e) => reply.status = format!("ccd: {e}"),
                }
            }
            reply
        }
    };
    // A failed call keeps its directory, with what it needs to run again: the directory of the
    // guest, the arguments, and the standard input.
    if reply.status.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
    } else {
        let what = format!("{}\n{}\n", call.dir, call.args.join(" "));
        let _ = std::fs::write(dir.join("call"), what);
        if !call.stdin.is_empty() {
            let _ = std::fs::write(dir.join("stdin"), &call.stdin);
        }
    }
    reply
}

/// Check each `.6` file of a reply with the instruction tables of `6l`. Return what `6l` cannot
/// encode, or `None` when each instruction has an encoding.
fn check_objects(files: &[(String, Vec<u8>)]) -> Option<String> {
    static CHECKER: OnceLock<crate::objcheck::Checker> = OnceLock::new();
    let checker = CHECKER.get_or_init(crate::objcheck::Checker::new);
    let mut report = String::new();
    for (name, bytes) in files.iter().filter(|(n, _)| n.ends_with(".6")) {
        match crate::objcheck::file(checker, name, bytes) {
            Ok((_, 0)) => {}
            Ok((text, _)) => report.push_str(&text),
            Err(e) => {
                report.push_str(&e);
                report.push('\n');
            }
        }
    }
    (!report.is_empty()).then_some(report)
}

/// The files that rucc wrote in `dir`, with their names in the guest.
fn outputs(dir: &Path, renamed: &[(String, String)]) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    for name in names {
        let bytes = std::fs::read(dir.join(&name))?;
        let guest = renamed
            .iter()
            .find(|(h, _)| *h == name)
            .map_or_else(|| name.clone(), |(_, g)| g.clone());
        files.push((guest, bytes));
    }
    Ok(files)
}

/// Serve one connection from the guest.
fn serve(cfg: &Config, exports: &Exports, stream: TcpStream) -> io::Result<()> {
    let mut r = BufReader::new(stream.try_clone()?);
    let first = line(&mut r)?.unwrap_or_default();
    if let Some(id) = first.strip_prefix("9p ") {
        // exportfs reads the stream from here, so no byte may wait in the buffer.
        if !r.buffer().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "9P before the call",
            ));
        }
        exports.put(id.to_owned(), stream);
        return Ok(());
    }
    let id = first
        .strip_prefix("call ")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("`{first}`")))?
        .to_owned();
    let call = read_call(&mut r)?;
    let start = Instant::now();
    let reply = compile(cfg, exports, &id, &call);
    let files: Vec<&str> = call
        .args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(String::as_str)
        .collect();
    let text = format!(
        "{} {} {} {} {}ms {}",
        if reply.status.is_empty() {
            "ok"
        } else {
            "FAIL"
        },
        call.cc,
        call.dir,
        files.join(" "),
        start.elapsed().as_millis(),
        reply.status
    );
    match &cfg.log {
        Some(log) => {
            let mut f = log.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = writeln!(f, "{text}");
        }
        None => println!("{text}"),
    }
    let mut w = io::BufWriter::new(stream);
    write_reply(&mut w, &reply)?;
    w.flush()
}

/// Run the daemon until it is killed.
pub fn run(cfg: &Config) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", cfg.port))
        .map_err(|e| format!("ccd: port {}: {e}", cfg.port))?;
    std::fs::create_dir_all(&cfg.work).map_err(|e| format!("ccd: {}: {e}", cfg.work.display()))?;
    eprintln!(
        "r9 ccd: on 127.0.0.1:{} with {}; the guest dials tcp!10.0.2.2!{}",
        cfg.port,
        cfg.rucc.display(),
        cfg.port
    );
    accept(&listener, cfg);
    Ok(())
}

/// Start `r9 ccd` on a free port of the loopback address, in a thread that runs until the
/// process exits. `cfg.port` is not used. Return the port.
pub fn start(cfg: &Config) -> Result<u16, String> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("ccd: a free port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("ccd: {e}"))?
        .port();
    std::fs::create_dir_all(&cfg.work).map_err(|e| format!("ccd: {}: {e}", cfg.work.display()))?;
    let cfg = cfg.clone();
    thread::spawn(move || accept(&listener, &cfg));
    Ok(port)
}

/// Serve each connection to `listener` in its own thread.
fn accept(listener: &TcpListener, cfg: &Config) {
    let cfg = Arc::new(cfg.clone());
    let exports = Arc::new(Exports::default());
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = stream.set_nodelay(true);
        let (cfg, exports) = (Arc::clone(&cfg), Arc::clone(&exports));
        thread::spawn(move || {
            if let Err(e) = serve(&cfg, &exports, stream) {
                eprintln!("r9 ccd: {e}");
            }
        });
    }
}

/// Run `r9 ccd` with the options of `args`.
pub fn command(args: &crate::cli::Args) -> Result<(), String> {
    let port = args
        .get("port")
        .map_or(Ok(PORT), str::parse)
        .map_err(|e| format!("--port: {e}"))?;
    let rucc = args.get("rucc").map_or_else(
        || std::env::var_os("R9_RUCC").map_or_else(|| "rucc".into(), Into::into),
        Into::into,
    );
    let work = args.get("work").map_or_else(
        || std::env::temp_dir().join(format!("r9-ccd-{}", std::process::id())),
        Into::into,
    );
    run(&Config {
        port,
        rucc,
        work,
        log: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: &[&str]) -> Call {
        Call {
            cc: "7c".into(),
            objtype: "arm64".into(),
            dir: "/sys/src/cmd/acme".into(),
            args: args.iter().map(|&a| a.to_owned()).collect(),
            stdin: Vec::new(),
        }
    }

    #[test]
    fn paths_are_cleaned_as_plan9_does() {
        assert_eq!(
            absolute("/sys/src/cmd/acme", "../common"),
            "/sys/src/cmd/common"
        );
        assert_eq!(absolute("/sys/src/cmd/acme", "."), "/sys/src/cmd/acme");
        assert_eq!(absolute("/a", "/sys/include/./x.h"), "/sys/include/x.h");
        assert_eq!(absolute("/", "../.."), "/");
    }

    #[test]
    fn sources_and_include_directories_go_to_the_server() {
        let root = "9p:tcp!127.0.0.1!4000";
        let t = translate(
            &call(&["-FTVw", "-I../common", "-I", "/sys/include/ape", "acme.c"]),
            root,
        )
        .unwrap();
        assert_eq!(
            t.args,
            [
                "--driver=kencc",
                "--target=aarch64-plan9",
                "--sysroot=9p:tcp!127.0.0.1!4000",
                "-FTVw",
                "-I9p:tcp!127.0.0.1!4000/sys/src/cmd/common",
                "-I9p:tcp!127.0.0.1!4000/sys/include/ape",
                "9p:tcp!127.0.0.1!4000/sys/src/cmd/acme/acme.c",
            ]
        );
        assert_eq!(
            t.include,
            "9p:tcp!127.0.0.1!4000/arm64/include 9p:tcp!127.0.0.1!4000/sys/include"
        );
        assert_eq!(
            t.options,
            "-idirafter 9p:tcp!127.0.0.1!4000/sys/src/cmd/acme"
        );
        assert!(t.renamed.is_empty());
    }

    #[test]
    fn an_output_goes_back_under_its_own_name() {
        let t = translate(&call(&["-o", "../x/y.7", "-oz.7", "a.c"]), "9p:a").unwrap();
        assert_eq!(
            t.renamed,
            [
                ("y.7".to_owned(), "../x/y.7".to_owned()),
                ("z.7".to_owned(), "z.7".to_owned())
            ]
        );
        assert!(translate(&call(&["-o"]), "9p:a").is_err());
        let mut c = call(&["a.c"]);
        c.objtype = "mips".into();
        assert!(translate(&c, "9p:a").is_err());
    }

    #[test]
    fn a_call_and_a_reply_read_back() {
        let mut c = call(&["-w", "a b.c"]);
        c.stdin = b"int x;\n".to_vec();
        let mut bytes = Vec::new();
        write_call(&mut bytes, "12.34", &c).unwrap();
        let mut r = io::Cursor::new(bytes);
        assert_eq!(line(&mut r).unwrap().unwrap(), "call 12.34");
        assert_eq!(read_call(&mut r).unwrap(), c);

        let reply = Reply {
            status: String::new(),
            stderr: b"warning\n".to_vec(),
            files: vec![("a.7".into(), vec![0, 1, 2, b'\n'])],
        };
        let mut bytes = Vec::new();
        write_reply(&mut bytes, &reply).unwrap();
        assert!(bytes.starts_with(b"status \nstderr 8\nwarning\nfile 4 a.7\n"));
        assert_eq!(read_reply(&mut io::Cursor::new(bytes)).unwrap(), reply);
    }

    #[test]
    fn an_export_waits_for_its_call() {
        let exports = Arc::new(Exports::default());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let other = Arc::clone(&exports);
        let t = thread::spawn(move || other.take("7", Duration::from_secs(5)).is_some());
        let _client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        exports.put("7".into(), server);
        assert!(t.join().unwrap());
        assert!(exports.take("8", Duration::from_millis(10)).is_none());
    }

    #[test]
    fn objcheck_passes_a_good_object_and_names_a_bad_one() {
        use crate::obj::tests::amd64_object_to;
        let good = vec![("a.6".to_string(), amd64_object_to(16))];
        assert_eq!(check_objects(&good), None);
        let bad = vec![
            ("x.7".to_string(), b"not an amd64 object".to_vec()),
            ("b.6".to_string(), amd64_object_to(crate::obj::d6::CONST)),
        ];
        let report = check_objects(&bad).unwrap();
        assert!(report.contains("b.6: main: MOVQ $1,$0"), "{report}");
        assert!(!report.contains("x.7"));
        let broken = vec![("c.6".to_string(), b"xx".to_vec())];
        assert!(check_objects(&broken).unwrap().starts_with("c.6: "));
    }
}
