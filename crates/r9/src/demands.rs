//! `r9 demands`: what rucc must learn first, ranked over the units of a G build (plan 05.6).
//!
//! Each unit that kencc compiled in a G build is compiled again with rucc on the host, with
//! `-fsyntax-only`, against a copy of the guest's tree and headers. The first error of each
//! unit is kept. The errors are grouped by their code and their text, with the names in quotes
//! taken out, and ranked by the number of units they stop.
//!
//! There are two modes. In the syntax mode, the kencc arguments are translated: `-I` and `-D`
//! pass through, the kencc default include directories are added, and the other flags are
//! dropped. The target is x86-64 or aarch64 Linux. The command is in [`rucc_command`]. This
//! mode was made before rucc had the kencc driver (tamnd/rucc#4147), and it shows what the
//! front end lacks with no help from the driver.
//!
//! In the kencc mode, which is `--kencc`, each unit is compiled with the kencc driver of rucc
//! and the Plan 9 target, with the arguments that kencc got, as `r9 ccd` compiles it. The tree
//! under the root is the sysroot. The objects go to a scratch directory and are then removed.
//! The command is in [`kencc_command`].

use crate::census::Record;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What became of one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Result1 {
    /// rucc found no error.
    Clean,
    /// The first error: the code, the text with names taken out, and the place.
    Error {
        /// The code, such as `E0547`, or empty.
        code: String,
        /// The text with each name in quotes replaced by `_`.
        message: String,
        /// `file:line:col` in the guest's tree.
        at: String,
        /// The names in single quotes, joined by spaces.
        names: String,
    },
    /// rucc stopped with no error line, or did not run.
    Crash(String),
    /// The unit was not replayed: its source was on standard input, the build made its source,
    /// or it is not for the guest's architecture.
    Skipped,
}

/// One ranked demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Demand {
    /// 1 for the demand that stops the most units.
    pub rank: usize,
    /// The diagnostic code.
    pub code: String,
    /// The text with names taken out.
    pub message: String,
    /// The number of units it stops.
    pub units: usize,
    /// Up to three places where it is the first error.
    pub examples: Vec<String>,
    /// The names that were taken out of the text, most units first, as `name (units)`. Up to
    /// five are kept.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

/// The whole result, as written to `demands.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Demands {
    /// The `objtype` of the build.
    pub objtype: String,
    /// The rucc command that was used, without the unit.
    pub rucc: String,
    /// The number of units replayed.
    pub replayed: usize,
    /// The units with no error.
    pub clean: usize,
    /// The units where rucc stopped with no error line.
    pub crashed: usize,
    /// The units not replayed.
    pub skipped: usize,
    /// The demands, most units first.
    pub demand: Vec<Demand>,
}

/// The rucc target for an `objtype`.
#[must_use]
pub fn target(objtype: &str) -> &'static str {
    match objtype {
        "arm64" => "aarch64-unknown-linux-gnu",
        _ => "x86_64-unknown-linux-gnu",
    }
}

/// The flags that every replay gets after the rucc program and its extra flags. Two of them
/// stand in for the kencc driver of rucc (tamnd/rucc#4147), so that the ranking shows what the
/// compiler lacks and not what the driver must set:
///
/// - kencc is older than C23, so `true` and `false` are names. `-std=gnu11` gives this.
/// - kencc looks for a quoted include in the directory of the source first, or in `.` with
///   `-.`. When no directory has the file, of either form, kencc opens the name as it is, in
///   the current directory (`macinc` in `cc/macbody`). [`rucc_command`] gives this with
///   `-iquote` and `-idirafter .`.
#[must_use]
pub fn base_flags(objtype: &str) -> Vec<String> {
    vec![
        format!("--target={}", target(objtype)),
        "-fsyntax-only".into(),
        "-nostdinc".into(),
        "-w".into(),
        "-std=gnu11".into(),
    ]
}

