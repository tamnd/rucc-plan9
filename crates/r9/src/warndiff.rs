//! `r9 warn-diff`: the warnings that G and R give for each unit (plan 05.8).
//!
//! The tree builds with `-w`, and a warning never fails a build. So this is a report and never a
//! gate. For each unit that both builds compiled, it lists the warnings that only G gives, which
//! is lost checking, and the warnings that only R gives, which is noise. The `varargck` format
//! warnings are the class that P3 drives to parity (06.6), so the report counts them apart.
//!
//! kencc writes a warning as `warning: FILE:LINE MESSAGE`. When the line is in an included file,
//! the place is `FILE:LINE[FILE:LINE]`, and the places of the files that include it follow. rucc
//! writes `FILE:LINE:COLUMN: warning: MESSAGE [CODE]`. A warning is compared by the base name of
//! its file, its line and its message.

use crate::census::Record;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// One warning: the base name of the file, the line and the message.
pub type Warning = (String, u32, String);

/// The key of a unit: the compiler name, the directory and the sources.
pub type Key = (String, String, String);

/// The warnings of one unit that only one build gives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnitDiff {
    /// The warnings that only G gives.
    pub lost: Vec<Warning>,
    /// The warnings that only R gives.
    pub added: Vec<Warning>,
}

/// The result of the comparison.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WarnDiff {
    /// The number of units that both builds compiled.
    pub units: usize,
    /// The number of warnings that both builds give.
    pub same: usize,
    /// The units with a difference.
    pub diffs: BTreeMap<Key, UnitDiff>,
}

/// The base name of a path.
fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether `word` is a kencc place, `FILE:LINE` or `FILE:LINE[FILE:LINE]`.
fn place(word: &str) -> Option<(String, u32)> {
    let first = word.split('[').next().unwrap_or(word);
    let (file, line) = first.rsplit_once(':')?;
    if file.is_empty() || word.contains('[') && !word.ends_with(']') {
        return None;
    }
    Some((base(file).to_owned(), line.parse().ok()?))
}

/// Read one line of standard error as a warning, or `None` when it is not one.
#[must_use]
pub fn parse_line(line: &str) -> Option<Warning> {
    let line = line.trim_end();
    if let Some(rest) = line.strip_prefix("warning: ") {
        // kencc: the first place is the place of the warning, and the places of the files that
        // include it follow.
        let mut words = rest.split(' ');
        let (file, n) = place(words.next()?)?;
        let mut message: Vec<&str> = words.collect();
        while message.first().is_some_and(|w| place(w).is_some()) {
            message.remove(0);
        }
        return Some((file, n, message.join(" ")));
    }
    // rucc: FILE:LINE:COLUMN: warning: MESSAGE [CODE]
    let (head, message) = line.split_once(": warning: ")?;
    let mut parts = head.rsplitn(3, ':');
    let _column = parts.next()?;
    let n: u32 = parts.next()?.parse().ok()?;
    let file = parts.next()?;
    let message = match message.rsplit_once(" [") {
        Some((m, code)) if code.ends_with(']') && code.starts_with('E') => m,
        _ => message,
    };
    Some((base(file).to_owned(), n, message.to_owned()))
}

