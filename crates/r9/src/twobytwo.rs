//! `r9 2x2`: run one suite on the combinations of the G and R kernels and user spaces, and find
//! which half of R a failure comes from (plan 12.7).
//!
//! | | G user space | R user space |
//! |---|---|---|
//! | G kernel | the oracle | user |
//! | R kernel | kernel | R |
//!
//! A test that only the user cell fails is a fault of the user space of R: a program or a
//! library. A test that only the kernel cell fails is a fault of the kernel of R. A test that both
//! fail points first at `libc` or at another library that both sides use.
//!
//! The user cell is the image of an R build (`r9 build --side R --keep`). `mk install` in
//! `/sys/src` builds no kernel, so that image boots the kernel of G with the user space of R. The
//! kernel cells need a kernel that rucc built (plan 09). `--kernel FILE` is an image with the user
//! space of G and the kernel of R, and `--both FILE` is an image with the user space and the
//! kernel of R. A cell runs only when its image is given, and the report names only the faults
//! that the cells that ran can show.
//!
//! The oracle is the baseline of the suite for the pin of the G image, in
//! `baselines/PIN/OBJTYPE/SUITE.json`, when there is one. Otherwise G boots `--runs` times, and a
//! test is in the comparison only when each run gives the same result.

use crate::baseline::{self, Results};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The suites that `r9 2x2` can run.
pub const SUITES: &[&str] = &["boot", "smoke", "test", "diffrun"];

/// The half of R that a failure comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blame {
    /// Each cell gives the result of the oracle.
    Same,
    /// The oracle fails the test, so it is not compared.
    NotGraded,
    /// Only the cells with the user space of R fail.
    User,
    /// Only the cells with the kernel of R fail.
    Kernel,
    /// The user cell and the kernel cell both fail.
    Library,
    /// Only the cell with the kernel and the user space of R fails.
    Together,
}

impl Blame {
    /// The word for the report.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::NotGraded => "gfail",
            Self::User => "USER",
            Self::Kernel => "KERNEL",
            Self::Library => "LIBRARY",
            Self::Together => "TOGETHER",
        }
    }
}

/// Find the half of R that `oracle` and the results of the other cells point at. A cell that
/// did not run is `None`.
#[must_use]
pub fn attribute(oracle: &str, user: Option<&str>, kernel: Option<&str>, r: Option<&str>) -> Blame {
    if oracle == "FAIL" {
        return Blame::NotGraded;
    }
    let differs = |cell: Option<&str>| cell.map(|v| v != oracle);
    match (differs(user), differs(kernel), differs(r)) {
        (Some(true), Some(true), _) => Blame::Library,
        (Some(true), _, _) => Blame::User,
        (_, Some(true), _) => Blame::Kernel,
        (_, _, Some(true)) => Blame::Together,
        _ => Blame::Same,
    }
}

/// The results of each cell. `user`, `kernel` and `r` are `None` for a cell that did not run.
#[derive(Debug, Clone, Default)]
pub struct Cells {
    /// The graded results of G.
    pub oracle: Results,
    /// The R user space on the G kernel.
    pub user: Option<Results>,
    /// The G user space on the R kernel.
    pub kernel: Option<Results>,
    /// The R user space on the R kernel.
    pub r: Option<Results>,
}

