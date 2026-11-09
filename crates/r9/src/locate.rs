//! `r9 locate`: find the commit that a release was cut from (plan 01.1).
//!
//! No file in a release image names its commit. So the image's tree is hashed file by file, as
//! git hashes a blob, and compared with the trees of the commits of `front` near the release
//! date. The commit with the fewest different files wins. When no commit matches exactly, the
//! differences are listed.
//!
//! The history is a blobless clone (`git clone --bare --filter=blob:none`), which has every tree
//! and no file contents, so it is small and `git ls-tree` needs no network.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Path to blob hash.
pub type Files = BTreeMap<String, String>;

/// How a tree differs from a commit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Files in the tree only.
    pub only_tree: Vec<String>,
    /// Files in the commit only.
    pub only_commit: Vec<String>,
    /// Files in both with different contents.
    pub changed: Vec<String>,
}

impl Diff {
    /// The number of files that differ.
    #[must_use]
    pub fn len(&self) -> usize {
        self.only_tree.len() + self.only_commit.len() + self.changed.len()
    }

    /// True when the tree matches the commit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Compare the files of a tree with the files of a commit.
#[must_use]
pub fn diff(tree: &Files, commit: &Files) -> Diff {
    let mut d = Diff::default();
    for (path, hash) in tree {
        match commit.get(path) {
            None => d.only_tree.push(path.clone()),
            Some(h) if h != hash => d.changed.push(path.clone()),
            Some(_) => {}
        }
    }
    for path in commit.keys() {
        if !tree.contains_key(path) {
            d.only_commit.push(path.clone());
        }
    }
    d
}

/// Hash the regular files under `root/prefix` for each prefix, as git hashes blobs. The paths
/// are relative to `root`.
pub fn hash_tree(root: &Path, prefixes: &[&str]) -> Result<Files, String> {
    let mut paths = Vec::new();
    for prefix in prefixes {
        walk(root, &root.join(prefix), &mut paths)?;
    }
    paths.sort();
    let hashes = hash_paths(root, &paths)?;
    Ok(paths.into_iter().zip(hashes).collect())
}

/// Hash the regular files of the tar archive `tar` whose paths start with one of `prefixes`, as
/// git hashes blobs. Each file is copied out under a numbered name, so that two paths that
/// differ only by case stay two files on a host that ignores case.
pub fn hash_archive(tar: &Path, prefixes: &[&str]) -> Result<Files, String> {
    let archive = crate::tar::list(tar)?;
    let entries: Vec<&crate::tar::Entry> = archive
        .files
        .iter()
        .filter(|e| {
            prefixes
                .iter()
                .any(|p| e.name.starts_with(&format!("{p}/")))
        })
        .collect();
    let dir = std::env::temp_dir().join(format!("r9-hash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut f = std::fs::File::open(tar).map_err(|e| format!("{}: {e}", tar.display()))?;
    let mut numbered = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let data = crate::tar::contents(&mut f, e)?;
        let path = dir.join(i.to_string());
        std::fs::write(&path, data).map_err(|x| format!("{}: {x}", path.display()))?;
        numbered.push(i.to_string());
    }
    let hashes = hash_paths(&dir, &numbered);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(entries
        .into_iter()
        .map(|e| e.name.clone())
        .zip(hashes?)
        .collect())
}

/// Hash the files `paths` under `root` with `git hash-object`, in order.
fn hash_paths(root: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = Command::new("git")
        .args(["hash-object", "--no-filters", "--stdin-paths"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin for git")?;
    let list = paths.join("\n") + "\n";
    let writer = std::thread::spawn(move || stdin.write_all(list.as_bytes()));
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git hash-object: {e}"))?;
    writer
        .join()
        .map_err(|_| "git hash-object: the writer failed".to_string())?
        .map_err(|e| format!("git hash-object: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git hash-object failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let hashes: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(String::from)
        .collect();
    if hashes.len() != paths.len() {
        return Err(format!(
            "git hash-object gave {} hashes for {} files",
            hashes.len(),
            paths.len()
        ));
    }
    Ok(hashes)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if kind.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            out.push(rel.display().to_string());
        }
    }
    Ok(())
}

/// The files of `commit` under the prefixes, from the git directory `repo`.
pub fn commit_files(repo: &Path, commit: &str, prefixes: &[&str]) -> Result<Files, String> {
    let out = git(repo)
        .args(["ls-tree", "-r", "--full-tree", commit, "--"])
        .args(prefixes)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-tree {commit}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut files = Files::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // <mode> SP <type> SP <hash> TAB <path>
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let mut meta = meta.split(' ');
        if let (Some(_), Some("blob"), Some(hash)) = (meta.next(), meta.next(), meta.next()) {
            files.insert(path.to_string(), hash.to_string());
        }
    }
    Ok(files)
}

/// The commits of `branch` with a committer date from `since` to `until` (dates such as
/// 2026-08-02), in the order of `git log`, as `(hash, date, subject)`. The dates of 9front
/// commits are not in order along the branch, so `git log --since` would stop too early. The
/// whole branch is listed and the dates are filtered here.
pub fn commits(
    repo: &Path,
    branch: &str,
    since: &str,
    until: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let out = git(repo)
        .args(["log", "--first-parent", "--format=%H%x09%cI%x09%s", branch])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git log {branch}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            Some((
                f.next()?.to_string(),
                f.next()?.to_string(),
                f.next().unwrap_or("").to_string(),
            ))
        })
        .filter(|(_, date, _)| {
            let day = date.get(..10).unwrap_or("");
            (since..=until).contains(&day)
        })
        .collect())
}

