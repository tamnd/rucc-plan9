//! `r9 bench`: the run-time benchmarks of plan 13.2, with the method of plan 13.3.
//!
//! `bench/r9-bench` runs in the guest. It builds the programs in `bench/c` with the compiler of
//! the image, makes the inputs with `r9gen`, and runs each benchmark. Each benchmark prints one
//! line, `bench NAME NS OPS`, or `skip NAME: reason` when it cannot run.
//!
//! `r9 bench` boots the G image and the R image in turn, `--runs` times each. For each
//! benchmark, the number is the median of NS over the runs of a side, and the ratio is R over G,
//! so a ratio under 1 means that R is faster. A benchmark is gated when it ran on each run of
//! both sides, and its coefficient of variation on G is at most [`MAX_CV`]. The headline is the
//! geometric mean of the gated ratios, and the gate is [`GATE`].

use crate::cli::Args;
use crate::repo::Repo;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The gate of plan 02.5: the geometric mean of R over G.
pub const GATE: f64 = 1.0;

/// The largest coefficient of variation on G of a gated benchmark (plan 13.3).
pub const MAX_CV: f64 = 0.01;

/// The times of one run, in nanoseconds, by benchmark.
pub type Run = BTreeMap<String, u64>;

/// Read the `bench` lines of a console. A `skip` line or a line that is not whole is not a
/// result. A name that comes twice keeps its last time.
#[must_use]
pub fn parse(console: &str) -> Run {
    let mut run = Run::new();
    for line in console.lines() {
        let words: Vec<&str> = line.trim().split(' ').collect();
        if let ["bench", name, ns, _ops] = words[..]
            && let Ok(ns) = ns.parse::<u64>()
        {
            run.insert((*name).to_string(), ns);
        }
    }
    run
}

/// One benchmark over the runs of both sides.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Row {
    /// The name.
    pub name: String,
    /// The times on G, in nanoseconds, one for each run where it ran.
    pub g: Vec<u64>,
    /// The times on R.
    pub r: Vec<u64>,
    /// The median on G.
    pub g_median: u64,
    /// The median on R.
    pub r_median: u64,
    /// `r_median / g_median`.
    pub ratio: f64,
    /// The coefficient of variation on G.
    pub g_cv: f64,
    /// Whether the ratio is in the gate.
    pub gated: bool,
    /// Why it is not, when it is not.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// What `r9 bench` found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Bench {
    /// The runs of each side.
    pub runs: usize,
    /// The G image.
    pub g_image: String,
    /// The R image.
    pub r_image: String,
    /// Each benchmark.
    pub rows: Vec<Row>,
    /// The number of gated benchmarks.
    pub gated: usize,
    /// The geometric mean of the gated ratios.
    pub geomean: f64,
}

impl Bench {
    /// Whether the gate passes.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.gated > 0 && self.geomean <= GATE
    }
}

/// The median of some times.
#[must_use]
pub fn median(v: &[u64]) -> u64 {
    let mut s = v.to_vec();
    s.sort_unstable();
    match s.len() {
        0 => 0,
        n if n % 2 == 1 => s[n / 2],
        n => u64::midpoint(s[n / 2 - 1], s[n / 2]),
    }
}

/// The coefficient of variation of some times: the standard deviation over the mean.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "times in nanoseconds")]
pub fn cv(v: &[u64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let n = v.len() as f64;
    let mean = v.iter().map(|&x| x as f64).sum::<f64>() / n;
    if mean == 0.0 {
        return 0.0;
    }
    let var = v.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0);
    var.sqrt() / mean
}

