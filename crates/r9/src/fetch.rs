//! `r9 fetch`: put pinned images and trees in the cache.
//!
//! `r9 fetch NAME` fetches the pin `NAME`: a tree by commit, or the images of a release.
//! `--row` limits a release to the image of one row. With no name, it fetches the newest release
//! and the tree of the `current` set, or only the release when `--row` is given.

use crate::repo::Repo;
use r9_pins::fetch;
use r9_pins::pins::{self, PinsFile};
use r9_pins::rows::Rows;
use r9_pins::sets::CURRENT;
use std::fmt::Write as _;

/// Run `r9 fetch`.
pub fn run(repo: &Repo, name: Option<&str>, row: Option<&str>) -> Result<String, String> {
    let path = repo.root.join(pins::FILE_NAME);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    pins::check_digest(&text)?;
    let file = PinsFile::parse(&text)?;
    let objtype = match row {
        Some(r) => Some(Rows::load(&repo.root)?.get(r)?.image.clone()),
        None => None,
    };

    let names: Vec<String> = match name {
        Some(n) => vec![n.to_string()],
        None => file
            .names()
            .into_iter()
            // A tree has no row, so --row with no name fetches only release images.
            .filter(|n| row.is_none() || file.release.contains_key(n))
            .filter(|n| {
                let sets = file
                    .tree
                    .get(n)
                    .map(|t| &t.sets)
                    .or_else(|| file.release.get(n).map(|r| &r.sets));
                sets.is_some_and(|s| s.iter().any(|s| s == CURRENT))
            })
            .collect(),
    };

    let mut report = String::new();
    for name in &names {
        if let Some(tree) = file.tree.get(name) {
            let got = fetch::tree(tree, &repo.cache)?;
            let _ = writeln!(report, "{name} {} {}", say(got.cached), got.path.display());
        } else if let Some(release) = file.release.get(name) {
            let mut any = false;
            for image in &release.image {
                if objtype
                    .as_ref()
                    .is_some_and(|o| format!("9front-{name}.{o}.gz") != image.file)
                {
                    continue;
                }
                any = true;
                let got = fetch::image(image, &repo.cache)?;
                let _ = writeln!(
                    report,
                    "{} {} {}",
                    image.file,
                    say(got.cached),
                    got.path.display()
                );
            }
            if !any {
                return Err(format!(
                    "release {name} has no image for row {}",
                    row.unwrap_or("")
                ));
            }
        } else {
            return Err(format!(
                "{} has no pin {name}; the pins are {}",
                pins::FILE_NAME,
                file.names().join(", ")
            ));
        }
    }
    Ok(report)
}

fn say(cached: bool) -> &'static str {
    if cached { "cached" } else { "fetched" }
}
