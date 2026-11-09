//! `r9 census`: what reached the compiler slot in one build (plan 05.6).
//!
//! The input is the `compile.jsonl` that `r9-cc` writes in the guest, one record per call. The
//! census counts the calls by origin and by compiler, lists the calls that failed, the calls
//! that read the source on standard input (from `pcc`), and the sources that were compiled more
//! than once, with whether their outputs agree. For an R build, a call with origin `ref-c`
//! fails the census.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// One file of a record.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct File {
    /// The path as the compiler saw it.
    pub file: String,
    /// The SHA-256 in hex, or empty when the file could not be read.
    pub sha256: String,
}

/// One call of the compiler slot.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Record {
    /// The name the compiler was called by, such as `6c`.
    pub cc: String,
    /// The `objtype` of the guest.
    pub objtype: String,
    /// The current directory.
    pub dir: String,
    /// The arguments after the name.
    pub argv: Vec<String>,
    /// The sources.
    #[serde(rename = "in")]
    pub inputs: Vec<File>,
    /// The outputs.
    #[serde(rename = "out")]
    pub outputs: Vec<File>,
    /// Whether the source came on standard input.
    pub stdin: bool,
    /// The exit status, empty for success.
    pub status: String,
    /// The time of the call in milliseconds.
    pub ms: u64,
    /// The user time of the compiler in milliseconds, from its wait message. Records from
    /// before this field have 0.
    #[serde(default)]
    pub user_ms: u64,
    /// The system time of the compiler in milliseconds, from its wait message.
    #[serde(default)]
    pub sys_ms: u64,
    /// `rucc`, `ref-c`, `ref-as` or `other-arch`.
    pub origin: String,
}

/// Read a `compile.jsonl` file. A line that does not parse is an error with its line number.
pub fn load(path: &Path) -> Result<Vec<Record>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Parse the text of a `compile.jsonl` file.
pub fn parse(text: &str) -> Result<Vec<Record>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

/// The census of one build.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Census {
    /// The number of calls.
    pub calls: usize,
    /// The calls by origin.
    pub by_origin: BTreeMap<String, usize>,
    /// The calls by compiler name.
    pub by_cc: BTreeMap<String, usize>,
    /// The calls that failed, as `dir: argv: status`.
    pub failed: Vec<String>,
    /// The calls with the source on standard input.
    pub stdin: usize,
    /// The sources compiled more than once, by SHA-256: the places, and whether every output
    /// was the same.
    pub repeated: BTreeMap<String, (Vec<String>, bool)>,
    /// The time of all calls, in milliseconds.
    pub ms: u64,
    /// The time by directory, in milliseconds.
    pub ms_by_dir: BTreeMap<String, u64>,
    /// The user and system time of all calls, in milliseconds.
    pub cpu_ms: u64,
    /// The spread of the time of one call.
    pub unit_ms: Spread,
    /// The spread of the user and system time of one call.
    pub unit_cpu_ms: Spread,
}

/// The spread of a list of times in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spread {
    /// The median.
    pub median: u64,
    /// The 90th percentile.
    pub p90: u64,
    /// The 99th percentile.
    pub p99: u64,
    /// The largest time.
    pub max: u64,
}

impl Spread {
    /// The spread of `times`.
    #[must_use]
    pub fn of(mut times: Vec<u64>) -> Self {
        if times.is_empty() {
            return Self::default();
        }
        times.sort_unstable();
        let at = |q: usize| times[(times.len() * q / 100).min(times.len() - 1)];
        Self {
            median: at(50),
            p90: at(90),
            p99: at(99),
            max: times[times.len() - 1],
        }
    }
}

impl std::fmt::Display for Spread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "median {} ms, p90 {} ms, p99 {} ms, max {} ms",
            self.median, self.p90, self.p99, self.max
        )
    }
}

/// Take the census of the records.
#[must_use]
pub fn take(records: &[Record]) -> Census {
    let mut c = Census {
        calls: records.len(),
        ..Census::default()
    };
    let mut by_source: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for r in records {
        *c.by_origin.entry(r.origin.clone()).or_default() += 1;
        *c.by_cc.entry(r.cc.clone()).or_default() += 1;
        if !r.status.is_empty() {
            c.failed.push(format!(
                "{}: {} {}: {}",
                r.dir,
                r.cc,
                r.argv.join(" "),
                r.status
            ));
        }
        if r.stdin {
            c.stdin += 1;
        }
        c.ms += r.ms;
        c.cpu_ms += r.user_ms + r.sys_ms;
        *c.ms_by_dir.entry(r.dir.clone()).or_default() += r.ms;
        if let ([input], [output]) = (r.inputs.as_slice(), r.outputs.as_slice())
            && !input.sha256.is_empty()
        {
            by_source
                .entry(input.sha256.clone())
                .or_default()
                .push((format!("{}/{}", r.dir, input.file), output.sha256.clone()));
        }
    }
    c.unit_ms = Spread::of(records.iter().map(|r| r.ms).collect());
    c.unit_cpu_ms = Spread::of(records.iter().map(|r| r.user_ms + r.sys_ms).collect());
    for (hash, places) in by_source {
        if places.len() > 1 {
            let same = places.iter().all(|(_, o)| *o == places[0].1);
            let names = places.into_iter().map(|(p, _)| p).collect();
            c.repeated.insert(hash, (names, same));
        }
    }
    c
}