/// The rucc command for one unit, to run in the unit's directory under `root`. `root` holds
/// the guest's tree: `sys/src`, `sys/include` and `<objtype>/include`. `rucc` is the program
/// and the extra flags.
#[must_use]
pub fn rucc_command(rucc: &[String], root: &Path, r: &Record) -> Vec<String> {
    let mut c = rucc.to_vec();
    c.extend(base_flags(&r.objtype));
    let host = |p: &str| {
        if p.starts_with('/') {
            root.join(p.trim_start_matches('/')).display().to_string()
        } else {
            p.to_string()
        }
    };
    let source_dir = r
        .inputs
        .first()
        .and_then(|s| Path::new(&s.file).parent())
        .map(|d| d.display().to_string())
        .filter(|d| !d.is_empty() && !r.argv.iter().any(|a| a == "-."))
        .unwrap_or_else(|| ".".into());
    c.push("-iquote".into());
    c.push(host(&source_dir));
    let mut args = r.argv.iter();
    while let Some(a) = args.next() {
        if let Some(rest) = a.strip_prefix("-I") {
            let dir = if rest.is_empty() {
                args.next().map_or("", String::as_str)
            } else {
                rest
            };
            c.push(format!("-I{}", host(dir)));
        } else if let Some(rest) = a.strip_prefix("-D") {
            let def = if rest.is_empty() {
                args.next().map_or("", String::as_str)
            } else {
                rest
            };
            c.push(format!("-D{def}"));
        } else if a == "-o" {
            args.next();
        } else if a.starts_with('-') {
            // -FTVw, -p, -., -a and the rest change nothing that -fsyntax-only sees.
        } else {
            // A source can have an absolute path, such as /sys/src/cmd/ndb/convM2DNS.c.
            c.push(host(a));
        }
    }
    // kencc searches these after the -I directories.
    c.push(format!("-I{}", host(&format!("/{}/include", r.objtype))));
    c.push(format!("-I{}", host("/sys/include")));
    c.push("-idirafter".into());
    c.push(".".into());
    c
}

/// Read the first error from rucc's standard error. `dir` is the unit's directory in the guest,
/// and a place under `root` on the host is given as a place in the guest.
#[must_use]
pub fn first_error(stderr: &str, dir: &str, root: &Path) -> Option<Result1> {
    let line = stderr.lines().find(|l| l.contains(": error: "))?;
    let (place, rest) = line.split_once(": error: ")?;
    let (text, code) = match rest.rfind(" [E") {
        Some(i) if rest.ends_with(']') => (&rest[..i], rest[i + 2..rest.len() - 1].to_string()),
        _ => (rest, String::new()),
    };
    let root = root.display().to_string();
    let at = if let Some(guest) = place.strip_prefix(root.trim_end_matches('/')) {
        guest.to_string()
    } else if place.starts_with('/') {
        place.to_string()
    } else {
        format!("{dir}/{place}")
    };
    Some(Result1::Error {
        code,
        message: without_names(text),
        at,
        names: quoted_names(text).join(" "),
    })
}

/// The names in single quotes, in order.
#[must_use]
pub fn quoted_names(text: &str) -> Vec<String> {
    text.split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// Replace each name in single quotes with `_`, so that errors of one kind group together.
#[must_use]
pub fn without_names(text: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    for ch in text.chars() {
        if ch == '\'' {
            if !quoted {
                out.push_str("'_'");
            }
            quoted = !quoted;
        } else if !quoted {
            out.push(ch);
        }
    }
    out
}

/// How a unit is replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `-fsyntax-only` with the Linux target, and the kencc arguments translated.
    Syntax,
    /// The kencc driver of rucc and the Plan 9 target, as `r9 ccd` runs it.
    Kencc,
}

/// The environment variables of a rucc command, as names and values.
pub type Env = Vec<(String, String)>;