/// Put the runs of both sides together.
#[must_use]
pub fn combine(g: &[Run], r: &[Run]) -> Bench {
    let mut names: Vec<&String> = g.iter().chain(r).flat_map(BTreeMap::keys).collect();
    names.sort();
    names.dedup();
    let mut b = Bench {
        runs: g.len().max(r.len()),
        ..Bench::default()
    };
    for name in names {
        let times = |runs: &[Run]| -> Vec<u64> {
            runs.iter()
                .filter_map(|run| run.get(name).copied())
                .collect()
        };
        let mut row = Row {
            name: name.clone(),
            g: times(g),
            r: times(r),
            ..Row::default()
        };
        row.g_median = median(&row.g);
        row.r_median = median(&row.r);
        row.ratio = crate::speed::ratio(row.r_median, row.g_median);
        row.g_cv = cv(&row.g);
        row.why = if row.g.len() < g.len() || row.r.len() < r.len() || row.g_median == 0 {
            "it did not run on each run".into()
        } else if row.g_cv > MAX_CV {
            format!("the variation on G is {:.1}%", row.g_cv * 100.0)
        } else {
            String::new()
        };
        row.gated = row.why.is_empty();
        b.rows.push(row);
    }
    let gated: Vec<f64> = b.rows.iter().filter(|r| r.gated).map(|r| r.ratio).collect();
    b.gated = gated.len();
    #[allow(clippy::cast_precision_loss, reason = "a count of benchmarks")]
    let mean = gated.iter().map(|x| x.ln()).sum::<f64>() / gated.len().max(1) as f64;
    b.geomean = if gated.is_empty() { 0.0 } else { mean.exp() };
    b
}

/// Format nanoseconds as milliseconds.
#[allow(clippy::cast_precision_loss, reason = "a time to print")]
fn ms(ns: u64) -> String {
    format!("{:.2}", ns as f64 / 1e6)
}

/// The report: a line for each benchmark, then the headline.
#[must_use]
pub fn report(b: &Bench) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "G: {}", b.g_image);
    let _ = writeln!(out, "R: {}", b.r_image);
    let _ = writeln!(out, "{} runs of each side, in turn\n", b.runs);
    let _ = writeln!(
        out,
        "{:<12} {:>12} {:>12} {:>7} {:>7}  note",
        "benchmark", "G ms", "R ms", "R/G", "G cv"
    );
    for r in &b.rows {
        let _ = writeln!(
            out,
            "{:<12} {:>12} {:>12} {:>7.3} {:>6.2}%  {}",
            r.name,
            ms(r.g_median),
            ms(r.r_median),
            r.ratio,
            r.g_cv * 100.0,
            if r.gated { "gated" } else { r.why.as_str() }
        );
    }
    let _ = writeln!(
        out,
        "\n{} benchmarks, {} gated; the geometric mean of R/G is {:.3} (the gate is {GATE:.2})",
        b.rows.len(),
        b.gated,
        b.geomean
    );
    let _ = writeln!(
        out,
        "r9 bench: {}",
        if b.passed() { "PASS" } else { "FAIL" }
    );
    out
}

/// Boot `image` once with the benchmarks, and read the times.
fn boot_once(
    repo: &Repo,
    args: &Args,
    objtype: &str,
    image: &Path,
    work: &Path,
) -> Result<Run, String> {
    let mut a = args.clone();
    a.target = None;
    let path = |p: &Path| p.display().to_string();
    let dir = repo.root.join("bench");
    a.values.insert("row".into(), objtype.to_string());
    a.values.insert("image".into(), path(image));
    a.values.insert("run".into(), path(&dir.join("run.rc")));
    a.values.insert("in".into(), path(&dir.join("guest")));
    a.values.insert("out".into(), path(work));
    a.values
        .entry("timeout".into())
        .or_insert_with(|| "3600".into());
    let req = crate::boot::request_from(repo, &a)?;
    let outcome = crate::boot::run(&req)?;
    let console = std::fs::read_to_string(work.join("console.log")).unwrap_or_default();
    if !console.contains("bench done") {
        eprintln!(
            "r9 bench: {} did not finish the benchmarks{}; console in {}",
            image.display(),
            if outcome.timed_out {
                " in the time"
            } else {
                ""
            },
            work.join("console.log").display()
        );
    }
    Ok(parse(&console))
}

