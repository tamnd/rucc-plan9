//! `r9 baseline`: run the suites of plan 12 three times on G, and keep what G gives (plan 12.1).
//!
//! A suite prints one line for each test on the console:
//!
//! | line | result |
//! |---|---|
//! | `ok NAME` | `ok` |
//! | `FAIL NAME: reason` | `FAIL` |
//! | `value NAME TEXT` | `value TEXT` |
//!
//! The reason of a failure has pids and times in it, so only `ok` and `FAIL` are kept. A value is
//! kept exactly. The `boot` suite also gives the boot console, with the numbers taken out, and the
//! status of each command of `smoke/boot.rc`.
//!
//! A test is graded when each run gives the same result, and flaky when the runs differ or a run
//! does not give it. `r9 baseline` writes `baselines/<pin>/<objtype>/<suite>.json` with the hash of
//! each run, the graded results and the flaky ones.

use crate::boot;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The suites, with the command file of each in `smoke/`.
pub const SUITES: &[(&str, &str)] = &[
    ("boot", "boot.rc"),
    ("smoke", "suite.rc"),
    ("test", "test.rc"),
];

/// The results of one run of one suite, by test name.
pub type Results = BTreeMap<String, String>;

/// Read the result lines in `lines`. A name that comes twice keeps its last result.
#[must_use]
pub fn parse<S: AsRef<str>>(lines: &[S]) -> Results {
    let mut out = Results::new();
    for line in lines {
        let line = line.as_ref().trim_end();
        if let Some(name) = line.strip_prefix("ok ") {
            if !name.is_empty() && !name.contains(' ') {
                out.insert(name.to_string(), "ok".into());
            }
        } else if let Some(rest) = line.strip_prefix("FAIL ") {
            let name = rest.split(':').next().unwrap_or_default();
            if !name.is_empty() && !name.contains(' ') {
                out.insert(name.to_string(), "FAIL".into());
            }
        } else if let Some(rest) = line.strip_prefix("value ")
            && let Some((name, text)) = rest.split_once(' ')
        {
            out.insert(name.to_string(), format!("value {}", text.trim()));
        }
    }
    out
}

/// The boot console up to the first shell prompt, with the numbers taken out. Each run of digits
/// becomes `N`, so that times, addresses, pids and sizes do not differ between runs. A word of hex
/// digits with a digit in it, such as `669a5000`, also becomes `N`. A rate at the end of a line,
/// such as `(N.N KiB/s)`, is taken out. Blank lines and the lines of the memory size are left out.
///
/// The lines that identify the kernel are also left out, because a kernel of a different build
/// gives different values in them. These are the line of the loader that counts the kernel in
/// with backspaces, and the two lines that paqfs writes for the boot file system: the label with
/// the build date, such as `pc64.bootfs: Thu Jul 30 07:08:11 GMT 2026`, and the `fingerprint:`.
#[must_use]
pub fn normalize(console: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in console.split('\n') {
        let line = raw.replace('\r', "");
        if line.starts_with("term%") || line.starts_with("cpu%") {
            break;
        }
        let line = line.trim();
        if line.is_empty()
            || line.contains(" memory")
            || line.contains('\u{8}')
            || line.contains(".bootfs: ")
            || line.starts_with("fingerprint: ")
        {
            continue;
        }
        let mut n = String::new();
        let mut word = String::new();
        for c in line.chars() {
            if c.is_ascii_alphanumeric() {
                word.push(c);
            } else {
                numbers(&word, &mut n);
                word.clear();
                n.push(c);
            }
        }
        numbers(&word, &mut n);
        // U-Boot gives the rate of a load only when the load took some time.
        if n.ends_with("/s)")
            && let Some(i) = n.rfind(" (N")
        {
            n.truncate(i);
        }
        out.push(n);
    }
    out
}

/// Writes one word of the console to `out` with its numbers taken out. A word of hex digits with a
/// digit in it becomes `N`, and `0x` and hex digits becomes `NxN`. In other words each run of
/// digits becomes `N`.
fn numbers(word: &str, out: &mut String) {
    let hex = |w: &str| {
        !w.is_empty()
            && w.chars().all(|c| c.is_ascii_hexdigit())
            && w.chars().any(|c| c.is_ascii_digit())
    };
    if let Some(rest) = word.strip_prefix("0x").or_else(|| word.strip_prefix("0X"))
        && (rest.is_empty() || rest.chars().all(|c| c.is_ascii_hexdigit()))
    {
        out.push_str("NxN");
        return;
    }
    if hex(word) {
        out.push('N');
        return;
    }
    let mut digits = false;
    for c in word.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('N');
            }
            digits = true;
        } else {
            out.push(c);
            digits = false;
        }
    }
}