impl Census {
    /// The problems that fail the census of an R build: calls that went to kencc, and calls
    /// that failed.
    #[must_use]
    pub fn problems(&self, side: Side) -> Vec<String> {
        let mut p = Vec::new();
        if side == Side::R {
            let refc = self.by_origin.get("ref-c").copied().unwrap_or(0);
            if refc > 0 {
                p.push(format!("{refc} calls went to kencc (ref-c)"));
            }
        }
        if !self.failed.is_empty() {
            p.push(format!("{} calls failed", self.failed.len()));
        }
        p
    }

    /// The report `r9 census` prints.
    #[must_use]
    pub fn report(&self, side: Side) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "calls      {}", self.calls);
        for (origin, n) in &self.by_origin {
            let _ = writeln!(out, "  origin   {origin:<12} {n}");
        }
        for (cc, n) in &self.by_cc {
            let _ = writeln!(out, "  cc       {cc:<12} {n}");
        }
        let _ = writeln!(out, "stdin      {}", self.stdin);
        let _ = writeln!(out, "time       {:.1}s", ms_to_s(self.ms));
        let _ = writeln!(out, "  per call {}", self.unit_ms);
        if self.cpu_ms > 0 {
            let _ = writeln!(out, "cpu        {:.1}s", ms_to_s(self.cpu_ms));
            let _ = writeln!(out, "  per call {}", self.unit_cpu_ms);
        }
        let mut dirs: Vec<_> = self.ms_by_dir.iter().collect();
        dirs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (dir, ms) in dirs.into_iter().take(10) {
            let _ = writeln!(out, "  {:>8.1}s {dir}", ms_to_s(*ms));
        }
        let differ = self.repeated.values().filter(|(_, same)| !same).count();
        let _ = writeln!(
            out,
            "repeated   {} sources, {differ} with different outputs",
            self.repeated.len()
        );
        for (places, same) in self.repeated.values() {
            if !same {
                let _ = writeln!(out, "  differ   {}", places.join(" "));
            }
        }
        let _ = writeln!(out, "failed     {}", self.failed.len());
        for f in self.failed.iter().take(20) {
            let _ = writeln!(out, "  {f}");
        }
        let problems = self.problems(side);
        if problems.is_empty() {
            let _ = writeln!(out, "census of the {side:?} build passes");
        } else {
            let _ = writeln!(
                out,
                "census of the {side:?} build fails: {}",
                problems.join("; ")
            );
        }
        out
    }
}

/// The side of a build: R is rucc in the compiler slot, G is kencc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// rucc.
    R,
    /// kencc.
    G,
}

impl Side {
    /// The side named on the command line, or the side the records show: R when any call has
    /// origin `rucc`.
    pub fn pick(name: Option<&str>, records: &[Record]) -> Result<Self, String> {
        match name {
            Some("R" | "r") => Ok(Self::R),
            Some("G" | "g") => Ok(Self::G),
            Some(other) => Err(format!("--side {other}: give R or G")),
            None if records.iter().any(|r| r.origin == "rucc") => Ok(Self::R),
            None => Ok(Self::G),
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn ms_to_s(ms: u64) -> f64 {
    ms as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"cc":"6c","objtype":"amd64","dir":"/sys/src/libbio","argv":["-FTVw","bfildes.c"],"in":[{"file":"bfildes.c","sha256":"aa"}],"out":[{"file":"bfildes.6","sha256":"b1"}],"stdin":false,"status":"","ms":268,"user_ms":200,"sys_ms":40,"origin":"ref-c"}
{"cc":"6c","objtype":"amd64","dir":"/sys/src/cmd/x","argv":["-FTVw","../../libbio/bfildes.c"],"in":[{"file":"../../libbio/bfildes.c","sha256":"aa"}],"out":[{"file":"bfildes.6","sha256":"b2"}],"stdin":false,"status":"","ms":100,"origin":"ref-c"}
{"cc":"8c","objtype":"amd64","dir":"/sys/src/9/pc","argv":["-FTVw","main.c"],"in":[{"file":"main.c","sha256":"cc"}],"out":[{"file":"main.8","sha256":""}],"stdin":false,"status":"8c 99: error","ms":5,"origin":"other-arch"}
{"cc":"6c","objtype":"amd64","dir":"/sys/src/ape/cmd","argv":["-o","x.6"],"in":[],"out":[{"file":"x.6","sha256":"dd"}],"stdin":true,"status":"","ms":7,"origin":"ref-c"}
"#;

    #[test]
    fn a_census_counts_the_calls() {
        let records = parse(SAMPLE).unwrap();
        let c = take(&records);
        assert_eq!(c.calls, 4);
        assert_eq!(c.by_origin["ref-c"], 3);
        assert_eq!(c.by_cc["8c"], 1);
        assert_eq!(c.stdin, 1);
        assert_eq!(c.ms, 380);
        assert_eq!(c.cpu_ms, 240);
        assert_eq!(
            c.unit_ms,
            Spread {
                median: 100,
                p90: 268,
                p99: 268,
                max: 268
            }
        );
        assert_eq!(c.unit_cpu_ms.max, 240);
        assert!(c.report(Side::G).contains("cpu        0.2s"));
        assert_eq!(c.failed.len(), 1);
        let (places, same) = &c.repeated["aa"];
        assert_eq!(places.len(), 2);
        assert!(!same);
        assert_eq!(Side::pick(None, &records).unwrap(), Side::G);
        assert_eq!(c.problems(Side::G), vec!["1 calls failed"]);
        assert_eq!(c.problems(Side::R).len(), 2);
        assert!(c.report(Side::G).contains("census of the G build fails"));
    }

    #[test]
    fn a_bad_line_is_named() {
        let err = parse("{}\n").unwrap_err();
        assert!(err.starts_with("line 1:"), "{err}");
    }
}
