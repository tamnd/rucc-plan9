//! `r9 diffrun`: run the programs of plan 12.5 on fixed inputs under G's user space and R's, and
//! compare what they write (plan 12.5).
//!
//! `guest/r9-diffrun` runs each case and prints `value NAME HASH`, with the SHA-1 of the output of
//! the case, or `FAIL NAME: reason`. The cases and their inputs are in `diffrun/`. A case is a
//! short rc script, and its input is a file of `diffrun/inputs` or a file of the tree.
//!
//! The G side is the `g-base` image of the row. It runs `--runs` times (3 by default), and a case
//! is in the comparison only when each G run gives the same value (plan 12.1). The R side is the
//! image that `r9 build --side R --keep` left, and it runs once. A case passes when the R value is
//! the same as the G value.
//!
//! The hash comes from the `sha1sum` of the image. On the R side it is the `sha1sum` of rucc, so
//! the `sha1sum` case is the check of the hash itself.

use crate::baseline::{self, Results};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The result of one case in the comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// G and R give the same value.
    Same,
    /// G and R give different values.
    Differs {
        /// What G gave.
        g: String,
        /// What R gave.
        r: String,
    },
    /// The G runs do not agree, so the case is not compared.
    Flaky,
    /// G fails the case, so it is not compared.
    GFails,
}