/// The SHA-256 of some text, in hex.
#[must_use]
pub fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// The hash of the results of a run.
#[must_use]
pub fn digest(results: &Results) -> String {
    let text = results.iter().fold(String::new(), |mut s, (k, v)| {
        let _ = writeln!(s, "{k}\t{v}");
        s
    });
    sha256(&text)
}

/// What one run of a suite gave.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// The results by test name.
    pub results: Results,
    /// The time of the boot.
    pub seconds: f64,
    /// Whether every command succeeded and the guest halted.
    pub passed: bool,
}

/// The baseline of one suite on one row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Baseline {
    /// The tests that gave the same result in each run.
    pub graded: Results,
    /// The tests whose runs differ, with the result of each run. A run that did not give the test
    /// has `missing`.
    pub flaky: BTreeMap<String, Vec<String>>,
}

/// Put the runs of one suite together.
#[must_use]
pub fn combine(runs: &[Run]) -> Baseline {
    let mut names: Vec<&String> = runs.iter().flat_map(|r| r.results.keys()).collect();
    names.sort();
    names.dedup();
    let mut b = Baseline::default();
    for name in names {
        let each: Vec<String> = runs
            .iter()
            .map(|r| {
                r.results
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| "missing".into())
            })
            .collect();
        if each.windows(2).all(|w| w[0] == w[1]) {
            b.graded.insert(name.clone(), each[0].clone());
        } else {
            b.flaky.insert(name.clone(), each);
        }
    }
    b
}

/// The JSON file of a baseline.
#[must_use]
pub fn to_json(pin: &str, objtype: &str, suite: &str, runs: &[Run], b: &Baseline) -> String {
    let runs: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            serde_json::json!({
                "sha256": digest(&r.results),
                "tests": r.results.len(),
                "seconds": (r.seconds * 10.0).round() / 10.0,
                "passed": r.passed,
            })
        })
        .collect();
    let failed = b.graded.values().filter(|v| *v == "FAIL").count();
    let value = serde_json::json!({
        "pin": pin,
        "objtype": objtype,
        "suite": suite,
        "runs": runs,
        "graded": b.graded.len(),
        "graded_fail": failed,
        "flaky": b.flaky.len(),
        "results": b.graded,
        "flaky_results": b.flaky,
    });
    serde_json::to_string_pretty(&value).unwrap_or_default() + "\n"
}

/// The results of a boot of one suite.
#[must_use]
pub fn results_of(suite: &str, outcome: &boot::Outcome, console: &str) -> Results {
    let mut lines: Vec<&str> = Vec::new();
    for s in &outcome.steps {
        lines.extend(s.output.iter().map(String::as_str));
    }
    let mut results = parse(&lines);
    if suite == "boot" {
        let normal = normalize(console).join("\n");
        results.insert("console".into(), format!("value {}", sha256(&normal)));
        for (i, s) in outcome.steps.iter().enumerate() {
            let r = if s.status.as_deref() == Some("") {
                "ok"
            } else {
                "FAIL"
            };
            results.insert(format!("step-{}", i + 1), r.into());
        }
    }
    results.insert(
        "halted".into(),
        if outcome.phase == crate::console::Phase::Halted && !outcome.timed_out {
            "ok".into()
        } else {
            "FAIL".into()
        },
    );
    results
}

/// The pin of an image: the commit in the name of a `g-base` image, or the file name up to the
/// first dot. The image that `r9 build --keep` leaves is `root.qcow2`, and its `build.toml` names
/// the image that the build started from. The pin of such an image is the pin of that image.
#[must_use]
pub fn pin_of(image: &Path) -> String {
    let mut image = image.to_path_buf();
    // A build can start from the image of an earlier build, so follow a short chain.
    for _ in 0..8 {
        let Some(base) = base_of(&image) else { break };
        image = base;
    }
    let name = image
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or_default();
    let stem = name.split('.').next().unwrap_or_default();
    stem.strip_prefix("g-base-").unwrap_or(stem).to_string()
}

/// The image that the build of `image` started from, from the `build.toml` next to it.
fn base_of(image: &Path) -> Option<std::path::PathBuf> {
    let text = std::fs::read_to_string(image.parent()?.join("build.toml")).ok()?;
    let table: toml::Table = text.parse().ok()?;
    let base = std::path::PathBuf::from(table.get("image")?.as_str()?);
    (base != image).then_some(base)
}

