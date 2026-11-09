//! `r9 tree-diff`: compare the outputs of two builds (plan 05.5).
//!
//! A build collects `changed.sha256`, the hash of each file that the build wrote. Two builds of
//! the same tree with the same compiler must give the same list: the same files with the same
//! hashes. A file that is in one list only is a failure, as is a file with two hashes.
//!
//! An archive records the date of each member, so the same archive from two builds has two
//! hashes. When a build also collects `members.sha256`, the hash of each member of each archive,
//! an archive whose members are the same in both builds is not a difference.
//!
//! The mkfile of cwfs compiles `conf.c` with the date of the build, so cwfs also differs from
//! build to build. Its files are in the dated group too.
//!
//! The mkfile of gs runs `genarch` to write `$objtype.h`. `genarch` times `memset` on blocks of
//! growing size to find the sizes of the caches, so the header differs from run to run on the
//! same machine. It is in the dated group too.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Path to hash.
pub type Hashes = BTreeMap<String, String>;

/// The outputs of one build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outputs {
    /// The hash of each file that the build wrote.
    pub files: Hashes,
    /// The hash of each member of each archive, keyed by `archive(member)`.
    pub members: Hashes,
}

/// How two builds differ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// The files that the first build wrote and the second did not.
    pub only_a: Vec<String>,
    /// The files that the second build wrote and the first did not.
    pub only_b: Vec<String>,
    /// The files with different contents.
    pub changed: Vec<String>,
    /// The files that differ only by the date or the speed of the build: archives with the same
    /// members, the files of cwfs, and the header that gs measures the caches into.
    pub dated: Vec<String>,
    /// The files in both builds.
    pub same: usize,
}

impl Diff {
    /// True when the builds wrote the same files with the same contents.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.only_a.is_empty() && self.only_b.is_empty() && self.changed.is_empty()
    }
}

/// Parse a list of `hash path` lines, as `sha1sum -2 256` writes them. The files of the rig
/// under `/tmp/r9` are not outputs of the build, and are left out.
#[must_use]
pub fn parse(text: &str) -> Hashes {
    text.lines()
        .filter_map(|l| {
            let (hash, path) = l.split_once(char::is_whitespace)?;
            let path = path.trim();
            (!path.starts_with("/tmp/r9/") && !hash.is_empty())
                .then(|| (path.to_string(), hash.to_string()))
        })
        .collect()
}

/// The `changed.sha256` and `members.sha256` of a build. `path` is the directory of the build,
/// its `collected` directory, or the `changed.sha256` file itself.
pub fn load(path: &Path) -> Result<Outputs, String> {
    let dir: PathBuf = if path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    } else if path.join("collected").is_dir() {
        path.join("collected")
    } else {
        path.to_path_buf()
    };
    let changed = if path.is_file() {
        path.to_path_buf()
    } else {
        dir.join("changed.sha256")
    };
    let text =
        std::fs::read_to_string(&changed).map_err(|e| format!("{}: {e}", changed.display()))?;
    let members = std::fs::read_to_string(dir.join("members.sha256")).unwrap_or_default();
    Ok(Outputs {
        files: parse(&text),
        members: parse(&members),
    })
}

/// The members of `archive` in `members`.
fn members_of<'a>(members: &'a Hashes, archive: &str) -> Vec<(&'a str, &'a str)> {
    let prefix = format!("{archive}(");
    members
        .range(prefix.clone()..)
        .take_while(|(k, _)| k.starts_with(&prefix))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// True when `path` is an archive: `lib.a` for the install, or `lib.a7` for a library local to
/// a command.
fn is_archive(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == "a" || (e.len() == 2 && e.starts_with('a')))
}

/// True when `path` holds the date of the build by design. The cwfs mkfile compiles `conf.c`
/// with the output of `date -n` in `DATE`, and links it into `$O.cwfs*`, which is installed as `cwfs*`.
fn has_build_date(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if path.starts_with("/sys/src/cmd/cwfs/") {
        return name.starts_with("conf.") || name.contains(".cwfs");
    }
    path.contains("/bin/") && name.starts_with("cwfs")
}

/// The architectures of 9front, as `objtype` names them.
const OBJTYPES: &[&str] = &[
    "386", "68000", "68020", "alpha", "amd64", "arm", "arm64", "mips", "power", "power64", "sparc",
    "spim",
];

/// True when `path` is the header that `genarch` writes for gs, `/sys/src/cmd/gs/$objtype.h`.
/// Its cache sizes come from a timing loop.
fn measures_the_machine(path: &str) -> bool {
    path.strip_prefix("/sys/src/cmd/gs/")
        .and_then(|name| name.strip_suffix(".h"))
        .is_some_and(|objtype| OBJTYPES.contains(&objtype))
}

