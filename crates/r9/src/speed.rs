//! `r9 speed --gcc`: the compile time of rucc against GCC 16 (plan 13.5).
//!
//! GCC cannot compile Plan 9 code for Plan 9, but GCC 16 with `-fplan9-extensions` compiles much
//! of the tree for the host. The units are the C calls of a build, from its `compile.jsonl`, with
//! the macros and the include directories that each mkfile gives. The sources and the headers
//! come from a copy of the pinned tree on the host. Each unit is compiled by GCC and then by
//! rucc, with the same flags and for the host target, and the CPU time of each compiler process
//! comes from `wait4`. Each unit runs `--runs` times on each side, and the median is kept.
//!
//! The gate is on the units that GCC compiles: the sum of the CPU time of rucc divided by the sum
//! of the CPU time of GCC must be at most 0.5. A unit that GCC compiles and rucc does not is a
//! failure of the gate. The objects are not used.

use crate::census::Record;
use crate::cli::Args;
use crate::repo::Repo;
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// The gate of plan 02.5: rucc's CPU time over GCC's.
pub const GATE: f64 = 0.5;

/// The largest peak resident memory of a rucc process (plan 02.5), in KiB.
pub const MAX_RSS_KIB: u64 = 1024 * 1024;

/// The flags that both compilers get before the flags of the unit. `USED` and `SET` are names
/// that kencc knows, and GCC does not, so both are macros with no body, as in plan 03.18.
#[must_use]
pub fn flags() -> Vec<String> {
    [
        "-fplan9-extensions",
        "-std=gnu99",
        "-O2",
        "-nostdinc",
        "-w",
        "-DUSED(...)=",
        "-DSET(...)=",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// One side of one unit.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Side {
    /// Whether each run compiled the unit.
    pub ok: bool,
    /// The median CPU time over the runs, in microseconds.
    pub cpu_us: u64,
    /// The largest peak resident memory over the runs, in KiB.
    pub max_rss_kib: u64,
    /// The first line of the standard error of a run that failed.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// One unit and its two sides.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Unit {
    /// The directory of the unit in the guest.
    pub dir: String,
    /// The sources, separated by spaces.
    pub sources: String,
    /// GCC.
    pub gcc: Side,
    /// rucc.
    pub rucc: Side,
}

impl Unit {
    /// rucc's CPU time over GCC's for this unit.
    #[must_use]
    pub fn ratio(&self) -> f64 {
        ratio(self.rucc.cpu_us, self.gcc.cpu_us)
    }

    /// The name of the unit in the reports.
    #[must_use]
    pub fn name(&self) -> String {
        format!("{}/{}", self.dir, self.sources)
    }
}

/// `a / b`, or 0 when `b` is 0.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "times in microseconds")]
pub fn ratio(a: u64, b: u64) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

/// What the command found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Speed {
    /// The target of the two compilers, from `gcc -dumpmachine`.
    pub host: String,
    /// The GCC command.
    pub gcc: String,
    /// The rucc command.
    pub rucc: String,
    /// The flags that both compilers got before the flags of each unit.
    pub flags: Vec<String>,
    /// The runs of each unit on each side.
    pub runs: usize,
    /// The C calls of the build that were not replayed: the source was on standard input, the
    /// build made the source, or the call was for another compiler.
    pub skipped: usize,
    /// The units that GCC compiles.
    pub gcc_units: usize,
    /// The units that GCC does not compile. They are not in the gate.
    pub gcc_failed: usize,
    /// The units that GCC compiles and rucc does not.
    pub rucc_failed: usize,
    /// The sum of GCC's CPU time over the units that GCC compiles, in microseconds.
    pub gcc_cpu_us: u64,
    /// The sum of rucc's CPU time over the same units, in microseconds.
    pub rucc_cpu_us: u64,
    /// `rucc_cpu_us / gcc_cpu_us`.
    pub ratio: f64,
    /// The 50th, 90th and 99th percentiles of the ratio of each unit.
    pub unit_ratio_percentiles: [f64; 3],
    /// The largest peak resident memory of a rucc process, in KiB.
    pub rucc_max_rss_kib: u64,
    /// Each unit.
    pub unit: Vec<Unit>,
}