/// The key of one record.
fn key(r: &Record) -> Key {
    let sources = if r.stdin || r.inputs.is_empty() {
        format!("stdin {}", r.argv.join(" "))
    } else {
        r.inputs
            .iter()
            .map(|f| f.file.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    (r.cc.clone(), r.dir.clone(), sources)
}

/// The warnings of each unit. A unit compiled more than once has the warnings of each call.
fn units(records: &[Record]) -> BTreeMap<Key, BTreeSet<Warning>> {
    let mut map: BTreeMap<Key, BTreeSet<Warning>> = BTreeMap::new();
    for r in records {
        let set = map.entry(key(r)).or_default();
        set.extend(r.stderr.lines().filter_map(parse_line));
    }
    map
}

/// Compare the warnings of the G build with those of the R build.
#[must_use]
pub fn diff(g: &[Record], r: &[Record]) -> WarnDiff {
    let (g, r) = (units(g), units(r));
    let mut d = WarnDiff::default();
    for (k, gw) in &g {
        let Some(rw) = r.get(k) else { continue };
        d.units += 1;
        d.same += gw.intersection(rw).count();
        let lost: Vec<Warning> = gw.difference(rw).cloned().collect();
        let added: Vec<Warning> = rw.difference(gw).cloned().collect();
        if !lost.is_empty() || !added.is_empty() {
            d.diffs.insert(k.clone(), UnitDiff { lost, added });
        }
    }
    d
}

/// Whether a message is one of the `varargck` format warnings of kencc's `dpcheck`.
#[must_use]
pub fn is_format(message: &str) -> bool {
    [
        "format mismatch",
        "more arguments than format",
        "more format than arguments",
        "unknown format",
        "cant find format arg",
        "format arg type",
        "nil format arg",
    ]
    .iter()
    .any(|p| message.starts_with(p))
        || message.ends_with("newprot: type not defined")
}

/// The class of a message: its words up to the first one with a digit, a quote or a `%`.
#[must_use]
pub fn class(message: &str) -> String {
    let words: Vec<&str> = message
        .split(' ')
        .take_while(|w| !w.chars().any(|c| c.is_ascii_digit() || "\"'%`".contains(c)))
        .take(4)
        .collect();
    if words.is_empty() {
        message.split(' ').next().unwrap_or_default().to_owned()
    } else {
        words.join(" ")
    }
}

/// The text of the report, with up to `top` units.
#[must_use]
pub fn report(d: &WarnDiff, top: usize) -> String {
    let mut out = String::new();
    let lost: usize = d.diffs.values().map(|u| u.lost.len()).sum();
    let added: usize = d.diffs.values().map(|u| u.added.len()).sum();
    let count = |f: &dyn Fn(&UnitDiff) -> usize| d.diffs.values().map(f).sum::<usize>();
    let lost_format = count(&|u| u.lost.iter().filter(|w| is_format(&w.2)).count());
    let added_format = count(&|u| u.added.iter().filter(|w| is_format(&w.2)).count());
    let _ = writeln!(
        out,
        "warn-diff: {} units in both builds, {} warnings the same, {lost} only in G, {added} only in R",
        d.units, d.same
    );
    let _ = writeln!(
        out,
        "format warnings (06.6): {lost_format} only in G, {added_format} only in R"
    );
    let mut classes: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for u in d.diffs.values() {
        for w in &u.lost {
            classes.entry(class(&w.2)).or_default().0 += 1;
        }
        for w in &u.added {
            classes.entry(class(&w.2)).or_default().1 += 1;
        }
    }
    let mut ranked: Vec<_> = classes.into_iter().collect();
    ranked.sort_by(|a, b| (b.1.0 + b.1.1).cmp(&(a.1.0 + a.1.1)).then(a.0.cmp(&b.0)));
    if !ranked.is_empty() {
        let _ = writeln!(out, "by class (only in G, only in R):");
    }
    for (c, (l, a)) in ranked.iter().take(top) {
        let _ = writeln!(out, "  {l:6} {a:6}  {c}");
    }
    for ((cc, dir, src), u) in d.diffs.iter().take(top) {
        let _ = writeln!(out, "{cc} {dir} {src}");
        for (f, n, m) in &u.lost {
            let _ = writeln!(out, "  G only: {f}:{n} {m}");
        }
        for (f, n, m) in &u.added {
            let _ = writeln!(out, "  R only: {f}:{n} {m}");
        }
    }
    if d.diffs.len() > top {
        let _ = writeln!(out, "and {} more units", d.diffs.len() - top);
    }
    out
}

/// The records of a build: `DIR/collected/compile.jsonl`, or the file itself.
fn records(path: &str) -> Result<Vec<Record>, String> {
    let p = std::path::Path::new(path);
    let file = if p.is_dir() {
        p.join("collected").join("compile.jsonl")
    } else {
        p.to_path_buf()
    };
    crate::census::load(&file)
}

/// Run `r9 warn-diff G --with R [--top N]`, and give the report.
pub fn command(args: &crate::cli::Args) -> Result<String, String> {
    let g = args
        .target
        .as_deref()
        .ok_or("give the directory of the G build")?;
    let r = args
        .get("with")
        .ok_or("give --with, the directory of the R build")?;
    let top = args
        .get("top")
        .map_or(Ok(20), str::parse)
        .map_err(|e| format!("--top: {e}"))?;
    Ok(report(&diff(&records(g)?, &records(r)?), top))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warnings_of_kencc_are_read() {
        assert_eq!(
            parse_line("warning: x.c:12 format mismatch d LONG, arg 2"),
            Some(("x.c".into(), 12, "format mismatch d LONG, arg 2".into()))
        );
        assert_eq!(
            parse_line("warning: /sys/include/libc.h:30[x.c:3] x.c:1 unused variable: p"),
            Some(("libc.h".into(), 30, "unused variable: p".into()))
        );
        assert_eq!(parse_line("x.c:12 syntax error"), None);
    }

    #[test]
    fn the_warnings_of_rucc_are_read() {
        assert_eq!(
            parse_line("/sys/src/cmd/x.c:5:24: warning: format mismatch d LONG, arg 2 [E0884]"),
            Some(("x.c".into(), 5, "format mismatch d LONG, arg 2".into()))
        );
        assert_eq!(parse_line("x.c:5:24: error: no [E0001]"), None);
    }

    fn rec(dir: &str, src: &str, stderr: &str) -> Record {
        let line = format!(
            "{{\"cc\":\"6c\",\"objtype\":\"amd64\",\"dir\":\"{dir}\",\"argv\":[\"-FTVw\",\"{src}\"],\"in\":[{{\"file\":\"{src}\",\"sha256\":\"\"}}],\"out\":[],\"stdin\":false,\"status\":\"\",\"ms\":1,\"origin\":\"rucc\",\"stderr\":{}}}",
            serde_json::to_string(stderr).unwrap()
        );
        crate::census::parse(&line).unwrap().remove(0)
    }

    #[test]
    fn the_units_are_compared() {
        let g = vec![
            rec(
                "/a",
                "x.c",
                "warning: x.c:3 format mismatch d LONG, arg 2\nwarning: x.c:9 used and not set: q\n",
            ),
            rec("/a", "y.c", "warning: y.c:1 unused variable: z\n"),
        ];
        let r = vec![
            rec(
                "/a",
                "x.c",
                "x.c:3:7: warning: format mismatch d LONG, arg 2 [E0884]\nx.c:4:1: warning: no return at end of function [E0800]\n",
            ),
            rec(
                "/a",
                "y.c",
                "y.c:1:5: warning: unused variable: z [E0801]\n",
            ),
        ];
        let d = diff(&g, &r);
        assert_eq!(d.units, 2);
        assert_eq!(d.same, 2);
        assert_eq!(d.diffs.len(), 1);
        let u = &d.diffs[&("6c".into(), "/a".into(), "x.c".into())];
        assert_eq!(
            u.lost,
            vec![("x.c".into(), 9, "used and not set: q".into())]
        );
        assert_eq!(
            u.added,
            vec![("x.c".into(), 4, "no return at end of function".into())]
        );
        let text = report(&d, 10);
        assert!(text.starts_with("warn-diff: 2 units in both builds, 2 warnings the same, 1 only in G, 1 only in R\nformat warnings (06.6): 0 only in G, 0 only in R\n"), "{text}");
        assert!(
            text.contains("  G only: x.c:9 used and not set: q"),
            "{text}"
        );
    }

    #[test]
    fn a_class_stops_at_a_name_or_a_number() {
        assert_eq!(
            class("format mismatch d LONG, arg 2"),
            "format mismatch d LONG,"
        );
        assert_eq!(class("used and not set: q"), "used and not set:");
        assert_eq!(class("1 is odd"), "1");
        assert!(is_format("more arguments than format INT"));
        assert!(!is_format("unused variable: z"));
    }
}