/// The report, and whether each graded test gives the result of the oracle in each cell.
#[must_use]
pub fn report(suite: &str, cells: &Cells) -> (bool, String) {
    let mut text = String::new();
    let mut counts = std::collections::BTreeMap::new();
    let get = |c: &Option<Results>, name: &str| {
        c.as_ref()
            .map(|r| r.get(name).map_or("missing", String::as_str).to_owned())
    };
    for (name, oracle) in &cells.oracle {
        let user = get(&cells.user, name);
        let kernel = get(&cells.kernel, name);
        let r = get(&cells.r, name);
        let blame = attribute(oracle, user.as_deref(), kernel.as_deref(), r.as_deref());
        *counts.entry(blame.word()).or_insert(0_usize) += 1;
        if !matches!(blame, Blame::Same | Blame::NotGraded) {
            let mut line = format!("{} {name}: G {oracle}", blame.word());
            for (cell, v) in [("user", &user), ("kernel", &kernel), ("R", &r)] {
                if let Some(v) = v {
                    let _ = write!(line, ", {cell} {v}");
                }
            }
            let _ = writeln!(text, "{line}");
        }
    }
    let ran: Vec<&str> = [
        ("user", cells.user.is_some()),
        ("kernel", cells.kernel.is_some()),
        ("R", cells.r.is_some()),
    ]
    .iter()
    .filter(|c| c.1)
    .map(|c| c.0)
    .collect();
    let n = |w: &str| counts.get(w).copied().unwrap_or(0);
    let faults = n("USER") + n("KERNEL") + n("LIBRARY") + n("TOGETHER");
    let _ = writeln!(
        text,
        "2x2 {suite}: cells G and {}; {} same, {} user, {} kernel, {} library, {} together, {} fail on G",
        ran.join(", "),
        n("same"),
        n("USER"),
        n("KERNEL"),
        n("LIBRARY"),
        n("TOGETHER"),
        n("gfail")
    );
    (faults == 0 && n("same") > 0, text)
}

/// Boot `image` once with `suite` and read the results.
fn boot_once(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
    objtype: &str,
    suite: &str,
    image: &Path,
    work: &Path,
) -> Result<Results, String> {
    // The suite is the target of `r9 2x2`, and the target of `r9 boot` is the release, so the
    // boot does not get it.
    let mut a = args.clone();
    a.target = None;
    if suite == "diffrun" {
        return crate::diffrun::boot_once(repo, &a, objtype, image, work);
    }
    let file = baseline::SUITES
        .iter()
        .find(|s| s.0 == suite)
        .map(|s| s.1)
        .ok_or_else(|| format!("there is no suite {suite}"))?;
    let path = |p: &Path| p.display().to_string();
    a.values.insert("row".into(), objtype.to_string());
    a.values.insert("image".into(), path(image));
    a.values
        .insert("run".into(), path(&repo.root.join("smoke").join(file)));
    a.values.insert("in".into(), path(&repo.root.join("guest")));
    a.values.insert("out".into(), path(work));
    let req = crate::boot::request_from(repo, &a)?;
    let outcome = crate::boot::run(&req)?;
    let console = std::fs::read_to_string(work.join("console.log")).unwrap_or_default();
    Ok(baseline::results_of(suite, &outcome, &console))
}

/// The graded results of the baseline file at `path`, when there is one.
fn from_baseline(path: &Path) -> Option<Results> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    serde_json::from_value(value.get("results")?.clone()).ok()
}