/// Run `r9 bench --r FILE`. `--g FILE` is the G image, and the default is the `g-base` image of
/// the row. The work goes to `work/bench` or `--out`.
pub fn command(repo: &Repo, args: &Args) -> Result<(bool, String), String> {
    let objtype = args.get("row").unwrap_or("amd64").to_string();
    let r_image = PathBuf::from(
        args.get("r")
            .ok_or("r9 bench needs --r FILE, the image of an R build")?,
    );
    let g_image = match args.get("g") {
        Some(g) => PathBuf::from(g),
        None => crate::image::g_base(repo, &objtype).ok_or_else(|| {
            format!("there is no g-base image for {objtype}; run r9 image build --row {objtype}")
        })?,
    };
    let runs: usize = args
        .get("runs")
        .map_or(Ok(10), str::parse)
        .map_err(|e| format!("--runs: {e}"))?;
    let out = args
        .get("out")
        .map_or_else(|| repo.root.join("work").join("bench"), PathBuf::from);
    let (mut g, mut r) = (Vec::new(), Vec::new());
    for n in 1..=runs.max(1) {
        eprintln!("r9 bench: run {n} of {runs}, G");
        g.push(boot_once(
            repo,
            args,
            &objtype,
            &g_image,
            &out.join(format!("g-{n}")),
        )?);
        eprintln!("r9 bench: run {n} of {runs}, R");
        r.push(boot_once(
            repo,
            args,
            &objtype,
            &r_image,
            &out.join(format!("r-{n}")),
        )?);
    }
    let mut b = combine(&g, &r);
    b.g_image = g_image.display().to_string();
    b.r_image = r_image.display().to_string();
    let text = report(&b);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let json = serde_json::to_string_pretty(&b).map_err(|e| e.to_string())?;
    for (name, body) in [("bench.json", json.as_str()), ("bench.txt", text.as_str())] {
        let p = out.join(name);
        std::fs::write(&p, body).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok((b.passed(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pairs: &[(&str, u64)]) -> Run {
        pairs.iter().map(|(n, t)| ((*n).to_string(), *t)).collect()
    }

    #[test]
    fn the_console_gives_the_bench_lines() {
        let console = "boot\r\nbench errstr 1200 200000\r\nskip udp: dial: no route\r\n\
                       bench sha256 990 1\nbench bad x 1\nbench done\n";
        assert_eq!(parse(console), run(&[("errstr", 1200), ("sha256", 990)]));
    }

    #[test]
    fn the_median_and_the_variation() {
        assert_eq!(median(&[5, 1, 3]), 3);
        assert_eq!(median(&[4, 1, 3, 2]), 2);
        assert_eq!(median(&[]), 0);
        assert!(cv(&[100, 100, 100]).abs() < 1e-12);
        assert!((cv(&[90, 110]) - 0.141_421).abs() < 1e-5);
    }

    #[test]
    fn a_noisy_or_missing_benchmark_is_not_gated() {
        let g = [
            run(&[("a", 1000), ("b", 1000), ("c", 100)]),
            run(&[("a", 1002), ("b", 1300), ("c", 100)]),
            run(&[("a", 1001), ("b", 1000)]),
        ];
        let r = [
            run(&[("a", 500), ("b", 900), ("c", 50)]),
            run(&[("a", 502), ("b", 900), ("c", 50)]),
            run(&[("a", 501), ("b", 900), ("c", 50)]),
        ];
        let b = combine(&g, &r);
        let row = |n: &str| b.rows.iter().find(|r| r.name == n).unwrap();
        assert!(row("a").gated);
        assert!(!row("b").gated && row("b").why.starts_with("the variation on G"));
        assert!(!row("c").gated && row("c").why == "it did not run on each run");
        assert_eq!(b.gated, 1);
        assert!((b.geomean - 0.5).abs() < 0.01);
        assert!(b.passed());
        assert!(report(&b).ends_with("r9 bench: PASS\n"));
    }

    #[test]
    fn the_geometric_mean_of_two_ratios() {
        let g = [
            run(&[("a", 100), ("b", 100)]),
            run(&[("a", 100), ("b", 100)]),
        ];
        let r = [run(&[("a", 50), ("b", 400)]), run(&[("a", 50), ("b", 400)])];
        let b = combine(&g, &r);
        assert!((b.geomean - std::f64::consts::SQRT_2).abs() < 1e-5);
        assert!(!b.passed());
    }
}