fn git(repo: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("--git-dir").arg(repo);
    c
}

/// The result of `r9 locate`.
#[derive(Debug, Clone)]
pub struct Located {
    /// The commit with the fewest different files.
    pub commit: String,
    /// Its date.
    pub date: String,
    /// Its subject.
    pub subject: String,
    /// How the tree differs from it.
    pub diff: Diff,
    /// The other commits with as few different files, which the tree cannot tell apart.
    pub ties: Vec<String>,
    /// The number of commits compared.
    pub compared: usize,
    /// The number of files in the tree.
    pub files: usize,
}

/// Compare `tree` with each commit of `branch` from `since` to `until`, and keep the best one.
/// The walk stops at the first exact match.
pub fn locate(
    repo: &Path,
    branch: &str,
    since: &str,
    until: &str,
    tree: &Files,
    prefixes: &[&str],
) -> Result<Located, String> {
    let list = commits(repo, branch, since, until)?;
    let mut best: Option<Located> = None;
    let mut compared = 0;
    for (commit, date, subject) in list {
        let d = diff(tree, &commit_files(repo, &commit, prefixes)?);
        compared += 1;
        let better = best.as_ref().is_none_or(|b| d.len() < b.diff.len());
        let exact = d.is_empty();
        if better {
            best = Some(Located {
                commit,
                date,
                subject,
                diff: d,
                ties: Vec::new(),
                compared,
                files: tree.len(),
            });
        } else if let Some(b) = best.as_mut().filter(|b| b.diff.len() == d.len()) {
            b.ties.push(commit);
        }
        if exact {
            break;
        }
    }
    let mut best = best.ok_or_else(|| format!("no commits of {branch} from {since} to {until}"))?;
    best.compared = compared;
    Ok(best)
}