/// Compare the R results with the G runs.
#[must_use]
pub fn compare(g_runs: &[Results], r: &Results) -> Vec<(String, Verdict)> {
    let runs: Vec<baseline::Run> = g_runs
        .iter()
        .map(|results| baseline::Run {
            results: results.clone(),
            seconds: 0.0,
            passed: true,
        })
        .collect();
    let b = baseline::combine(&runs);
    let mut out = Vec::new();
    for (name, g) in &b.graded {
        if name == "halted" {
            continue;
        }
        let verdict = if g == "FAIL" {
            Verdict::GFails
        } else {
            let r = r.get(name).cloned().unwrap_or_else(|| "missing".into());
            if &r == g {
                Verdict::Same
            } else {
                Verdict::Differs { g: g.clone(), r }
            }
        };
        out.push((name.clone(), verdict));
    }
    for name in b.flaky.keys() {
        out.push((name.clone(), Verdict::Flaky));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The text of the comparison, and whether each compared case gives the same value.
#[must_use]
pub fn report(verdicts: &[(String, Verdict)]) -> (bool, String) {
    let mut text = String::new();
    let count = |f: fn(&Verdict) -> bool| verdicts.iter().filter(|(_, v)| f(v)).count();
    let same = count(|v| *v == Verdict::Same);
    let differs = count(|v| matches!(v, Verdict::Differs { .. }));
    let flaky = count(|v| *v == Verdict::Flaky);
    let gfails = count(|v| *v == Verdict::GFails);
    for (name, v) in verdicts {
        let line = match v {
            Verdict::Same => format!("same {name}"),
            Verdict::Differs { g, r } => format!("DIFF {name}: G {g}, R {r}"),
            Verdict::Flaky => format!("flaky {name}: the G runs differ, so it is not compared"),
            Verdict::GFails => format!("gfail {name}: G fails it, so it is not compared"),
        };
        let _ = writeln!(text, "{line}");
    }
    let _ = writeln!(
        text,
        "diffrun: {same} same, {differs} differ, {flaky} flaky on G, {gfails} fail on G"
    );
    (differs == 0 && same > 0, text)
}

/// Boot `image` once with the diffrun suite and read the results.
fn boot_once(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
    objtype: &str,
    image: &Path,
    work: &Path,
) -> Result<Results, String> {
    let mut a = args.clone();
    let path = |p: &Path| p.display().to_string();
    a.values.insert("row".into(), objtype.to_string());
    a.values.insert("image".into(), path(image));
    let run = repo.root.join("smoke").join("diffrun.rc");
    a.values.insert("run".into(), path(&run));
    a.values.insert("in".into(), path(&stage(repo, work)?));
    a.values.insert("out".into(), path(work));
    a.values
        .entry("timeout".into())
        .or_insert_with(|| "3600".into());
    let req = crate::boot::request_from(repo, &a)?;
    let outcome = crate::boot::run(&req)?;
    let console = std::fs::read_to_string(work.join("console.log")).unwrap_or_default();
    Ok(baseline::results_of("diffrun", &outcome, &console))
}

/// Put `guest/r9-diffrun` and `diffrun/` together in one directory for the tar disk.
fn stage(repo: &crate::repo::Repo, work: &Path) -> Result<PathBuf, String> {
    let dir = work.with_extension("in");
    let _ = std::fs::remove_dir_all(&dir);
    copy_tree(&repo.root.join("diffrun"), &dir.join("diffrun"))?;
    let script = repo.root.join("guest").join("r9-diffrun");
    std::fs::copy(&script, dir.join("r9-diffrun"))
        .map_err(|e| format!("{}: {e}", script.display()))?;
    Ok(dir)
}

/// Copy the files under `from` to `to`.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", from.display()))?;
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Run `r9 diffrun`. `--r FILE` is the R image. `--g FILE` is the G image, and the default is the
/// `g-base` image of the row. The work goes to `work/diffrun` or `--out`.
pub fn command(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
) -> Result<(bool, String), String> {
    let objtype = args.get("row").unwrap_or("amd64").to_string();
    let r_image = PathBuf::from(
        args.get("r")
            .ok_or("r9 diffrun needs --r FILE, the image of an R build")?,
    );
    let g_image = match args.get("g") {
        Some(g) => PathBuf::from(g),
        None => crate::image::g_base(repo, &objtype).ok_or_else(|| {
            format!("there is no g-base image for {objtype}; run r9 image build --row {objtype}")
        })?,
    };
    let count: usize = args
        .get("runs")
        .map_or(Ok(3), str::parse)
        .map_err(|e| format!("--runs: {e}"))?;
    if count == 0 {
        return Err("--runs must be 1 or more".into());
    }
    let out = args
        .get("out")
        .map_or_else(|| repo.root.join("work").join("diffrun"), PathBuf::from);
    let mut runs = Vec::new();
    for n in 1..=count {
        let results = boot_once(repo, args, &objtype, &g_image, &out.join(format!("g-{n}")))?;
        eprintln!(
            "r9 diffrun: G run {n}: {} results, {}",
            results.len(),
            baseline::digest(&results)
        );
        runs.push(results);
    }
    let r = boot_once(repo, args, &objtype, &r_image, &out.join("r"))?;
    eprintln!(
        "r9 diffrun: R run: {} results, {}",
        r.len(),
        baseline::digest(&r)
    );
    let verdicts = compare(&runs, &r);
    let (ok, text) = report(&verdicts);
    let path = out.join("diffrun.txt");
    std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((ok, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn results(pairs: &[(&str, &str)]) -> Results {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn a_case_is_compared_only_when_g_agrees_with_itself() {
        let g1 = results(&[("awk", "value a1"), ("date", "value d1"), ("gs", "FAIL")]);
        let g2 = results(&[("awk", "value a1"), ("date", "value d2"), ("gs", "FAIL")]);
        let r = results(&[("awk", "value a1"), ("date", "value d1"), ("gs", "value g")]);
        let v = compare(&[g1, g2], &r);
        assert_eq!(
            v,
            vec![
                ("awk".into(), Verdict::Same),
                ("date".into(), Verdict::Flaky),
                ("gs".into(), Verdict::GFails),
            ]
        );
        assert!(report(&v).0);
    }

    #[test]
    fn a_different_or_missing_value_is_a_difference() {
        let g = results(&[("bc", "value b1"), ("dc", "value d1"), ("halted", "ok")]);
        let r = results(&[("bc", "value b2"), ("halted", "FAIL")]);
        let v = compare(std::slice::from_ref(&g), &r);
        assert_eq!(v.len(), 2);
        assert!(matches!(&v[0].1, Verdict::Differs { r, .. } if r == "value b2"));
        assert!(matches!(&v[1].1, Verdict::Differs { r, .. } if r == "missing"));
        let (ok, text) = report(&v);
        assert!(!ok);
        assert!(text.contains("DIFF bc: G value b1, R value b2"), "{text}");
        assert!(text.ends_with("diffrun: 0 same, 2 differ, 0 flaky on G, 0 fail on G\n"));
    }

    #[test]
    fn no_compared_case_is_not_a_pass() {
        let g = results(&[("x", "FAIL")]);
        assert!(!report(&compare(std::slice::from_ref(&g), &g)).0);
    }
}