/// The rucc command and its environment for one unit in the kencc mode, as `r9 ccd` makes
/// them. The tree under `root` is the sysroot.
///
/// # Errors
///
/// The `$objtype` of the unit has no rucc target, or the arguments are not complete.
pub fn kencc_command(
    rucc: &[String],
    root: &Path,
    r: &Record,
) -> Result<(Vec<String>, Env), String> {
    let call = crate::ccd::Call {
        cc: r.cc.clone(),
        objtype: r.objtype.clone(),
        dir: r.dir.clone(),
        args: r.argv.clone(),
        stdin: Vec::new(),
    };
    let t = crate::ccd::translate(&call, &root.display().to_string())?;
    let mut c = rucc.to_vec();
    c.extend(t.args);
    let env = vec![
        ("objtype".to_owned(), r.objtype.clone()),
        ("INCLUDE".to_owned(), t.include),
        ("R9_OPT".to_owned(), t.options),
    ];
    Ok((c, env))
}

/// The number of scratch directories made, so that each has its own name.
static SCRATCH: AtomicUsize = AtomicUsize::new(0);

/// Replay one unit.
#[must_use]
pub fn replay(rucc: &[String], root: &Path, r: &Record, mode: Mode) -> Result1 {
    let host_letter = match r.objtype.as_str() {
        "amd64" => "6c",
        "arm64" => "7c",
        _ => "",
    };
    if r.stdin || r.inputs.is_empty() || r.cc != host_letter {
        return Result1::Skipped;
    }
    // A source that the build made, such as a y.tab.c of yacc, is not in a copy of the tree.
    let generated = r.inputs.iter().any(|f| {
        let path = crate::ccd::absolute(&r.dir, &f.file);
        !root.join(path.trim_start_matches('/')).exists()
    });
    if generated {
        return Result1::Skipped;
    }
    let (c, env, dir, scratch) = match mode {
        Mode::Syntax => {
            let dir = root.join(r.dir.trim_start_matches('/'));
            (rucc_command(rucc, root, r), Vec::new(), dir, false)
        }
        Mode::Kencc => {
            let (c, env) = match kencc_command(rucc, root, r) {
                Ok(command) => command,
                Err(e) => return Result1::Crash(e),
            };
            let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("r9-demands-{}-{n}", std::process::id()));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Result1::Crash(format!("cannot make {}: {e}", dir.display()));
            }
            (c, env, dir, true)
        }
    };
    let ran = Command::new(&c[0])
        .args(&c[1..])
        .envs(env)
        .current_dir(&dir)
        .output();
    if scratch {
        let _ = std::fs::remove_dir_all(&dir);
    }
    match ran {
        Err(e) => Result1::Crash(format!("cannot run {} in {}: {e}", c[0], dir.display())),
        Ok(out) if out.status.success() => Result1::Clean,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            first_error(&stderr, &r.dir, root).unwrap_or_else(|| {
                Result1::Crash(
                    stderr
                        .lines()
                        .next()
                        .unwrap_or("no message")
                        .chars()
                        .take(200)
                        .collect(),
                )
            })
        }
    }
}