/// Compare two builds.
#[must_use]
pub fn diff(a: &Outputs, b: &Outputs) -> Diff {
    let mut d = Diff::default();
    for (path, hash) in &a.files {
        match b.files.get(path) {
            None => d.only_a.push(path.clone()),
            Some(h) if h == hash => d.same += 1,
            Some(_) => {
                let ma = members_of(&a.members, path);
                if has_build_date(path)
                    || measures_the_machine(path)
                    || (is_archive(path) && !ma.is_empty() && ma == members_of(&b.members, path))
                {
                    d.dated.push(path.clone());
                } else {
                    d.changed.push(path.clone());
                }
            }
        }
    }
    d.only_b = b
        .files
        .keys()
        .filter(|p| !a.files.contains_key(*p))
        .cloned()
        .collect();
    d
}

/// The report of a comparison. At most `top` names are printed in each group.
#[must_use]
pub fn report(d: &Diff, top: usize) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "same       {}", d.same);
    for (title, list) in [
        ("only in A", &d.only_a),
        ("only in B", &d.only_b),
        ("changed", &d.changed),
        ("dated", &d.dated),
    ] {
        let _ = writeln!(s, "{title:<10} {}", list.len());
        for p in list.iter().take(top) {
            let _ = writeln!(s, "  {p}");
        }
        if list.len() > top {
            let _ = writeln!(s, "  and {} more", list.len() - top);
        }
    }
    let _ = writeln!(
        s,
        "{}",
        if d.is_empty() {
            "the builds are the same"
        } else {
            "the builds differ"
        }
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs(files: &str, members: &str) -> Outputs {
        Outputs {
            files: parse(files),
            members: parse(members),
        }
    }

    #[test]
    fn the_rig_files_are_left_out() {
        let h = parse("aa\t/tmp/r9/t0\nbb\t/sys/src/cmd/a.7\n\n");
        assert_eq!(h.len(), 1);
        assert_eq!(h["/sys/src/cmd/a.7"], "bb");
    }

    #[test]
    fn the_kinds_of_difference() {
        let a = outputs(
            "1\t/x.7\n2\t/y.7\n3\t/z.7\n4\t/l.a\n5\t/m.a\n",
            "8\t/l.a(p.7)\n9\t/m.a(q.7)\n",
        );
        let b = outputs(
            "1\t/x.7\n7\t/y.7\n6\t/w.7\n5\t/l.a\n4\t/m.a\n",
            "8\t/l.a(p.7)\n0\t/m.a(q.7)\n",
        );
        let d = diff(&a, &b);
        assert_eq!(d.same, 1);
        assert_eq!(d.only_a, ["/z.7"]);
        assert_eq!(d.only_b, ["/w.7"]);
        assert_eq!(d.changed, ["/m.a", "/y.7"]);
        assert_eq!(d.dated, ["/l.a"]);
        assert!(!d.is_empty());
        let r = report(&d, 1);
        assert!(r.contains("dated      1\n  /l.a\n"), "{r}");
        assert!(r.contains("changed    2\n  /m.a\n  and 1 more\n"), "{r}");
    }

    #[test]
    fn archives_and_cwfs_are_known() {
        assert!(is_archive("/arm64/lib/libc.a"));
        assert!(is_archive("/sys/src/cmd/cc/cc.a7"));
        assert!(!is_archive("/sys/src/cmd/cc/cc.h"));
        assert!(has_build_date("/arm64/bin/cwfs64x"));
        assert!(has_build_date("/sys/src/cmd/cwfs/cwfs64x/7.cwfs64x"));
        assert!(has_build_date("/sys/src/cmd/cwfs/cwfs64x/conf.7"));
        assert!(!has_build_date("/sys/src/cmd/cwfs/cwfs64x/main.7"));
        assert!(!has_build_date("/arm64/bin/cat"));
        assert!(measures_the_machine("/sys/src/cmd/gs/amd64.h"));
        assert!(!measures_the_machine("/sys/src/cmd/gs/arch.h"));
        assert!(!measures_the_machine("/sys/src/cmd/gs/src/amd64.h"));
    }

    #[test]
    fn dated_archives_alone_are_the_same() {
        let a = outputs("4\t/l.a\n", "8\t/l.a(p.7)\n");
        let b = outputs("5\t/l.a\n", "8\t/l.a(p.7)\n");
        assert!(diff(&a, &b).is_empty());
    }
}