impl Speed {
    /// Whether the gate passes: rucc compiles each unit that GCC compiles, the ratio is at most
    /// [`GATE`], and no rucc process goes over [`MAX_RSS_KIB`].
    #[must_use]
    pub fn passed(&self) -> bool {
        self.gcc_units > 0
            && self.rucc_failed == 0
            && self.ratio <= GATE
            && self.rucc_max_rss_kib <= MAX_RSS_KIB
    }
}

/// The value at the fraction `p` of sorted values, by the nearest rank.
#[must_use]
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an index into a short list"
    )]
    let i = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[i - 1]
}

/// Add the totals and the percentiles to `s` from its units.
pub fn total(s: &mut Speed) {
    let compiled: Vec<&Unit> = s.unit.iter().filter(|u| u.gcc.ok).collect();
    s.gcc_units = compiled.len();
    s.gcc_failed = s.unit.len() - compiled.len();
    s.rucc_failed = compiled.iter().filter(|u| !u.rucc.ok).count();
    let both: Vec<&&Unit> = compiled.iter().filter(|u| u.rucc.ok).collect();
    s.gcc_cpu_us = both.iter().map(|u| u.gcc.cpu_us).sum();
    s.rucc_cpu_us = both.iter().map(|u| u.rucc.cpu_us).sum();
    s.ratio = ratio(s.rucc_cpu_us, s.gcc_cpu_us);
    let mut ratios: Vec<f64> = both.iter().map(|u| u.ratio()).collect();
    ratios.sort_by(f64::total_cmp);
    s.unit_ratio_percentiles = [
        percentile(&ratios, 0.5),
        percentile(&ratios, 0.9),
        percentile(&ratios, 0.99),
    ];
    s.rucc_max_rss_kib = s.unit.iter().map(|u| u.rucc.max_rss_kib).max().unwrap_or(0);
}

/// Whether a call can be compiled on the host: it is a C call of the row's compiler, and each of
/// its sources is in the copy of the tree.
#[must_use]
pub fn replayable(root: &Path, r: &Record) -> bool {
    let letter = match r.objtype.as_str() {
        "amd64" => "6c",
        "arm64" => "7c",
        _ => return false,
    };
    !r.stdin
        && !r.inputs.is_empty()
        && r.cc == letter
        && r.inputs.iter().all(|f| {
            let path = crate::ccd::absolute(&r.dir, &f.file);
            root.join(path.trim_start_matches('/')).is_file()
        })
}