/// Replay every unit, on all host CPUs, and rank the first errors.
#[must_use]
pub fn run(rucc: &[String], root: &Path, records: &[Record], mode: Mode) -> Demands {
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let chunk = records.len().div_ceil(threads).max(1);
    let results: Vec<Result1> = std::thread::scope(|s| {
        let handles: Vec<_> = records
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|r| replay(rucc, root, r, mode))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let objtype = records.first().map_or("", |r| r.objtype.as_str());
    let mut command = rucc.to_vec();
    match mode {
        Mode::Syntax => command.extend(base_flags(objtype)),
        Mode::Kencc => {
            command.push("--driver=kencc".to_owned());
            let target = crate::ccd::target(objtype).unwrap_or("none");
            command.push(format!("--target={target}"));
        }
    }
    rank(objtype, &command.join(" "), &results)
}

/// Rank the results.
#[must_use]
pub fn rank(objtype: &str, rucc: &str, results: &[Result1]) -> Demands {
    let mut groups: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
    let (mut clean, mut crashed, mut skipped) = (0, 0, 0);
    for r in results {
        match r {
            Result1::Clean => clean += 1,
            Result1::Skipped => skipped += 1,
            Result1::Crash(m) => {
                crashed += 1;
                groups
                    .entry(("crash".into(), without_names(m)))
                    .or_default()
                    .push((String::new(), String::new()));
            }
            Result1::Error {
                code,
                message,
                at,
                names,
            } => groups
                .entry((code.clone(), message.clone()))
                .or_default()
                .push((at.clone(), names.clone())),
        }
    }
    let mut demand: Vec<Demand> = groups
        .into_iter()
        .map(|((code, message), units)| {
            let names = common(units.iter().map(|(_, n)| n.as_str()), 5);
            Demand {
                rank: 0,
                code,
                message,
                units: units.len(),
                examples: distinct(units.into_iter().map(|(at, _)| at).collect(), 3),
                names,
            }
        })
        .collect();
    demand.sort_by(|a, b| {
        b.units
            .cmp(&a.units)
            .then(a.code.cmp(&b.code))
            .then(a.message.cmp(&b.message))
    });
    for (i, d) in demand.iter_mut().enumerate() {
        d.rank = i + 1;
    }
    Demands {
        objtype: objtype.to_string(),
        rucc: rucc.to_string(),
        replayed: results.len() - skipped,
        clean,
        crashed,
        skipped,
        demand,
    }
}

/// The `n` most common texts that are not empty, as `text (count)`.
fn common<'a>(texts: impl Iterator<Item = &'a str>, n: usize) -> Vec<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for t in texts.filter(|t| !t.is_empty()) {
        *counts.entry(t).or_default() += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    counts
        .into_iter()
        .take(n)
        .map(|(t, c)| format!("{t} ({c})"))
        .collect()
}

/// The first `n` distinct places that are not empty.
fn distinct(places: Vec<String>, n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in places {
        if out.len() == n {
            break;
        }
        if !p.is_empty() && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

impl Demands {
    /// The report `r9 demands` prints: the counts and the first `top` demands.
    #[must_use]
    pub fn report(&self, top: usize) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} units replayed, {} clean, {} crashed, {} skipped; {}",
            self.replayed, self.clean, self.crashed, self.skipped, self.rucc
        );
        for d in self.demand.iter().take(top) {
            let _ = writeln!(
                out,
                "{:>3} {:>5} {:<6} {}  ({})",
                d.rank,
                d.units,
                d.code,
                d.message,
                d.examples.first().map_or("", String::as_str)
            );
            if !d.names.is_empty() {
                let _ = writeln!(out, "              names: {}", d.names.join(", "));
            }
        }
        out
    }

    /// Write `demands.toml`.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(
            path,
            format!("# Written by r9 demands. Do not edit.\n{text}"),
        )
        .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The rucc program, `R9_RUCC` or `rucc` on the path, followed by the extra flags in