/// Boot the image once with the commands of one suite, and read the results.
#[allow(clippy::too_many_arguments)]
fn boot_once(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
    objtype: &str,
    image: &Path,
    pin: &str,
    suite: &str,
    file: &str,
    n: usize,
) -> Result<Run, String> {
    let mut a = args.clone();
    let work = repo
        .root
        .join("work")
        .join("baseline")
        .join(pin)
        .join(objtype)
        .join(format!("{suite}-{n}"));
    let path = |p: PathBuf| p.display().to_string();
    a.values.insert("row".into(), objtype.to_string());
    a.values.insert("image".into(), path(image.to_path_buf()));
    a.values
        .insert("run".into(), path(repo.root.join("smoke").join(file)));
    a.values.insert("in".into(), path(repo.root.join("guest")));
    a.values.insert("out".into(), path(work.clone()));
    let req = boot::request_from(repo, &a)?;
    let outcome = boot::run(&req)?;
    let console = std::fs::read_to_string(work.join("console.log")).unwrap_or_default();
    Ok(Run {
        results: results_of(suite, &outcome, &console),
        seconds: outcome.seconds,
        passed: outcome.passed,
    })
}

/// Run `r9 baseline`. Each row in `--row` runs each suite `--runs` times, from `--image` or else
/// from the `g-base` image of the row. The work goes to `work/baseline`, and the files to
/// `baselines/`. The baselines in the repository come from the G build of the pin, given with
/// `--image`, because the programs in `g-base` that are not tools or libraries are those of the
/// release (`baselines/README.md`).
pub fn command(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
) -> Result<(bool, String), String> {
    let rows: Vec<String> = args
        .get("row")
        .unwrap_or("amd64,arm64")
        .split(',')
        .map(str::to_string)
        .collect();
    let count: usize = args
        .get("runs")
        .map_or(Ok(3), str::parse)
        .map_err(|e| format!("--runs: {e}"))?;
    if count == 0 {
        return Err("--runs must be 1 or more".into());
    }
    let wanted: Vec<&str> = args.get("suites").map_or_else(
        || SUITES.iter().map(|s| s.0).collect(),
        |s| s.split(',').collect(),
    );
    for w in &wanted {
        if !SUITES.iter().any(|s| s.0 == *w) {
            return Err(format!("--suites: there is no suite {w}"));
        }
    }
    let out_dir = args
        .get("out")
        .map_or_else(|| repo.root.join("baselines"), PathBuf::from);
    let mut report = String::new();
    let mut all_ok = true;
    for objtype in &rows {
        let image = match args.get("image") {
            Some(i) => PathBuf::from(i),
            None => crate::image::g_base(repo, objtype).ok_or_else(|| {
                format!(
                    "there is no g-base image for {objtype}; run r9 image build --row {objtype}"
                )
            })?,
        };
        let pin = args
            .get("pin")
            .map_or_else(|| pin_of(&image), str::to_string);
        for suite in &wanted {
            let file = SUITES
                .iter()
                .find(|s| s.0 == *suite)
                .map(|s| s.1)
                .unwrap_or_default();
            let mut runs = Vec::new();
            for n in 1..=count {
                let run = boot_once(repo, args, objtype, &image, &pin, suite, file, n)?;
                let fails = run.results.values().filter(|v| *v == "FAIL").count();
                let line = format!(
                    "{objtype} {suite} run {n}: {} results, {fails} FAIL, {:.1}s, {}",
                    run.results.len(),
                    run.seconds,
                    digest(&run.results)
                );
                eprintln!("r9 baseline: {line}");
                let _ = writeln!(report, "{line}");
                runs.push(run);
            }
            let b = combine(&runs);
            let dir = out_dir.join(&pin).join(objtype);
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = dir.join(format!("{suite}.json"));
            std::fs::write(&path, to_json(&pin, objtype, suite, &runs, &b))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            let failed = b.graded.values().filter(|v| *v == "FAIL").count();
            let _ = writeln!(
                report,
                "{objtype} {suite}: {} graded, {failed} of them FAIL, {} flaky; {}",
                b.graded.len(),
                b.flaky.len(),
                path.display()
            );
            for (name, each) in &b.flaky {
                let _ = writeln!(report, "  flaky {name}: {}", each.join(" | "));
            }
            all_ok &= runs
                .iter()
                .all(|r| r.results.get("halted").is_some_and(|h| h == "ok"));
        }
    }
    Ok((all_ok, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_lines_are_read() {
        let r = parse(&[
            "ok rfork-n",
            "FAIL dial: dial 4123: connection refused",
            "value libc/test/pow.out c86cba15",
            "smoke: 1 ok, 1 FAIL",
            "ok",
            "some other line",
        ]);
        assert_eq!(r.len(), 3);
        assert_eq!(r["rfork-n"], "ok");
        assert_eq!(r["dial"], "FAIL");
        assert_eq!(r["libc/test/pow.out"], "value c86cba15");
    }

    #[test]
    fn the_console_loses_its_numbers() {
        let c = "\r\nPlan 9\r\n9 bytes read in 2 ms (4.4 KiB/s)\r\n126 holes free\r\n2048M memory: 1024M kernel\r\nE820: 0x0000 1234\r\n\r\nterm% ls\r\nafter\r\n";
        assert_eq!(
            normalize(c),
            vec!["Plan N", "N bytes read in N ms", "N holes free", "N: NxN N"]
        );
    }

    #[test]
    fn the_console_loses_the_identity_of_the_kernel() {
        let a = "\r\n \u{8}1\u{8}2\u{8}\r\n0x0060e000 0x669a5000 1715040256\r\npc64.bootfs: Thu Jul 30 07:08:11 GMT 2026\r\nfingerprint: 0368af54765999c0feb827d1038ac2c575379162\r\ncpu3: AX 00000663 CX F6F8320B\r\nsdE0: ich\r\ncpu% ";
        let b = "\r\n \u{8}1\u{8}2\u{8}3\u{8}\r\n0x0062d000 0x669b8000 1714991104\r\npc64.bootfs: Wed Nov 11 14:27:57 GMT 2026\r\nfingerprint: 1de86c4be9d7451ea3264a0b0cac80356c14ecfb\r\ncpu3: AX 00000663 CX F6F8320B\r\nsdE0: ich\r\ncpu% ";
        assert_eq!(
            normalize(a),
            vec!["NxN NxN N", "cpuN: AX N CX N", "sdEN: ich"]
        );
        assert_eq!(normalize(a), normalize(b));
    }

    fn run(pairs: &[(&str, &str)]) -> Run {
        Run {
            results: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            seconds: 1.0,
            passed: true,
        }
    }

    #[test]
    fn a_test_that_differs_is_flaky() {
        let runs = [
            run(&[("a", "ok"), ("b", "ok"), ("c", "FAIL")]),
            run(&[("a", "ok"), ("b", "FAIL"), ("c", "FAIL")]),
            run(&[("a", "ok"), ("c", "FAIL")]),
        ];
        let b = combine(&runs);
        assert_eq!(b.graded.len(), 2);
        assert_eq!(b.graded["c"], "FAIL");
        assert_eq!(b.flaky["b"], vec!["ok", "FAIL", "missing"]);
    }

    #[test]
    fn the_pin_comes_from_the_image_name() {
        assert_eq!(
            pin_of(Path::new("/c/images/g-base-2859ae04e800.arm64.qcow2")),
            "2859ae04e800"
        );
        assert_eq!(
            pin_of(Path::new("9front-11952.amd64.qcow2")),
            "9front-11952"
        );
    }

    #[test]
    fn the_pin_of_a_build_is_the_pin_of_its_base() {
        let dir = std::env::temp_dir().join(format!("r9-pin-{}", std::process::id()));
        let g = dir.join("g");
        std::fs::create_dir_all(&g).unwrap();
        std::fs::write(
            g.join("build.toml"),
            "side = \"G\"\nimage = \"/c/images/g-base-2859ae04e800.amd64.qcow2\"\n",
        )
        .unwrap();
        let k = dir.join("k");
        std::fs::create_dir_all(&k).unwrap();
        let root = g.join("root.qcow2");
        std::fs::write(
            k.join("build.toml"),
            format!("image = {:?}\n", root.display().to_string()),
        )
        .unwrap();
        assert_eq!(pin_of(&root), "2859ae04e800");
        assert_eq!(pin_of(&k.join("root.qcow2")), "2859ae04e800");
        assert_eq!(pin_of(&dir.join("root.qcow2")), "root");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_digest_does_not_depend_on_the_order() {
        let a = run(&[("a", "ok"), ("b", "FAIL")]);
        let b = run(&[("b", "FAIL"), ("a", "ok")]);
        assert_eq!(digest(&a.results), digest(&b.results));
    }
}
