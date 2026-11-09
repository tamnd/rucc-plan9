//! Turn the rules of `sets.toml` into the pins of `pins.toml`.

use crate::pins::{Image, PinsFile, Release, Tree};
use crate::publish;
use crate::sets::{CURRENT, RELEASES, Sets};
use crate::web::Web;
use std::collections::BTreeMap;

/// The pins and the notes that `r9 sets` prints with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expanded {
    /// The new contents of `pins.toml`.
    pub file: PinsFile,
    /// Things a person should know, such as a release that is only on a torrent.
    pub notes: Vec<String>,
}

/// Expand `sets` with what 9front publishes. `time` is the `as_of` of the file, and `until`
/// limits the tree commit to that time, for a rerun of an old expansion. The commit of a
/// release that `old` has already found is kept.
pub fn expand(
    sets: &Sets,
    web: &dyn Web,
    time: &str,
    until: Option<&str>,
    old: Option<&PinsFile>,
) -> Result<Expanded, String> {
    let mut notes = Vec::new();
    let mut file = PinsFile {
        as_of: time.to_string(),
        tree: BTreeMap::new(),
        release: BTreeMap::new(),
    };

    let commit = web.commit_at(&sets.current.repo, &sets.current.branch, until)?;
    file.tree.insert(
        sets.current.tree.clone(),
        Tree {
            sets: vec![CURRENT.to_string()],
            repo: sets.current.repo.clone(),
            branch: sets.current.branch.clone(),
            commit: commit.sha,
            date: commit.date,
        },
    );

    let announcements = publish::feed(&web.get(&sets.source.feed)?)?;
    let announcements: Vec<_> = announcements
        .into_iter()
        .filter(|a| until.is_none_or(|u| a.date.as_str() <= u))
        .collect();
    if announcements.len() < sets.releases.newest {
        return Err(format!(
            "sets.toml wants {} releases but the feed has {}",
            sets.releases.newest,
            announcements.len()
        ));
    }
    let on_http = publish::listing(&web.get(&sets.source.iso)?);

    for (index, a) in announcements.iter().take(sets.releases.newest).enumerate() {
        let listed = publish::page(&web.get(&a.page)?);
        let number = listed
            .first()
            .map(|l| l.number.clone())
            .ok_or_else(|| format!("{} lists no images", a.page))?;
        if listed.iter().any(|l| l.number != number) {
            return Err(format!("{} lists images of more than one release", a.page));
        }
        let mut images = Vec::new();
        for wanted in &sets.source.images {
            let (objtype, kind) = wanted.split_once('.').unwrap_or_default();
            let name = format!("9front-{number}.{wanted}.gz");
            let l = listed
                .iter()
                .find(|l| l.file == name)
                .ok_or_else(|| format!("{} does not list {name}", a.page))?;
            let http = on_http.contains(&name);
            let (url, torrent) = if http {
                (
                    format!("{}{name}", sets.source.iso),
                    format!("{}{name}.torrent", sets.source.iso),
                )
            } else {
                (String::new(), format!("{}{name}.torrent", sets.source.old))
            };
            if !http {
                notes.push(format!(
                    "{name} is only on a torrent; r9 fetch needs aria2c for it"
                ));
            }
            images.push(Image {
                file: name,
                objtype: objtype.to_string(),
                kind: kind.to_string(),
                sha256: l.sha256.clone(),
                url,
                torrent,
            });
        }
        let mut in_sets = Vec::new();
        if index == 0 {
            in_sets.push(CURRENT.to_string());
        }
        in_sets.push(RELEASES.to_string());
        let commit = old
            .and_then(|o| o.release.get(&number))
            .map(|r| r.commit.clone())
            .unwrap_or_default();
        if commit.is_empty() {
            notes.push(format!(
                "{number} has no commit yet; r9 baseline finds it from the image's /sys (01.1)"
            ));
        }
        file.release.insert(
            number,
            Release {
                sets: in_sets,
                title: a.title.clone(),
                date: a.date.clone(),
                page: a.page.clone(),
                commit,
                image: images,
            },
        );
    }
    Ok(Expanded { file, notes })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::web::Commit;

    /// The web as a table of URLs.
    pub(crate) struct Table(pub BTreeMap<String, String>);

    impl Web for Table {
        fn get(&self, url: &str) -> Result<String, String> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| format!("no {url} in the table"))
        }
        fn commit_at(&self, slug: &str, branch: &str, _: Option<&str>) -> Result<Commit, String> {
            assert_eq!((slug, branch), ("9front/9front", "front"));
            Ok(Commit {
                sha: "2859ae04e800737307470538a942ab8f8261253b".into(),
                date: "2026-11-07T22:41:41Z".into(),
            })
        }
    }

    fn line(sha: char, file: &str) -> String {
        format!("   {}      {file}\n", sha.to_string().repeat(64))
    }

    pub(crate) fn web() -> Table {
        let mut t = BTreeMap::new();
        t.insert(
            "https://9front.org/releases/feed.json".to_string(),
            r#"
  "url": "http://9front.org:80/releases/2026/08/02/0/",
  "title": "9FRONT "THIS WAS SUPPOSED TO BE FUN" RELEASED",
  "url": "http://9front.org:80/releases/2026/01/24/0/",
  "title": "9FRONT "GEFS SERVICE PACK 1" RELEASED",
  "url": "http://9front.org:80/releases/2025/10/11/0/",
  "title": "9FRONT "RELEASE" RELEASED",
"#
            .to_string(),
        );
        t.insert(
            "https://9front.org/iso/".to_string(),
            r#"<a href="9front-11952.amd64.qcow2.gz"></a><a href="9front-11952.arm64.qcow2.gz"></a>"#
                .to_string(),
        );
        t.insert(
            "https://9front.org/releases/2026/08/02/0/".to_string(),
            line('a', "9front-11952.amd64.qcow2.gz")
                + &line('b', "9front-11952.arm64.qcow2.gz")
                + &line('c', "9front-11952.pi.img.gz"),
        );
        t.insert(
            "https://9front.org/releases/2026/01/24/0/".to_string(),
            line('d', "9front-11554.amd64.qcow2.gz") + &line('e', "9front-11554.arm64.qcow2.gz"),
        );
        Table(t)
    }

    #[test]
    fn the_rules_expand_into_a_tree_and_two_releases() {
        let sets = Sets::parse(crate::sets::tests::SAMPLE).unwrap();
        let Expanded { file, notes } =
            expand(&sets, &web(), "2026-11-09T10:00:00Z", None, None).unwrap();
        assert_eq!(file.names(), vec!["front", "11554", "11952"]);
        let new = &file.release["11952"];
        assert_eq!(new.sets, vec!["current", "releases"]);
        assert_eq!(new.image[0].sha256, "a".repeat(64));
        assert_eq!(new.image[1].objtype, "arm64");
        assert!(
            new.image[0]
                .url
                .ends_with("/iso/9front-11952.amd64.qcow2.gz")
        );
        let old = &file.release["11554"];
        assert_eq!(old.sets, vec!["releases"]);
        assert!(old.image[0].url.is_empty());
        assert!(old.image[0].torrent.contains("/iso/old/"));
        assert!(notes.iter().any(|n| n.contains("only on a torrent")));
        assert!(
            file.problems(&Sets::names()).is_empty(),
            "{:?}",
            file.problems(&Sets::names())
        );
        let text = file.render().unwrap();
        crate::pins::check_digest(&text).unwrap();
    }

    #[test]
    fn a_found_commit_is_kept() {
        let sets = Sets::parse(crate::sets::tests::SAMPLE).unwrap();
        let mut old = expand(&sets, &web(), "t", None, None).unwrap().file;
        old.release.get_mut("11952").unwrap().commit = "f".repeat(40);
        let new = expand(&sets, &web(), "t", None, Some(&old)).unwrap();
        assert_eq!(new.file.release["11952"].commit, "f".repeat(40));
        assert!(
            !new.notes
                .iter()
                .any(|n| n.starts_with("11952 has no commit"))
        );
    }

    #[test]
    fn an_old_time_keeps_out_newer_releases() {
        let sets = Sets::parse(crate::sets::tests::SAMPLE).unwrap();
        let err = expand(&sets, &web(), "t", Some("2026-03-01T00:00:00Z"), None).unwrap_err();
        assert!(err.contains("2025/10/11"), "{err}");
    }
}