/// The median of some durations, in microseconds.
fn median_us(mut times: Vec<Duration>) -> u64 {
    times.sort();
    times
        .get(times.len().saturating_sub(1) / 2)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

/// Compile one unit once with `cc`. The object goes to `obj`.
fn compile_once(
    cc: &[String],
    root: &Path,
    r: &Record,
    obj: &Path,
) -> Result<r9_rusage::Usage, String> {
    let mut c = Command::new(&cc[0]);
    c.args(&cc[1..])
        .args(flags())
        .arg("-c")
        .args(crate::demands::unit_flags(root, r))
        .arg("-o")
        .arg(obj)
        .current_dir(root.join(r.dir.trim_start_matches('/')));
    let out = r9_rusage::output(&mut c).map_err(|e| format!("cannot run {}: {e}", cc[0]))?;
    if out.usage.status.success() {
        Ok(out.usage)
    } else {
        let text = String::from_utf8_lossy(&out.stderr);
        let line = text
            .lines()
            .find(|l| l.contains("error"))
            .or_else(|| text.lines().next())
            .unwrap_or("no message");
        Err(format!(
            "{}: {}",
            out.usage.status,
            line.chars().take(200).collect::<String>()
        ))
    }
}

/// The sizes in bytes of the preprocessed text of a unit, from `cc -E`.
fn preprocessed(cc: &[String], root: &Path, r: &Record) -> usize {
    let mut c = Command::new(&cc[0]);
    c.args(&cc[1..])
        .args(flags())
        .arg("-E")
        .args(crate::demands::unit_flags(root, r))
        .current_dir(root.join(r.dir.trim_start_matches('/')));
    r9_rusage::output(&mut c).map_or(0, |o| o.stdout.len())
}

/// Compile one unit `runs` times on each side, GCC first in each run.
fn unit(gcc: &[String], rucc: &[String], root: &Path, r: &Record, runs: usize, obj: &Path) -> Unit {
    let mut times = [Vec::new(), Vec::new()];
    let mut sides = [Side::default(), Side::default()];
    for _ in 0..runs.max(1) {
        for (i, cc) in [gcc, rucc].into_iter().enumerate() {
            if !sides[i].error.is_empty() {
                continue;
            }
            match compile_once(cc, root, r, obj) {
                Ok(u) => {
                    times[i].push(u.cpu());
                    sides[i].max_rss_kib = sides[i].max_rss_kib.max(u.max_rss_kib);
                }
                Err(e) => sides[i].error = e,
            }
        }
    }
    let _ = std::fs::remove_file(obj);
    for (side, t) in sides.iter_mut().zip(times) {
        side.ok = side.error.is_empty();
        side.cpu_us = median_us(t);
    }
    let [g, rc] = sides;
    Unit {
        dir: r.dir.clone(),
        sources: r
            .inputs
            .iter()
            .map(|f| f.file.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        gcc: g,
        rucc: rc,
    }
}

/// The first line of the output of a command, or empty.
fn first_line(c: &[String], arg: &str) -> String {
    Command::new(&c[0])
        .args(&c[1..])
        .arg(arg)
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// Compile each replayable unit of `records` with GCC and rucc, on `jobs` threads.
#[must_use]
pub fn run(
    gcc: &[String],
    rucc: &[String],
    root: &Path,
    records: &[Record],
    runs: usize,
    jobs: usize,
    scratch: &Path,
) -> Speed {
    let todo: Vec<&Record> = records.iter().filter(|r| replayable(root, r)).collect();
    let skipped = records.len() - todo.len();
    let next = AtomicUsize::new(0);
    let done = Mutex::new(vec![None; todo.len()]);
    std::thread::scope(|s| {
        for t in 0..jobs.max(1) {
            let (todo, next, done) = (&todo, &next, &done);
            let obj = scratch.join(format!("unit-{t}.o"));
            s.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(r) = todo.get(i) else { break };
                    let u = unit(gcc, rucc, root, r, runs, &obj);
                    done.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)[i] = Some(u);
                    if (i + 1) % 200 == 0 {
                        eprintln!("r9 speed: {} of {} units", i + 1, todo.len());
                    }
                }
            });
        }
    });
    let mut s = Speed {
        host: first_line(gcc, "-dumpmachine"),
        gcc: gcc.join(" "),
        rucc: rucc.join(" "),
        flags: flags(),
        runs,
        skipped,
        unit: done
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .into_iter()
            .flatten()
            .collect(),
        ..Speed::default()
    };
    total(&mut s);
    s
}

/// Format microseconds as seconds with three places.
#[allow(clippy::cast_precision_loss, reason = "a time to print")]
fn secs(us: u64) -> String {
    format!("{:.3}s", us as f64 / 1e6)
}