impl Located {
    /// The report `r9 locate` prints. At most `max` paths of each kind are listed.
    #[must_use]
    pub fn report(&self, max: usize) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{} {} {}", self.commit, self.date, self.subject);
        let _ = writeln!(
            out,
            "{} files, {} commits compared, {} files differ{}",
            self.files,
            self.compared,
            self.diff.len(),
            if self.diff.is_empty() {
                ": exact match"
            } else {
                ""
            }
        );
        if !self.ties.is_empty() {
            let _ = writeln!(
                out,
                "{} other commits differ in as few files: {}",
                self.ties.len(),
                self.ties
                    .iter()
                    .map(|c| c.get(..12).unwrap_or(c))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        for (what, paths) in [
            ("only in the tree", &self.diff.only_tree),
            ("only in the commit", &self.diff.only_commit),
            ("changed", &self.diff.changed),
        ] {
            if paths.is_empty() {
                continue;
            }
            let _ = writeln!(out, "{what}: {}", paths.len());
            for p in paths.iter().take(max) {
                let _ = writeln!(out, "  {p}");
            }
        }
        out
    }
}

/// The prefixes of a release tree that are compared.
pub const PREFIXES: &[&str] = &["sys/src", "sys/include"];

/// The days from 1970-01-01 to a date `YYYY-MM-DD`.
pub fn days(date: &str) -> Result<i64, String> {
    let bad = || format!("{date} is not a date YYYY-MM-DD");
    let mut f = date.get(..10).ok_or_else(bad)?.split('-');
    let mut next =
        || -> Result<i64, String> { f.next().and_then(|x| x.parse().ok()).ok_or_else(bad) };
    let (y, m, d) = (next()?, next()?, next()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(bad());
    }
    // The algorithm of H. Hinnant, days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(era * 146_097 + doe - 719_468)
}

/// The date `YYYY-MM-DD` of a day number from [`days`].
#[must_use]
pub fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The blobless clone of a GitHub repository in the cache. Make it when it is not there, and
/// fetch new commits when it is.
pub fn history(cache: &Path, github: &str) -> Result<std::path::PathBuf, String> {
    let dir = cache
        .join("history")
        .join(format!("{}.git", github.replace('/', "-")));
    let status = if dir.join("HEAD").is_file() {
        git(&dir).args(["fetch", "-q", "origin"]).status()
    } else {
        std::fs::create_dir_all(cache.join("history"))
            .map_err(|e| format!("{}: {e}", cache.display()))?;
        Command::new("git")
            .args(["clone", "-q", "--bare", "--filter=blob:none"])
            .arg(format!("https://github.com/{github}"))
            .arg(&dir)
            .status()
    }
    .map_err(|e| format!("cannot run git: {e}"))?;
    if status.success() {
        Ok(dir)
    } else {
        Err(format!(
            "cannot clone or fetch {github} into {}",
            dir.display()
        ))
    }
}

/// `r9 locate`: find the commit of the release `name` from its tree `tree`, over the `window`
/// days up to the day after the release date. Return the report.
pub fn run(
    repo: &crate::repo::Repo,
    name: &str,
    tree: &Path,
    window: i64,
) -> Result<String, String> {
    let pins = r9_pins::pins::PinsFile::load(&repo.root)?
        .ok_or("there is no pins.toml; run r9 sets --write")?;
    let release = pins
        .release
        .get(name)
        .ok_or_else(|| format!("no release {name} in pins.toml"))?;
    let front = pins
        .tree
        .values()
        .next()
        .ok_or("pins.toml has no tree to walk")?;
    let history = history(&repo.cache, &front.repo)?;
    let day = days(&release.date)?;
    let files = if tree.is_file() {
        hash_archive(tree, PREFIXES)?
    } else {
        hash_tree(tree, PREFIXES)?
    };
    if files.is_empty() {
        return Err(format!("{} has no sys/src or sys/include", tree.display()));
    }
    let located = locate(
        &history,
        &front.branch,
        &date(day - window),
        &date(day + 1),
        &files,
        PREFIXES,
    )?;
    Ok(located.report(20))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> Files {
        pairs
            .iter()
            .map(|(p, h)| ((*p).to_string(), (*h).to_string()))
            .collect()
    }

    #[test]
    fn dates_go_to_days_and_back() {
        assert_eq!(days("1970-01-01").unwrap(), 0);
        assert_eq!(days("2026-08-02").unwrap() - days("2026-07-31").unwrap(), 2);
        assert_eq!(date(days("2024-03-01").unwrap() - 1), "2024-02-29");
        assert_eq!(date(days("2026-08-02").unwrap() - 90), "2026-05-04");
        assert!(days("2026-13-01").is_err());
    }

    #[test]
    fn a_diff_lists_each_kind() {
        let tree = files(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let commit = files(&[("a", "1"), ("b", "9"), ("d", "4")]);
        let d = diff(&tree, &commit);
        assert_eq!(d.only_tree, ["c"]);
        assert_eq!(d.only_commit, ["d"]);
        assert_eq!(d.changed, ["b"]);
        assert_eq!(d.len(), 3);
        assert!(diff(&tree, &tree).is_empty());
    }

    #[test]
    fn a_tree_hashes_as_git_does() {
        let d = std::env::temp_dir().join(format!("r9-locate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sys/src")).unwrap();
        std::fs::write(d.join("sys/src/hello"), "hello\n").unwrap();
        let f = hash_tree(&d, &["sys/src", "sys/missing"]).unwrap();
        // git hash-object of "hello\n".
        assert_eq!(
            f.get("sys/src/hello").map(String::as_str),
            Some("ce013625030ba8dba906f756967f9e9ca394464a")
        );
        assert_eq!(f.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
