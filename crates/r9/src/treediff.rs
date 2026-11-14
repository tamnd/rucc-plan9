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
//!
//! Some archives get two members with the same name from two directories. `libap.a` of APE gets
//! `strtod.7` from `ap/gen` and from `ap/stdio`. mk installs both with `ar vu`, and `ar u` keeps
//! the old member when the new file is not newer than the date of the member. Dates have a
//! resolution of one second. Thus the member that the archive keeps depends on whether the two
//! compiles end in the same second, and the programs that link the archive differ too. These
//! files are in the raced group. They are not a failure of the compiler, but they are reported.
//!
//! The R build and the G build of the pin use two compilers, so their files have different
//! contents. With `--names`, `r9 tree-diff` compares only the names and the modes of the files
//! that the two builds wrote (plan 02.2). The modes come from the `walk` file of each build.

use std::collections::{BTreeMap, BTreeSet};
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
    /// The members, as `archive(member)`, that two or more `ar` commands of the build put in the
    /// same archive.
    pub twice: BTreeSet<String>,
    /// The directories of the APE units: the units that `pcc` sends to the compiler on standard
    /// input, and the units with an include directory of APE. The headers of APE add `libap.a`
    /// with `#pragma lib`, so the programs of these directories link it.
    pub ape_dirs: BTreeSet<String>,
    /// The mode of each file that the build wrote, as `walk -e x` prints it. A build that was
    /// collected before the walk had the mode has no modes.
    pub modes: Hashes,
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
    /// The archives whose different members are all members that two `ar` commands wrote, and
    /// the APE programs that link such an archive.
    pub raced: Vec<String>,
    /// The members that make the archives of `raced` differ.
    pub raced_members: Vec<String>,
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
    let log = std::fs::read_to_string(dir.join("mk.log")).unwrap_or_default();
    let compile = std::fs::read_to_string(dir.join("compile.jsonl")).unwrap_or_default();
    let walk = std::fs::read_to_string(dir.join("walk")).unwrap_or_default();
    let files = parse(&text);
    let modes = modes(&walk)
        .into_iter()
        .filter(|(p, _)| files.contains_key(p))
        .collect();
    Ok(Outputs {
        files,
        modes,
        members: parse(&members),
        twice: twice(&log),
        ape_dirs: ape_dirs(&compile),
    })
}

/// The mode of each file in a walk with the fields `msxp`: the time, the size, the mode and the
/// path. A walk with the old fields `msp` has no mode, and gives no lines.
#[must_use]
pub fn modes(walk: &str) -> Hashes {
    walk.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(4, char::is_whitespace).collect();
            (f.len() == 4 && f[2].len() == 11 && f[2].ends_with(['-', 'r', 'w', 'x']))
                .then(|| (f[3].trim().to_string(), f[2].to_string()))
        })
        .collect()
}

/// How the names and the modes of two builds differ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    /// The number of names in both builds.
    pub same: usize,
    /// The files that the first build wrote and the second did not.
    pub only_a: Vec<String>,
    /// The files that the second build wrote and the first did not.
    pub only_b: Vec<String>,
    /// The files with two modes, as `path A-mode B-mode`.
    pub modes: Vec<String>,
    /// The number of files in both builds that have a mode in both walks.
    pub with_mode: usize,
}

impl Names {
    /// True when the builds wrote the same names with the same modes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.only_a.is_empty() && self.only_b.is_empty() && self.modes.is_empty()
    }
}

/// Compare the names and the modes of the files of two builds.
#[must_use]
pub fn names(a: &Outputs, b: &Outputs) -> Names {
    let mut n = Names::default();
    for path in a.files.keys() {
        if !b.files.contains_key(path) {
            n.only_a.push(path.clone());
            continue;
        }
        n.same += 1;
        if let (Some(ma), Some(mb)) = (a.modes.get(path), b.modes.get(path)) {
            n.with_mode += 1;
            if ma != mb {
                n.modes.push(format!("{path} {ma} {mb}"));
            }
        }
    }
    n.only_b = b
        .files
        .keys()
        .filter(|p| !a.files.contains_key(*p))
        .cloned()
        .collect();
    n
}