/// The report: the totals, the gate, the units that rucc does not compile, and the `top` units
/// with the largest ratio, with the sizes of their preprocessed text in `sizes`.
#[must_use]
pub fn report(s: &Speed, top: &[(&Unit, usize)]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "host:  {}", s.host);
    let _ = writeln!(out, "gcc:   {}", s.gcc);
    let _ = writeln!(out, "rucc:  {}", s.rucc);
    let _ = writeln!(out, "flags: {}", s.flags.join(" "));
    let _ = writeln!(
        out,
        "runs:  {} of each unit on each side; the median is kept",
        s.runs
    );
    let _ = writeln!(
        out,
        "units: {} that GCC compiles, {} that GCC does not compile, {} calls not replayed",
        s.gcc_units, s.gcc_failed, s.skipped
    );
    if s.rucc_failed > 0 {
        let _ = writeln!(
            out,
            "\nrucc does not compile {} units that GCC compiles:",
            s.rucc_failed
        );
        for u in s.unit.iter().filter(|u| u.gcc.ok && !u.rucc.ok) {
            let _ = writeln!(out, "  {}: {}", u.name(), u.rucc.error);
        }
    }
    let [p50, p90, p99] = s.unit_ratio_percentiles;
    let _ = writeln!(
        out,
        "\nthe ratio of each unit: p50 {p50:.3}, p90 {p90:.3}, p99 {p99:.3}"
    );
    let _ = writeln!(
        out,
        "the peak resident memory of rucc: {} MiB (the limit is {} MiB)",
        s.rucc_max_rss_kib / 1024,
        MAX_RSS_KIB / 1024
    );
    if !top.is_empty() {
        let _ = writeln!(out, "\nthe {} units with the largest ratio:", top.len());
        let _ = writeln!(out, "  ratio  rucc      gcc       preprocessed  unit");
        for (u, size) in top {
            let _ = writeln!(
                out,
                "  {:<6.3} {:<9} {:<9} {:<13} {}",
                u.ratio(),
                secs(u.rucc.cpu_us),
                secs(u.gcc.cpu_us),
                size,
                u.name()
            );
        }
    }
    let _ = writeln!(
        out,
        "\nCPU time: rucc {}, gcc {}, ratio {:.3} (the gate is {GATE})",
        secs(s.rucc_cpu_us),
        secs(s.gcc_cpu_us),
        s.ratio
    );
    let _ = writeln!(
        out,
        "r9 speed --gcc: {}",
        if s.passed() { "PASS" } else { "FAIL" }
    );
    out
}

/// The units with the largest ratio, at most `n`, with the sizes of their preprocessed text.
fn worst<'a>(
    s: &'a Speed,
    gcc: &[String],
    root: &Path,
    records: &[Record],
    n: usize,
) -> Vec<(&'a Unit, usize)> {
    let mut both: Vec<&Unit> = s.unit.iter().filter(|u| u.gcc.ok && u.rucc.ok).collect();
    both.sort_by(|a, b| b.ratio().total_cmp(&a.ratio()));
    both.truncate(n);
    both.into_iter()
        .map(|u| {
            let size = records
                .iter()
                .find(|r| {
                    r.dir == u.dir
                        && r.inputs
                            .iter()
                            .map(|f| f.file.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                            == u.sources
                })
                .map_or(0, |r| preprocessed(gcc, root, r));
            (u, size)
        })
        .collect()
}

/// Write the parts of the pinned tree that the units read to `dir`: `sys/src`, `sys/include`
/// and `OBJTYPE/include`. Return `dir`.
pub fn pinned_tree(repo: &Repo, objtype: &str, dir: &Path) -> Result<PathBuf, String> {
    let pins = r9_pins::pins::PinsFile::load(&repo.root)?
        .ok_or("there is no pins.toml; run r9 sets --write")?;
    let (_, tree) = pins.tree.iter().next().ok_or("pins.toml has no tree")?;
    let history = crate::locate::history(&repo.cache, &tree.repo)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tar = dir.join("tree.tar");
    let paths = [
        "sys/src".to_string(),
        "sys/include".to_string(),
        format!("{objtype}/include"),
    ];
    crate::image::archive(&history, &tree.commit, &paths, &tar)?;
    let status = Command::new("tar")
        .arg("xf")
        .arg(&tar)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar cannot read {}", tar.display()));
    }
    std::fs::remove_file(&tar).map_err(|e| format!("{}: {e}", tar.display()))?;
    Ok(dir.to_path_buf())
}

