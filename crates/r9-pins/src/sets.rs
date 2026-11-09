//! `sets.toml`: the rules that say which trees and releases are pinned (plan 04.1).
//!
//! People edit this file. `r9 sets --write` reads it and writes `pins.toml`.

use serde::Deserialize;
use std::path::Path;

/// The name of the file at the root of the repository.
pub const FILE_NAME: &str = "sets.toml";

/// The set of the newest tree and the newest release.
pub const CURRENT: &str = "current";

/// The set of the supported releases.
pub const RELEASES: &str = "releases";

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sets {
    /// The rules of the `current` set.
    pub current: Current,
    /// The rules of the `releases` set.
    pub releases: Releases,
    /// Where 9front publishes its releases.
    pub source: Source,
}

/// The `current` set: the head of a branch and the newest release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Current {
    /// The name of the tree pin, such as `front`.
    pub tree: String,
    /// The repository on GitHub, as `owner/name`.
    pub repo: String,
    /// The branch.
    pub branch: String,
    /// The policy that moves the pin, for the record (04.1).
    pub policy: String,
}

/// The `releases` set.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Releases {
    /// How many of the newest releases are pinned.
    pub newest: usize,
}

/// Where the releases are.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// The JSON feed of the release announcements.
    pub feed: String,
    /// The directory with the images of the newest release.
    pub iso: String,
    /// The directory with the torrents of the older releases.
    pub old: String,
    /// The images to pin from each release, as `objtype.kind`, such as `amd64.qcow2`.
    pub images: Vec<String>,
}

impl Sets {
    /// Read `sets.toml` from the repository root.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(FILE_NAME);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parse the text of the file and check its rules.
    pub fn parse(text: &str) -> Result<Self, String> {
        let sets: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if sets.releases.newest == 0 {
            return Err("releases.newest must be 1 or more".into());
        }
        if sets.source.images.is_empty() {
            return Err("source.images is empty".into());
        }
        for image in &sets.source.images {
            match image.split_once('.') {
                Some((objtype, "qcow2" | "iso")) if !objtype.is_empty() => {}
                _ => {
                    return Err(format!(
                        "source.images: {image} is not objtype.qcow2 or objtype.iso"
                    ));
                }
            }
        }
        if !sets.current.repo.contains('/') {
            return Err(format!(
                "current.repo {} is not owner/name",
                sets.current.repo
            ));
        }
        Ok(sets)
    }

    /// The names of the sets.
    #[must_use]
    pub fn names() -> Vec<String> {
        vec![CURRENT.to_string(), RELEASES.to_string()]
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"
[current]
tree = "front"
repo = "9front/9front"
branch = "front"
policy = "weekly"

[releases]
newest = 2

[source]
feed = "https://9front.org/releases/feed.json"
iso = "https://9front.org/iso/"
old = "https://9front.org/iso/old/"
images = ["amd64.qcow2", "arm64.qcow2"]
"#;

    #[test]
    fn the_sample_parses() {
        let sets = Sets::parse(SAMPLE).unwrap();
        assert_eq!(sets.releases.newest, 2);
        assert_eq!(sets.current.tree, "front");
    }

    #[test]
    fn bad_rules_are_refused() {
        let bad = SAMPLE.replace("newest = 2", "newest = 0");
        assert!(Sets::parse(&bad).unwrap_err().contains("newest"));
        let bad = SAMPLE.replace("\"arm64.qcow2\"", "\"arm64.img\"");
        assert!(Sets::parse(&bad).unwrap_err().contains("arm64.img"));
        let bad = SAMPLE.replace("tree = \"front\"", "tree = \"front\"\nextra = 1");
        assert!(Sets::parse(&bad).is_err());
    }
}