/// The report of a comparison of names. At most `top` names are printed in each group.
#[must_use]
pub fn report_names(n: &Names, top: usize) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "same names {}", n.same);
    let _ = writeln!(s, "with mode  {}", n.with_mode);
    for (title, list) in [
        ("only in A", &n.only_a),
        ("only in B", &n.only_b),
        ("mode", &n.modes),
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
        if !n.is_empty() {
            "the names of the builds differ"
        } else if n.with_mode < n.same {
            "the builds wrote the same names, but some files have no mode in a walk"
        } else {
            "the builds wrote the same names with the same modes"
        }
    );
    s
}

/// The members that two or more `ar` commands in the mk log put in the same archive, as
/// `archive(member)`. The commands are the lines that start with `ar` and a key with `r` or `u`.
#[must_use]
pub fn twice(log: &str) -> BTreeSet<String> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for line in log.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("ar") {
            continue;
        }
        let (Some(key), Some(archive)) = (words.next(), words.next()) else {
            continue;
        };
        if !key.chars().all(|c| "rvuc".contains(c)) || !(key.contains('r') || key.contains('u')) {
            continue;
        }
        for member in words {
            *count.entry(format!("{archive}({member})")).or_default() += 1;
        }
    }
    count
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|(k, _)| k)
        .collect()
}

/// The directories of the compile records that read standard input or that give an include
/// directory of APE.
#[must_use]
pub fn ape_dirs(compile: &str) -> BTreeSet<String> {
    compile
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|r| {
            r["stdin"].as_bool() == Some(true)
                || r["argv"].as_array().is_some_and(|a| {
                    a.iter()
                        .filter_map(serde_json::Value::as_str)
                        .any(|w| w.starts_with("-I") && w.ends_with("/include/ape"))
                })
        })
        .filter_map(|r| r["dir"].as_str().map(str::to_string))
        .collect()
}

/// The members of `archive` that differ between `a` and `b`, or that only one of them has.
/// `__.SYMDEF` is left out, because `ar` writes it from the other members.
fn changed_members(a: &Hashes, b: &Hashes, archive: &str) -> Vec<String> {
    let ma: BTreeMap<_, _> = members_of(a, archive).into_iter().collect();
    let mb: BTreeMap<_, _> = members_of(b, archive).into_iter().collect();
    let names: BTreeSet<&str> = ma.keys().chain(mb.keys()).copied().collect();
    names
        .into_iter()
        .filter(|n| !n.ends_with("(__.SYMDEF)") && ma.get(n) != mb.get(n))
        .map(str::to_string)
        .collect()
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
    raced(a, b, &mut d);
    d.only_b = b
        .files
        .keys()
        .filter(|p| !a.files.contains_key(*p))
        .cloned()
        .collect();
    d
}

