//! `r9 fixedpoint`: build the tree again with the kencc that rucc built, and compare the result
//! with G (plan 12.4).
//!
//! The image of an R build (`r9 build --side R --keep`) has the `6c`, `6l` and `6a` that rucc
//! compiled. A build of side G in that image, with no rucc on the host, is Rk: kencc built by
//! rucc. kencc gives the same output for the same input, so Rk must write the same files as G,
//! which is kencc built by kencc. A file that differs names a unit that the kencc of R compiles
//! in another way, and thus a function of kencc that rucc compiled wrong.
//!
//! The tree of the R image already has the objects of the R build, and `mk install` alone does
//! nothing there. So both builds run `mk clean` and then `mk install` in each directory, and each
//! file is built again. The G side is a build of the `g-base` image with the same targets, or
//! the directory of such a build with `--g`.

use crate::treediff;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The mk targets of both builds.
pub const TARGETS: &str = "clean install";

/// The arguments of `r9 build --side G` for one side of the fixed point. `image` is the image to
/// boot, or `None` for the `g-base` image of the row.
#[must_use]
pub fn build_args(args: &crate::cli::Args, image: Option<&Path>, out: &Path) -> crate::cli::Args {
    let mut a = args.clone();
    "build".clone_into(&mut a.command);
    a.target = None;
    for name in ["r", "g", "image", "rucc", "test", "test-timeout"] {
        a.values.remove(name);
    }
    a.values.insert("side".into(), "G".into());
    a.values.insert("targets".into(), TARGETS.into());
    a.values.insert("out".into(), out.display().to_string());
    if let Some(image) = image {
        a.values.insert("image".into(), image.display().to_string());
    }
    a
}

/// The last line of the report, and whether Rk is G.
#[must_use]
pub fn verdict(g_passed: bool, rk_passed: bool, d: &treediff::Diff) -> (bool, String) {
    if !g_passed {
        return (
            false,
            "fixedpoint: the G build failed, so there is no comparison".into(),
        );
    }
    if !rk_passed {
        return (false, "fixedpoint: the Rk build failed".into());
    }
    if d.is_empty() {
        return (true, "fixedpoint: Rk is the same as G".into());
    }
    let n = d.only_a.len() + d.only_b.len() + d.changed.len();
    (
        false,
        format!(
            "fixedpoint: Rk differs from G in {n} files; use r9 objdiff on each changed object"
        ),
    )
}

/// Run one build of side G and return whether it passed.
fn build(repo: &crate::repo::Repo, args: &crate::cli::Args, what: &str) -> Result<bool, String> {
    eprintln!("r9 fixedpoint: the {what} build");
    let (passed, report) = crate::build::command(repo, args)?;
    eprint!("{report}");
    Ok(passed)
}

/// Run `r9 fixedpoint`. `--r FILE` is the image of an R build. `--g DIR` is the directory of a G
/// build with the targets of [`TARGETS`]; with no `--g`, the G build runs first. The work goes to
/// `work/fixedpoint` or `--out`, in `g` and `rk`.
pub fn command(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
) -> Result<(bool, String), String> {
    let r_image = PathBuf::from(
        args.get("r")
            .ok_or("r9 fixedpoint needs --r FILE, the image of an R build")?,
    );
    if !r_image.is_file() {
        return Err(format!("{}: there is no such image", r_image.display()));
    }
    let out = args
        .get("out")
        .map_or_else(|| repo.root.join("work").join("fixedpoint"), PathBuf::from);
    let (g_dir, g_passed) = if let Some(dir) = args.get("g") {
        let dir = PathBuf::from(dir);
        let toml = std::fs::read_to_string(dir.join("build.toml"))
            .map_err(|e| format!("{}: {e}", dir.join("build.toml").display()))?;
        (dir, toml.contains("passed = true"))
    } else {
        let dir = out.join("g");
        let passed = build(repo, &build_args(args, None, &dir), "G")?;
        (dir, passed)
    };
    let rk_dir = out.join("rk");
    let rk_passed = build(repo, &build_args(args, Some(&r_image), &rk_dir), "Rk")?;
    let g = treediff::load(&g_dir)?;
    let rk = treediff::load(&rk_dir)?;
    let d = treediff::diff(&g, &rk);
    let top = args
        .get("top")
        .map_or(Ok(20), str::parse)
        .map_err(|e| format!("--top: {e}"))?;
    let (ok, last) = verdict(g_passed, rk_passed, &d);
    let mut text = format!(
        "A is G in {}, B is Rk in {}\n",
        g_dir.display(),
        rk_dir.display()
    );
    text.push_str(&treediff::report(&d, top));
    let _ = writeln!(text, "{last}");
    let path = out.join("fixedpoint.txt");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((ok, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_side_is_a_g_build_that_builds_each_file_again() {
        let mut args = crate::cli::Args {
            command: "fixedpoint".into(),
            ..crate::cli::Args::default()
        };
        args.values.insert("r".into(), "/x/root.qcow2".into());
        args.values.insert("rucc".into(), "/x/rucc".into());
        args.values.insert("accel".into(), "tcg".into());
        let g = build_args(&args, None, Path::new("/w/g"));
        assert_eq!(g.command, "build");
        assert_eq!(g.get("side"), Some("G"));
        assert_eq!(g.get("targets"), Some("clean install"));
        assert_eq!(g.get("out"), Some("/w/g"));
        assert_eq!(g.get("accel"), Some("tcg"));
        assert_eq!(g.get("image"), None);
        assert_eq!(g.get("r"), None);
        assert_eq!(g.get("rucc"), None);
        let rk = build_args(&args, Some(Path::new("/x/root.qcow2")), Path::new("/w/rk"));
        assert_eq!(rk.get("image"), Some("/x/root.qcow2"));
    }

    #[test]
    fn rk_passes_only_when_both_builds_pass_and_write_the_same_files() {
        let same = treediff::Diff::default();
        assert!(verdict(true, true, &same).0);
        assert!(!verdict(false, true, &same).0);
        assert!(!verdict(true, false, &same).0);
        let d = treediff::Diff {
            changed: vec!["/amd64/bin/awk".into()],
            only_b: vec!["/sys/src/cmd/x.6".into()],
            ..treediff::Diff::default()
        };
        let (ok, text) = verdict(true, true, &d);
        assert!(!ok);
        assert!(text.contains("in 2 files"), "{text}");
    }
}
