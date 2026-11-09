//! `r9 check`: check the committed configuration with no network.
//!
//! It reads `sets.toml`, `pins.toml`, `rows.toml` and `machines.toml`. It fails when one does not
//! parse, when the digest of `pins.toml` does not hold, or when the files do not agree with each
//! other: a row whose image is not in `sets.toml`, or a release with no image for a row.

use crate::repo::Repo;
use r9_pins::machines::Machines;
use r9_pins::pins::{self, PinsFile};
use r9_pins::rows::Rows;
use r9_pins::sets::Sets;
use std::fmt::Write as _;

/// Run `r9 check`. It gives the report, or the list of problems as the error.
pub fn run(repo: &Repo) -> Result<String, String> {
    let sets = Sets::load(&repo.root)?;
    let rows = Rows::load(&repo.root)?;
    let machines = Machines::load(&repo.root)?;
    let path = repo.root.join(pins::FILE_NAME);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e}; write it with r9 sets --write", path.display()))?;
    pins::check_digest(&text)?;
    let file = PinsFile::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;

    let mut problems = file.problems(&Sets::names());
    problems.extend(agree(&sets, &rows, &file));
    if !problems.is_empty() {
        return Err(format!(
            "{} problems:\n{}",
            problems.len(),
            problems.join("\n")
        ));
    }
    let mut report = String::new();
    let _ = writeln!(
        report,
        "{}: {} trees and {} releases, as of {}",
        pins::FILE_NAME,
        file.tree.len(),
        file.release.len(),
        file.as_of
    );
    let _ = writeln!(
        report,
        "{}: rows {}",
        r9_pins::rows::FILE_NAME,
        names(rows.row.keys())
    );
    let _ = writeln!(
        report,
        "{}: machines {}",
        r9_pins::machines::FILE_NAME,
        names(machines.machine.keys())
    );
    let _ = writeln!(report, "ok");
    Ok(report)
}

fn names<'a>(keys: impl Iterator<Item = &'a String>) -> String {
    keys.map(String::as_str).collect::<Vec<_>>().join(", ")
}

/// The ways the files do not agree.
fn agree(sets: &Sets, rows: &Rows, file: &PinsFile) -> Vec<String> {
    let mut out = Vec::new();
    for (name, row) in &rows.row {
        if !sets.source.images.contains(&row.image) {
            out.push(format!(
                "row {name} boots {}, which sets.toml does not pin",
                row.image
            ));
            continue;
        }
        for (number, release) in &file.release {
            let want = format!("9front-{number}.{}.gz", row.image);
            if !release.image.iter().any(|i| i.file == want) {
                out.push(format!("release {number} has no {want} for row {name}"));
            }
        }
    }
    if let Some(tree) = file.tree.get(&sets.current.tree) {
        if tree.repo != sets.current.repo || tree.branch != sets.current.branch {
            out.push(format!(
                "tree {} is {} {}, but sets.toml says {} {}",
                sets.current.tree, tree.repo, tree.branch, sets.current.repo, sets.current.branch
            ));
        }
    } else {
        out.push(format!("pins.toml has no tree {}", sets.current.tree));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_committed_files_pass() {
        let repo = Repo {
            root: root(),
            cache: PathBuf::from("/nonexistent"),
        };
        let report = run(&repo).unwrap();
        assert!(report.ends_with("ok\n"), "{report}");
    }

    #[test]
    fn a_row_without_an_image_is_found() {
        let repo = root();
        let sets = Sets::load(&repo).unwrap();
        let rows = Rows::load(&repo).unwrap();
        let mut file = PinsFile::load(&repo).unwrap().unwrap();
        for release in file.release.values_mut() {
            release.image.retain(|i| i.objtype != "arm64");
        }
        let problems = agree(&sets, &rows, &file).join("\n");
        assert!(problems.contains("for row arm64"), "{problems}");
    }
}