/// Move the raced archives and the APE programs that follow them from `changed` to `raced`.
fn raced(a: &Outputs, b: &Outputs, d: &mut Diff) {
    let mut ape = false;
    let mut keep = Vec::new();
    for path in std::mem::take(&mut d.changed) {
        let members = if is_archive(&path) {
            changed_members(&a.members, &b.members, &path)
        } else {
            Vec::new()
        };
        if !members.is_empty()
            && members
                .iter()
                .all(|m| a.twice.contains(m) && b.twice.contains(m))
        {
            ape |= path.contains("/lib/ape/");
            d.raced_members.extend(members);
            d.raced.push(path);
        } else {
            keep.push(path);
        }
    }
    if ape {
        let in_ape_dir = |path: &str, dirs: &BTreeSet<String>| {
            Path::new(path)
                .parent()
                .and_then(Path::to_str)
                .is_some_and(|dir| dirs.contains(dir))
        };
        let programs: Vec<String> = keep
            .iter()
            .filter(|p| in_ape_dir(p, &a.ape_dirs) && in_ape_dir(p, &b.ape_dirs))
            .cloned()
            .collect();
        let hashes = |files: &Hashes| -> BTreeSet<String> {
            programs
                .iter()
                .filter_map(|p| files.get(p).cloned())
                .collect()
        };
        let (ha, hb) = (hashes(&a.files), hashes(&b.files));
        let (follow, rest): (Vec<String>, Vec<String>) = keep.into_iter().partition(|p| {
            programs.contains(p)
                || (a.files.get(p).is_some_and(|h| ha.contains(h))
                    && b.files.get(p).is_some_and(|h| hb.contains(h)))
        });
        d.raced.extend(follow);
        d.raced.sort();
        keep = rest;
    }
    d.changed = keep;
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
        ("raced", &d.raced),
    ] {
        let _ = writeln!(s, "{title:<10} {}", list.len());
        for p in list.iter().take(top) {
            let _ = writeln!(s, "  {p}");
        }
        if list.len() > top {
            let _ = writeln!(s, "  and {} more", list.len() - top);
        }
    }
    for m in &d.raced_members {
        let _ = writeln!(s, "  raced by ar u: {m}");
    }
    let _ = writeln!(
        s,
        "{}",
        if !d.is_empty() {
            "the builds differ"
        } else if d.raced.is_empty() {
            "the builds are the same"
        } else {
            "the builds are the same, except for the members that ar u keeps by the time of the build"
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
            ..Outputs::default()
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
    fn the_names_and_the_modes_are_compared() {
        let walk = "1794498647 27618 --rw-rw-r-- /sys/src/x.6\n\
                    1794498647 10 --rwxrwxr-x /amd64/bin/x\n\
                    1794498647 19279 /sys/src/9/ip/ip.h\n";
        let m = modes(walk);
        assert_eq!(m.len(), 2);
        assert_eq!(m["/amd64/bin/x"], "--rwxrwxr-x");
        let mut a = outputs("1\t/sys/src/x.6\n2\t/amd64/bin/x\n3\t/amd64/bin/y\n", "");
        let mut b = outputs("4\t/sys/src/x.6\n5\t/amd64/bin/x\n6\t/amd64/bin/z\n", "");
        a.modes = m.clone();
        b.modes = m;
        let n = names(&a, &b);
        assert_eq!(n.same, 2);
        assert_eq!(n.with_mode, 2);
        assert_eq!(n.only_a, ["/amd64/bin/y"]);
        assert_eq!(n.only_b, ["/amd64/bin/z"]);
        assert!(n.modes.is_empty());
        b.modes.insert("/amd64/bin/x".into(), "--rw-rw-r--".into());
        let n = names(&a, &b);
        assert_eq!(n.modes, ["/amd64/bin/x --rwxrwxr-x --rw-rw-r--"]);
        assert!(report_names(&n, 5).ends_with("the names of the builds differ\n"));
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

    #[test]
    fn a_member_from_two_ar_commands_is_found() {
        let log = "ar vu /arm64/lib/ape/libap.a abs.7 strtod.7\nr - strtod.7\n\
                   ar vu /arm64/lib/ape/libap.a fopen.7 strtod.7\nar t x.a\n";
        let t = twice(log);
        assert_eq!(
            t.into_iter().collect::<Vec<_>>(),
            ["/arm64/lib/ape/libap.a(strtod.7)"]
        );
        let c = "{\"dir\":\"/sys/src/cmd/eqn\",\"stdin\":true}\n\
                 {\"dir\":\"/sys/src/cmd/cat\",\"stdin\":false,\"argv\":[\"-I/sys/include\"]}\n\
                 {\"dir\":\"/a/w\",\"stdin\":false,\"argv\":[\"-Bp\",\"-I/sys/include/ape\"]}\n";
        assert_eq!(
            ape_dirs(c).into_iter().collect::<Vec<_>>(),
            ["/a/w", "/sys/src/cmd/eqn"]
        );
    }

    #[test]
    fn a_raced_archive_and_its_programs_are_not_changed() {
        let lib = "/arm64/lib/ape/libap.a";
        let race = |s: &str, m: &str| {
            let mut o = outputs(
                &format!(
                    "{s}\t{lib}\n{s}\t/sys/src/cmd/eqn/7.out\n{s}\t/arm64/bin/eqn\n1\t/arm64/bin/cat\n"
                ),
                &format!("{m}\t{lib}(strtod.7)\n{m}\t{lib}(__.SYMDEF)\n2\t{lib}(abs.7)\n"),
            );
            o.twice.insert(format!("{lib}(strtod.7)"));
            o.ape_dirs.insert("/sys/src/cmd/eqn".into());
            o
        };
        let (a, b) = (race("5", "6"), race("7", "8"));
        let d = diff(&a, &b);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(d.raced, ["/arm64/bin/eqn", lib, "/sys/src/cmd/eqn/7.out"]);
        assert_eq!(d.raced_members, [format!("{lib}(strtod.7)")]);
        assert!(report(&d, 5).contains("except for the members that ar u keeps"));

        let mut c = race("7", "8");
        c.twice.clear();
        let d = diff(&a, &c);
        assert_eq!(d.changed.len(), 3, "{d:?}");
        assert!(d.raced.is_empty());
    }
}