/// Run `r9 speed`. Return whether the gate passed, and the report.
pub fn command(repo: &Repo, args: &Args) -> Result<(bool, String), String> {
    if !args.has("gcc") {
        return Err("r9 speed has one mode now: give --gcc".into());
    }
    let calls = args
        .target
        .as_deref()
        .ok_or("give the compile.jsonl of a build")?;
    let records = crate::census::load(Path::new(calls))?;
    let objtype = records
        .first()
        .map(|r| r.objtype.clone())
        .ok_or_else(|| format!("{calls} has no calls"))?;
    let out = args
        .get("out")
        .map_or_else(|| repo.root.join("work").join("speed"), PathBuf::from);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let root = match args.get("tree") {
        Some(t) => PathBuf::from(t),
        None => pinned_tree(repo, &objtype, &out.join("tree"))?,
    };
    let number = |name: &str, default: usize| -> Result<usize, String> {
        args.get(name)
            .map_or(Ok(default), str::parse)
            .map_err(|e| format!("--{name}: {e}"))
    };
    let runs = number("runs", 3)?;
    let jobs = number("jobs", 1)?;
    let top = number("top", 50)?;
    let gcc: Vec<String> = args
        .get("cc")
        .unwrap_or("gcc-16")
        .split_whitespace()
        .map(String::from)
        .collect();
    let rucc = crate::demands::find_rucc(None);
    let scratch = out.join("scratch");
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let s = run(&gcc, &rucc, &root, &records, runs, jobs, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    let json = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    let file = out.join("speed.json");
    std::fs::write(&file, json).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = report(&s, &worst(&s, &gcc, &root, &records, top));
    let file = out.join("speed.txt");
    std::fs::write(&file, &text).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok((s.passed(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(ok: bool, cpu_us: u64) -> Side {
        Side {
            ok,
            cpu_us,
            max_rss_kib: 1000,
            error: if ok { String::new() } else { "1: error".into() },
        }
    }

    fn unit(name: &str, gcc: Side, rucc: Side) -> Unit {
        Unit {
            dir: "/sys/src/libc/port".into(),
            sources: name.into(),
            gcc,
            rucc,
        }
    }

    #[test]
    fn the_totals_count_only_the_units_that_gcc_compiles() {
        let mut s = Speed {
            unit: vec![
                unit("a.c", side(true, 1000), side(true, 400)),
                unit("b.c", side(true, 3000), side(true, 1100)),
                unit("c.c", side(false, 0), side(true, 9000)),
            ],
            ..Speed::default()
        };
        total(&mut s);
        assert_eq!((s.gcc_units, s.gcc_failed, s.rucc_failed), (2, 1, 0));
        assert_eq!((s.gcc_cpu_us, s.rucc_cpu_us), (4000, 1500));
        assert!((s.ratio - 0.375).abs() < 1e-9);
        assert!(s.passed());
    }

    #[test]
    fn a_unit_that_rucc_does_not_compile_fails_the_gate() {
        let mut s = Speed {
            unit: vec![
                unit("a.c", side(true, 1000), side(true, 100)),
                unit("b.c", side(true, 1000), side(false, 0)),
            ],
            ..Speed::default()
        };
        total(&mut s);
        assert_eq!(s.rucc_failed, 1);
        assert!(!s.passed());
        assert!(report(&s, &[]).contains("rucc does not compile 1 units"));
    }

    #[test]
    fn a_slow_rucc_fails_the_gate() {
        let mut s = Speed {
            unit: vec![unit("a.c", side(true, 1000), side(true, 600))],
            ..Speed::default()
        };
        total(&mut s);
        assert!(!s.passed());
        assert!(report(&s, &[]).ends_with("r9 speed --gcc: FAIL\n"));
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert!((percentile(&v, 0.5) - 50.0).abs() < 1e-9);
        assert!((percentile(&v, 0.9) - 90.0).abs() < 1e-9);
        assert!((percentile(&v, 0.99) - 99.0).abs() < 1e-9);
        assert!((percentile(&[], 0.5)).abs() < 1e-9);
    }

    #[test]
    fn the_median_of_three_runs_is_the_middle_one() {
        let t = vec![
            Duration::from_micros(30),
            Duration::from_micros(10),
            Duration::from_micros(20),
        ];
        assert_eq!(median_us(t), 20);
    }
}