/// Run `r9 2x2 SUITE --r FILE`. `--g FILE` is the G image, and the default is the `g-base` image
/// of the row. `--kernel FILE` and `--both FILE` are the images of the kernel cells. The work goes
/// to `work/2x2/SUITE` or `--out`.
pub fn command(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
) -> Result<(bool, String), String> {
    let suite = args
        .target
        .as_deref()
        .ok_or("give the suite: boot, smoke, test or diffrun")?;
    if !SUITES.contains(&suite) {
        return Err(format!(
            "there is no suite {suite}; the suites are {}",
            SUITES.join(", ")
        ));
    }
    let objtype = args.get("row").unwrap_or("amd64").to_string();
    let r_image = PathBuf::from(
        args.get("r")
            .ok_or("r9 2x2 needs --r FILE, the image of an R build")?,
    );
    let g_image = match args.get("g") {
        Some(g) => PathBuf::from(g),
        None => crate::image::g_base(repo, &objtype).ok_or_else(|| {
            format!("there is no g-base image for {objtype}; run r9 image build --row {objtype}")
        })?,
    };
    let out = args.get("out").map_or_else(
        || repo.root.join("work").join("2x2").join(suite),
        PathBuf::from,
    );
    let saved = repo
        .root
        .join("baselines")
        .join(baseline::pin_of(&g_image))
        .join(&objtype)
        .join(format!("{suite}.json"));
    let oracle = if let Some(results) = from_baseline(&saved) {
        eprintln!("r9 2x2: the oracle is {}", saved.display());
        results
    } else {
        let count: usize = args
            .get("runs")
            .map_or(Ok(1), str::parse)
            .map_err(|e| format!("--runs: {e}"))?;
        let mut runs = Vec::new();
        for n in 1..=count.max(1) {
            let results = boot_once(
                repo,
                args,
                &objtype,
                suite,
                &g_image,
                &out.join(format!("g-{n}")),
            )?;
            runs.push(baseline::Run {
                results,
                seconds: 0.0,
                passed: true,
            });
        }
        baseline::combine(&runs).graded
    };
    let user = boot_once(repo, args, &objtype, suite, &r_image, &out.join("user"))?;
    let cell = |flag: &str, dir: &str| -> Result<Option<Results>, String> {
        args.get(flag)
            .map(|image| {
                boot_once(
                    repo,
                    args,
                    &objtype,
                    suite,
                    Path::new(image),
                    &out.join(dir),
                )
            })
            .transpose()
    };
    let cells = Cells {
        oracle,
        user: Some(user),
        kernel: cell("kernel", "kernel")?,
        r: cell("both", "R")?,
    };
    let (ok, text) = report(suite, &cells);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let path = out.join("2x2.txt");
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
    fn the_cells_that_fail_name_the_half_of_r() {
        let a = |u, k, r| attribute("ok", u, k, r);
        assert_eq!(a(Some("ok"), Some("ok"), Some("ok")), Blame::Same);
        assert_eq!(a(Some("FAIL"), Some("ok"), Some("FAIL")), Blame::User);
        assert_eq!(a(Some("ok"), Some("FAIL"), Some("FAIL")), Blame::Kernel);
        assert_eq!(a(Some("FAIL"), Some("FAIL"), Some("FAIL")), Blame::Library);
        assert_eq!(a(Some("ok"), Some("ok"), Some("FAIL")), Blame::Together);
        assert_eq!(a(Some("FAIL"), None, None), Blame::User);
        assert_eq!(a(None, None, None), Blame::Same);
        assert_eq!(attribute("FAIL", Some("ok"), None, None), Blame::NotGraded);
    }

    #[test]
    fn the_user_cell_alone_names_the_user_space() {
        let cells = Cells {
            oracle: results(&[("awk", "value a"), ("cc", "ok"), ("gs", "FAIL")]),
            user: Some(results(&[("awk", "value b"), ("cc", "ok")])),
            ..Cells::default()
        };
        let (ok, text) = report("smoke", &cells);
        assert!(!ok);
        assert!(
            text.contains("USER awk: G value a, user value b\n"),
            "{text}"
        );
        assert!(text.ends_with(
            "2x2 smoke: cells G and user; 1 same, 1 user, 0 kernel, 0 library, 0 together, 1 fail on G\n"
        ));
    }

    #[test]
    fn the_kernel_cells_name_the_kernel_and_the_library() {
        let cells = Cells {
            oracle: results(&[("cc", "ok"), ("ls", "ok"), ("halted", "ok")]),
            user: Some(results(&[("cc", "ok"), ("ls", "FAIL"), ("halted", "ok")])),
            kernel: Some(results(&[("cc", "ok"), ("ls", "FAIL"), ("halted", "FAIL")])),
            r: Some(results(&[("cc", "ok"), ("ls", "FAIL"), ("halted", "FAIL")])),
        };
        let (ok, text) = report("smoke", &cells);
        assert!(!ok);
        assert!(text.contains("LIBRARY ls:"), "{text}");
        assert!(text.contains("KERNEL halted:"), "{text}");
        assert!(text.contains("cells G and user, kernel, R;"), "{text}");
    }

    #[test]
    fn a_missing_result_is_a_fault() {
        let cells = Cells {
            oracle: results(&[("cc", "ok"), ("halted", "ok")]),
            user: Some(results(&[("halted", "ok")])),
            ..Cells::default()
        };
        let (ok, text) = report("smoke", &cells);
        assert!(!ok);
        assert!(text.contains("USER cc: G ok, user missing"), "{text}");
    }
}
