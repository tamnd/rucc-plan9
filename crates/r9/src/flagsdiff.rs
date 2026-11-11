//! `r9 flags-diff`: whether two builds called the compiler slot with the same arguments (plan
//! 05.5).
//!
//! kencc has no identity that a build can ask about, so two builds are configured the same when
//! each unit got the same arguments. A unit is the compiler name, the directory, and the names
//! of the sources. A call that read its source on standard input has no source names, and its
//! arguments are a part of its key. A unit can be compiled more than once in a build, and the
//! number of times depends on what mk found up to date, so each key has the set of the distinct
//! arguments of its calls. The time that a mkfile puts in `-DDATE=`, such as the one of cwfs, is
//! not compared.

use crate::census::Record;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The key of a unit: the compiler name, the directory, and the sources.
type Key = (String, String, String);

/// What two builds differ in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlagsDiff {
    /// The number of units in both builds with the same arguments.
    pub same: usize,
    /// The units in both builds with other arguments, with the arguments of each build.
    pub changed: Vec<(Key, Vec<String>, Vec<String>)>,
    /// The units that only the first build has.
    pub only_a: Vec<Key>,
    /// The units that only the second build has.
    pub only_b: Vec<Key>,
}

impl FlagsDiff {
    /// Whether the two builds called the compiler slot in the same way.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.only_a.is_empty() && self.only_b.is_empty()
    }
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

/// An argument with the digits of a `-DDATE=` taken out.
fn without_date(arg: &str) -> String {
    match arg.strip_prefix("-DDATE=") {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => "-DDATE=N".to_owned(),
        _ => arg.to_owned(),
    }
}

/// The distinct arguments of each unit, by key.
fn units(records: &[Record]) -> BTreeMap<Key, Vec<String>> {
    let mut map: BTreeMap<Key, Vec<String>> = BTreeMap::new();
    for r in records {
        let argv: Vec<String> = r.argv.iter().map(|a| without_date(a)).collect();
        map.entry(key(r)).or_default().push(argv.join(" "));
    }
    for calls in map.values_mut() {
        calls.sort();
        calls.dedup();
    }
    map
}

/// Compare the calls of two builds.
#[must_use]
pub fn diff(a: &[Record], b: &[Record]) -> FlagsDiff {
    let (a, b) = (units(a), units(b));
    let mut d = FlagsDiff::default();
    for (k, calls) in &a {
        match b.get(k) {
            None => d.only_a.push(k.clone()),
            Some(other) if other == calls => d.same += 1,
            Some(other) => d.changed.push((k.clone(), calls.clone(), other.clone())),
        }
    }
    d.only_b = b.keys().filter(|k| !a.contains_key(*k)).cloned().collect();
    d
}

/// The text of a unit key.
fn unit(k: &Key) -> String {
    format!("{} {} {}", k.0, k.1, k.2)
}

/// The report, with at most `top` units in each group.
#[must_use]
pub fn report(d: &FlagsDiff, top: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} units with the same arguments, {} with other arguments, {} only in the first build, {} only in the second",
        d.same,
        d.changed.len(),
        d.only_a.len(),
        d.only_b.len()
    );
    if !d.changed.is_empty() {
        let _ = writeln!(out, "other arguments:");
    }
    for (k, a, b) in d.changed.iter().take(top) {
        let _ = writeln!(out, "  {}", unit(k));
        let _ = writeln!(out, "    first:  {}", a.join(" | "));
        let _ = writeln!(out, "    second: {}", b.join(" | "));
    }
    for (title, keys) in [
        ("only in the first build:", &d.only_a),
        ("only in the second build:", &d.only_b),
    ] {
        if !keys.is_empty() {
            let _ = writeln!(out, "{title}");
        }
        for k in keys.iter().take(top) {
            let _ = writeln!(out, "  {}", unit(k));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(dir: &str, argv: &[&str], source: &str) -> Record {
        Record {
            cc: "6c".into(),
            objtype: "amd64".into(),
            dir: dir.into(),
            argv: argv.iter().map(ToString::to_string).collect(),
            inputs: vec![crate::census::File {
                file: source.into(),
                sha256: "aa".into(),
            }],
            outputs: Vec::new(),
            stdin: false,
            status: String::new(),
            ms: 1,
            user_ms: 0,
            sys_ms: 0,
            origin: "rucc".into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn the_same_calls_are_the_same() {
        let a = [record("/sys/src/cmd", &["-FTVw", "cat.c"], "cat.c")];
        let d = diff(&a, &a);
        assert!(d.is_empty());
        assert_eq!(d.same, 1);
    }

    #[test]
    fn other_arguments_and_other_units_are_found() {
        let a = [
            record("/sys/src/cmd", &["-FTVw", "cat.c"], "cat.c"),
            record("/sys/src/cmd", &["-FTVw", "ls.c"], "ls.c"),
        ];
        let b = [
            record("/sys/src/cmd", &["-FTw", "cat.c"], "cat.c"),
            record("/sys/src/cmd", &["-FTVw", "wc.c"], "wc.c"),
        ];
        let d = diff(&a, &b);
        assert_eq!(d.same, 0);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.only_a.len(), 1);
        assert_eq!(d.only_b.len(), 1);
        let text = report(&d, 20);
        assert!(text.contains("first:  -FTVw cat.c"), "{text}");
        assert!(text.contains("second: -FTw cat.c"), "{text}");
        assert!(text.contains("6c /sys/src/cmd wc.c"), "{text}");
    }

    #[test]
    fn a_unit_compiled_twice_is_compared_as_a_set() {
        let one = record("/sys/src/libc", &["-FTVw", "a.c"], "a.c");
        let two = record("/sys/src/libc", &["-FTVw", "-p", "a.c"], "a.c");
        let d = diff(
            &[one.clone(), two.clone(), one.clone()],
            &[two.clone(), one],
        );
        assert!(d.is_empty(), "{d:?}");
        let d = diff(
            std::slice::from_ref(&two),
            &[two.clone(), record("/sys/src/libc", &["a.c"], "a.c")],
        );
        assert_eq!(d.changed.len(), 1);
    }

    #[test]
    fn the_date_of_a_mkfile_is_not_compared() {
        let a = [record(
            "/sys/src/cmd/cwfs",
            &["-DDATE=1794358430", "conf.c"],
            "conf.c",
        )];
        let b = [record(
            "/sys/src/cmd/cwfs",
            &["-DDATE=1794300108", "conf.c"],
            "conf.c",
        )];
        assert!(diff(&a, &b).is_empty());
        let c = [record(
            "/sys/src/cmd/cwfs",
            &["-DDATE=x", "conf.c"],
            "conf.c",
        )];
        assert!(!diff(&a, &c).is_empty());
    }
}
