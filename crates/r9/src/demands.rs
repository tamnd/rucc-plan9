//! `r9 demands`: what rucc must learn first, ranked over the units of a G build (plan 05.6).
//!
//! Each unit that kencc compiled in a G build is compiled again with rucc on the host, with
//! `-fsyntax-only`, against a copy of the guest's tree and headers. The first error of each
//! unit is kept. The errors are grouped by their code and their text, with the names in quotes
//! taken out, and ranked by the number of units they stop.
//!
//! rucc has no kencc driver yet (tamnd/rucc#4147). Until it has one, the kencc arguments are
//! translated: `-I` and `-D` pass through, the kencc default include directories are added, and
//! the other flags are dropped. The target is x86-64 or aarch64 Linux, which is the closest
//! target rucc has. The command is in [`rucc_command`], so it changes in one place.

use crate::census::Record;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    },
    /// rucc stopped with no error line, or did not run.
    Crash(String),
    /// The unit was not replayed: its source was on standard input, or it is not for the
    /// guest's architecture.
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

/// The flags that every replay gets after the rucc program and its extra flags.
#[must_use]
pub fn base_flags(objtype: &str) -> Vec<String> {
    vec![
        format!("--target={}", target(objtype)),
        "-fsyntax-only".into(),
        "-nostdinc".into(),
        "-w".into(),
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
            c.push(a.clone());
        }
    }
    // kencc searches these after the -I directories.
    c.push(format!("-I{}", host(&format!("/{}/include", r.objtype))));
    c.push(format!("-I{}", host("/sys/include")));
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
    })
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

/// Replay one unit.
#[must_use]
pub fn replay(rucc: &[String], root: &Path, r: &Record) -> Result1 {
    let host_letter = match r.objtype.as_str() {
        "amd64" => "6c",
        "arm64" => "7c",
        _ => "",
    };
    if r.stdin || r.inputs.is_empty() || r.cc != host_letter {
        return Result1::Skipped;
    }
    let dir = root.join(r.dir.trim_start_matches('/'));
    let c = rucc_command(rucc, root, r);
    match Command::new(&c[0]).args(&c[1..]).current_dir(&dir).output() {
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
pub fn run(rucc: &[String], root: &Path, records: &[Record]) -> Demands {
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let chunk = records.len().div_ceil(threads).max(1);
    let results: Vec<Result1> = std::thread::scope(|s| {
        let handles: Vec<_> = records
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|r| replay(rucc, root, r))
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
    command.extend(base_flags(objtype));
    rank(objtype, &command.join(" "), &results)
}

/// Rank the results.
#[must_use]
pub fn rank(objtype: &str, rucc: &str, results: &[Result1]) -> Demands {
    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
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
                    .push(String::new());
            }
            Result1::Error { code, message, at } => groups
                .entry((code.clone(), message.clone()))
                .or_default()
                .push(at.clone()),
        }
    }
    let mut demand: Vec<Demand> = groups
        .into_iter()
        .map(|((code, message), places)| Demand {
            rank: 0,
            code,
            message,
            units: places.len(),
            examples: distinct(places, 3),
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
            origin: "ref-c".into(),
        }
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
                "-I../common",
                "-I/g/sys/include/ape",
                "-DX=1",
                "acme.c",
                "-I/g/amd64/include",
                "-I/g/sys/include",
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
        assert_eq!(d.demand[1].rank, 2);
        assert!(d.report(10).contains("E1"));
    }
}