/// `flags`, which are separated by spaces.
#[must_use]
pub fn find_rucc(flags: Option<&str>) -> Vec<String> {
    let program = std::env::var_os("R9_RUCC").map_or_else(|| PathBuf::from("rucc"), PathBuf::from);
    let mut c = vec![program.display().to_string()];
    c.extend(flags.unwrap_or("").split_whitespace().map(String::from));
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(argv: &[&str]) -> Record {
        Record {
            cc: "6c".into(),
            objtype: "amd64".into(),
            dir: "/sys/src/cmd/acme".into(),
            argv: argv.iter().map(ToString::to_string).collect(),
            inputs: vec![crate::census::File {
                file: "acme.c".into(),
                sha256: "aa".into(),
            }],
            outputs: Vec::new(),
            stdin: false,
            status: String::new(),
            ms: 1,
            user_ms: 0,
            sys_ms: 0,
            origin: "ref-c".into(),
        }
    }

    #[test]
    fn the_kencc_mode_compiles_as_ccd_does() {
        let r = record(&["-FTVw", "-I", "../common", "-o", "a.6", "acme.c"]);
        let (c, env) = kencc_command(&["rucc".into()], Path::new("/g"), &r).unwrap();
        assert_eq!(c[0], "rucc");
        assert!(c.contains(&"--driver=kencc".to_owned()), "{c:?}");
        assert!(c.contains(&"--target=x86_64-plan9".to_owned()), "{c:?}");
        assert!(c.contains(&"-FTVw".to_owned()), "{c:?}");
        assert!(
            c.iter()
                .any(|a| a.ends_with("acme.c") && a.starts_with("/g/")),
            "{c:?}"
        );
        assert!(
            env.iter()
                .any(|(k, v)| k == "INCLUDE" && v.contains("/g/amd64/include"))
        );
        assert!(env.iter().any(|(k, v)| k == "objtype" && v == "amd64"));
    }

    #[test]
    fn the_kencc_arguments_are_translated() {
        let r = record(&[
            "-FTVw",
            "-I",
            "../common",
            "-I/sys/include/ape",
            "-DX=1",
            "-o",
            "a.6",
            "acme.c",
        ]);
        let c = rucc_command(
            &["rucc".into(), "-fplan9-extensions".into()],
            Path::new("/g"),
            &r,
        );
        assert_eq!(
            c[1..],
            [
                "-fplan9-extensions",
                "--target=x86_64-unknown-linux-gnu",
                "-fsyntax-only",
                "-nostdinc",
                "-w",
                "-std=gnu11",
                "-iquote",
                ".",
                "-I../common",
                "-I/g/sys/include/ape",
                "-DX=1",
                "acme.c",
                "-I/g/amd64/include",
                "-I/g/sys/include",
                "-idirafter",
                ".",
            ]
        );
    }

    #[test]
    fn the_first_error_is_kept_without_names() {
        let err = "a.c:2:10: warning: declaration does not declare anything [E0547]\n\
                   a.c:3:26: error: 'struct R' has no member named 'k' [E0502]\n\
                   a.c:4:21: error: implicit declaration of function 'h' [E0521]\n";
        assert_eq!(
            first_error(err, "/sys/src/x", Path::new("/g")),
            Some(Result1::Error {
                code: "E0502".into(),
                message: "'_' has no member named '_'".into(),
                at: "/sys/src/x/a.c:3:26".into(),
                names: "struct R k".into(),
            })
        );
        assert_eq!(
            first_error("a.c:1:1: warning: w [E1]\n", "/d", Path::new("/g")),
            None
        );
        let Some(Result1::Error { at, .. }) = first_error(
            "/g/sys/include/bio.h:50:8: error: e [E1]\n",
            "/d",
            Path::new("/g/"),
        ) else {
            panic!("no error");
        };
        assert_eq!(at, "/sys/include/bio.h:50:8");
    }

    #[test]
    fn demands_are_ranked_by_units() {
        let e = |code: &str, at: &str| Result1::Error {
            code: code.into(),
            message: "m".into(),
            at: at.into(),
            names: if at == "a" {
                String::new()
            } else {
                format!("n{at}")
            },
        };
        let d = rank(
            "amd64",
            "rucc",
            &[
                e("E2", "a"),
                e("E1", "b"),
                e("E1", "b"),
                e("E1", "c"),
                Result1::Clean,
                Result1::Skipped,
            ],
        );
        assert_eq!((d.replayed, d.clean, d.skipped), (5, 1, 1));
        assert_eq!(d.demand[0].code, "E1");
        assert_eq!(d.demand[0].units, 3);
        assert_eq!(d.demand[0].examples, ["b", "c"]);
        assert_eq!(d.demand[0].names, ["nb (2)", "nc (1)"]);
        assert!(d.demand[1].names.is_empty());
        assert_eq!(d.demand[1].rank, 2);
        assert!(d.report(10).contains("E1"));
    }
}
