//! `r9 9h verify`: the `9h` kernel uses the sources of the pinned tree with no change (plan 10.2).
//!
//! The kernel is built in the tree's arch directory, with the files of `9h/<arch>` bound after it.
//! The check has two parts. The first part reads only the trees:
//! - no file in `9h/<arch>` has the name of a file in `port/`, in the arch directory, or in `pc/`
//!   for amd64, whose mkfile compiles the files of `pc/` by name, so the union cannot hide a tree
//!   file;
//! - no device of the tree uses the character `Z`, which `#Z` of the 9h kernel needs.
//!
//! The second part reads the `compile.jsonl` of the build, when it is given. Each source that
//! the compiler read in `/sys/src/9` must be a file of the tree with the pin's hash, a file of
//! `9h/<arch>` with its hash, or a file that the build makes (`$CONF.c`, `errstr.h`, the `.i`
//! files). For amd64, a header that the mkfile copies from `pc/` must have the hash of the pin's
//! file in `pc/`.

use crate::census::Record;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The device character of the 9h devices, `#Z`.
pub const DEVCHAR: char = 'Z';

/// The tree's kernel directory for an objtype.
#[must_use]
pub fn archdir(objtype: &str) -> &str {
    match objtype {
        "amd64" => "pc64",
        o => o,
    }
}

/// The tree directories whose files the kernel of `objtype` compiles by name.
#[must_use]
pub fn treedirs(objtype: &str) -> Vec<&str> {
    match archdir(objtype) {
        "pc64" => vec!["port", "pc64", "pc"],
        arch => vec!["port", arch],
    }
}

/// The files that the mkfiles make in the arch directory, other than those named for a config.
const MADE: [&str; 6] = ["cfs.h", "fs.h", "conf.h", "errstr.h", "init.h", "reboot.h"];

/// A file that the build makes in the arch directory: a file of `MADE`, a `.i` file, or a file
/// named for a config, as `9h.c`, `9h.rootc.c` and `boot9h.c` are.
fn made(name: &str, confs: &BTreeSet<String>) -> bool {
    let stem = name.split('.').next().unwrap_or("");
    MADE.contains(&name)
        || Path::new(name).extension().is_some_and(|e| e == "i")
        || (stem != name
            && (confs.contains(stem)
                || stem.strip_prefix("boot").is_some_and(|c| confs.contains(c))))
}

/// The result of the check.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// What is wrong, one line each.
    pub problems: Vec<String>,
    /// The tree sources that have the pin's hash.
    pub tree: usize,
    /// The 9h sources that have their hash.
    pub h9: BTreeSet<String>,
    /// The sources that the build made.
    pub generated: usize,
}

impl Verdict {
    /// The report, with PASS or FAIL on the first line.
    #[must_use]
    pub fn report(&self, records: bool) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "{} 9h verify",
            if self.problems.is_empty() {
                "PASS"
            } else {
                "FAIL"
            }
        );
        if records {
            let _ = writeln!(
                s,
                "sources: {} of the tree, {} of 9h, {} made by the build",
                self.tree,
                self.h9.len(),
                self.generated
            );
        } else {
            let _ = writeln!(s, "sources: not checked, because no records were given");
        }
        for p in &self.problems {
            let _ = writeln!(s, "  {p}");
        }
        s
    }
}

fn names(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// A C file. Plan 9 names are case-sensitive, so `.C` is not one.
fn is_c(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| e == "c")
}

fn sha256(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(
        Sha256::digest(&bytes)
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            }),
    )
}

/// The device characters of the `Dev` tables in the C files of `dir`, with the file of each.
#[must_use]
pub fn devchars(dir: &Path) -> BTreeMap<char, String> {
    let mut out = BTreeMap::new();
    for name in names(dir).into_iter().filter(|n| is_c(n)) {
        let Ok(text) = std::fs::read_to_string(dir.join(&name)) else {
            continue;
        };
        let mut rest = text.as_str();
        while let Some(at) = rest.find("Dev ") {
            rest = &rest[at + 4..];
            let line_start = text.len() - rest.len() - 4;
            if line_start > 0 && !text[..line_start].ends_with('\n') {
                continue;
            }
            let Some(eq) = rest.find('=') else { break };
            let head = &rest[..eq];
            if head.contains(['(', ';', '\n']) {
                continue;
            }
            let body = rest[eq + 1..].trim_start();
            let Some(body) = body.strip_prefix('{') else {
                continue;
            };
            let body = body.trim_start();
            let mut cs = body.chars();
            if cs.next() == Some('\'')
                && let (Some(c), Some('\'')) = (cs.next(), cs.next())
            {
                out.entry(c).or_insert_with(|| name.clone());
            }
        }
    }
    out
}

/// The kernel sources of the pinned tree, which `r9 9h verify` reads when it has no `--tree`.
/// Write `sys/src/9` of the first tree of `pins.toml` from the history clone in the cache to
/// `dir`, and return `dir`. The check reads only `sys/src/9`, so only those blobs are fetched.
pub fn pinned_tree(repo: &crate::repo::Repo, dir: &Path) -> Result<PathBuf, String> {
    let pins = r9_pins::pins::PinsFile::load(&repo.root)?
        .ok_or("there is no pins.toml; run r9 sets --write")?;
    let (_, tree) = pins.tree.iter().next().ok_or("pins.toml has no tree")?;
    let history = crate::locate::history(&repo.cache, &tree.repo)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tar = dir.join("kernel.tar");
    crate::image::archive(&history, &tree.commit, &["sys/src/9".to_string()], &tar)?;
    let status = std::process::Command::new("tar")
        .arg("xf")
        .arg(&tar)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar cannot read {}", tar.display()));
    }
    std::fs::remove_file(&tar).map_err(|e| format!("{}: {e}", tar.display()))?;
    Ok(dir.to_path_buf())
}

/// The part of the check that reads only the trees. `tree` is the root of the pinned tree, and
/// `h9` is the `9h` directory of this repository.
#[must_use]
pub fn check_trees(tree: &Path, h9: &Path, objtype: &str) -> Verdict {
    let mut v = Verdict::default();
    let k = tree.join("sys/src/9");
    let ours = names(&h9.join(objtype));
    if ours.is_empty() {
        v.problems
            .push(format!("{} has no files", h9.join(objtype).display()));
    }
    for d in treedirs(objtype) {
        let theirs = names(&k.join(d));
        if theirs.is_empty() {
            v.problems.push(format!(
                "{} does not have {d}, so it is not the root of a tree",
                tree.display()
            ));
        }
        for n in ours.intersection(&theirs) {
            v.problems
                .push(format!("9h/{objtype}/{n} has the name of {d}/{n}"));
        }
        if let Some(f) = devchars(&k.join(d)).get(&DEVCHAR) {
            v.problems.push(format!(
                "{d}/{f} has a device with the character {DEVCHAR}, which the 9h kernel needs"
            ));
        }
    }
    v
}

/// Make a path in the guest normal: remove `.` and resolve `..`.
#[must_use]
pub fn clean(dir: &str, file: &str) -> String {
    let joined = if file.starts_with('/') {
        file.to_string()
    } else {
        format!("{dir}/{file}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The part of the check that reads the records of the build.
pub fn check_records(v: &mut Verdict, records: &[Record], tree: &Path, h9: &Path, objtype: &str) {
    let arch = archdir(objtype);
    let archpath = format!("/sys/src/9/{arch}");
    let ours = names(&h9.join(objtype));
    let confs: BTreeSet<String> = names(&tree.join("sys/src/9").join(arch))
        .into_iter()
        .chain(ours.iter().cloned())
        .filter(|n| !n.contains('.'))
        .collect();
    let mut seen = BTreeSet::new();
    for r in records.iter().filter(|r| r.objtype == objtype) {
        for f in &r.inputs {
            let path = clean(&r.dir, &f.file);
            if !path.starts_with("/sys/src/9/") || !seen.insert((path.clone(), f.sha256.clone())) {
                continue;
            }
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", &path));
            if dir == archpath && ours.contains(name) {
                if sha256(&h9.join(objtype).join(name)).as_deref() == Some(f.sha256.as_str()) {
                    v.h9.insert(name.to_string());
                } else {
                    v.problems
                        .push(format!("{path} is not the same as 9h/{objtype}/{name}"));
                }
                continue;
            }
            if dir == archpath && made(name, &confs) {
                v.generated += 1;
                continue;
            }
            let mut host: PathBuf = tree.join(path.trim_start_matches('/'));
            if dir == archpath && arch == "pc64" && !host.exists() {
                // A header of PCHEADERS, which the pc64 mkfile copies from pc.
                host = tree.join("sys/src/9/pc").join(name);
            }
            match sha256(&host) {
                Some(h) if h == f.sha256 => v.tree += 1,
                Some(_) => v
                    .problems
                    .push(format!("{path} is not the file of the pin")),
                None => v
                    .problems
                    .push(format!("{path} is not in the tree or in 9h")),
            }
        }
    }
    let wanted: Vec<&String> = ours.iter().filter(|n| is_c(n)).collect();
    if v.h9.is_empty() {
        v.problems.push(format!(
            "the records have no unit of 9h/{objtype}, so the 9h kernel was not built"
        ));
    } else {
        for n in wanted {
            if !v.h9.contains(n) {
                v.problems
                    .push(format!("the records have no unit for 9h/{objtype}/{n}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_records, check_trees, clean, devchars, made};
    use crate::census::{File, Record};
    use std::path::{Path, PathBuf};

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("r9-h9verify-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn put(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn record(dir: &str, file: &str, text: &str) -> Record {
        record_for("arm64", dir, file, text)
    }

    fn record_for(objtype: &str, dir: &str, file: &str, text: &str) -> Record {
        use sha2::Digest;
        use std::fmt::Write as _;
        let h = sha2::Sha256::digest(text.as_bytes())
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
        Record {
            cc: "7c".into(),
            objtype: objtype.into(),
            dir: dir.into(),
            argv: vec![file.into()],
            inputs: vec![File {
                file: file.into(),
                sha256: h,
            }],
            outputs: vec![],
            stdin: false,
            status: String::new(),
            ms: 0,
            user_ms: 0,
            sys_ms: 0,
            origin: "rucc".into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn paths_are_made_normal() {
        assert_eq!(
            clean("/sys/src/9/arm64", "../port/proc.c"),
            "/sys/src/9/port/proc.c"
        );
        assert_eq!(
            clean("/sys/src/9/arm64", "./main.c"),
            "/sys/src/9/arm64/main.c"
        );
        assert_eq!(clean("/x", "/sys/a.c"), "/sys/a.c");
    }

    #[test]
    fn the_device_characters_are_found() {
        let d = dir("dev");
        put(
            &d,
            "devcons.c",
            "x\nDev consdevtab = {\n\t'c',\n\t\"cons\",\n};\n",
        );
        put(
            &d,
            "devx.c",
            "extern Dev xdevtab;\nDev xdevtab = { 'Z', \"x\" };\n",
        );
        let c = devchars(&d);
        assert_eq!(c.get(&'c').map(String::as_str), Some("devcons.c"));
        assert_eq!(c.get(&'Z').map(String::as_str), Some("devx.c"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_trees_and_the_records_are_checked() {
        let tree = dir("tree");
        let h9 = dir("h9");
        put(&tree, "sys/src/9/port/proc.c", "proc");
        put(
            &tree,
            "sys/src/9/port/devcons.c",
            "Dev consdevtab = {\n\t'c',\n};\n",
        );
        put(&tree, "sys/src/9/arm64/main.c", "main");
        put(&tree, "sys/src/9/arm64/qemu", "conf");
        put(&h9, "arm64/9h", "conf");
        put(&h9, "arm64/sdh9.c", "sd");
        let v = check_trees(&tree, &h9, "arm64");
        assert!(v.problems.is_empty(), "{:?}", v.problems);

        let good = [
            record("/sys/src/9/arm64", "../port/proc.c", "proc"),
            record("/sys/src/9/arm64", "main.c", "main"),
            record("/sys/src/9/arm64", "sdh9.c", "sd"),
            record("/sys/src/9/arm64", "9h.c", "made"),
            record("/sys/src/cmd", "ls.c", "other"),
        ];
        let mut v = check_trees(&tree, &h9, "arm64");
        check_records(&mut v, &good, &tree, &h9, "arm64");
        assert!(v.problems.is_empty(), "{:?}", v.problems);
        assert_eq!((v.tree, v.h9.len(), v.generated), (2, 1, 1));
        assert!(v.report(true).starts_with("PASS"));

        let bad = [
            record("/sys/src/9/arm64", "main.c", "changed"),
            record("/sys/src/9/arm64", "new.c", "new"),
        ];
        let mut v = check_trees(&tree, &h9, "arm64");
        check_records(&mut v, &bad, &tree, &h9, "arm64");
        assert_eq!(v.problems.len(), 3, "{:?}", v.problems);
        assert!(v.report(true).starts_with("FAIL"));

        put(&h9, "arm64/main.c", "mine");
        put(
            &tree,
            "sys/src/9/port/devz.c",
            "Dev zdevtab = {\n\t'Z',\n};\n",
        );
        let v = check_trees(&tree, &h9, "arm64");
        assert_eq!(v.problems.len(), 2, "{:?}", v.problems);
        let _ = std::fs::remove_dir_all(&tree);
        let _ = std::fs::remove_dir_all(&h9);
    }

    #[test]
    fn the_made_files_are_known() {
        let confs = ["9h".to_string(), "mkfile".to_string()].into();
        for n in [
            "9h.c",
            "9h.rootc.c",
            "9h.root.s",
            "boot9h.c",
            "errstr.h",
            "apbootstrap.i",
        ] {
            assert!(made(n, &confs), "{n}");
        }
        for n in ["main.c", "9h", "mp.h", "x9h.c"] {
            assert!(!made(n, &confs), "{n}");
        }
    }

    #[test]
    fn amd64_builds_in_pc64_with_the_files_of_pc() {
        let tree = dir("tree64");
        let h9 = dir("h964");
        put(&tree, "sys/src/9/port/proc.c", "proc");
        put(&tree, "sys/src/9/pc64/main.c", "main");
        put(&tree, "sys/src/9/pc64/pc64", "conf");
        put(&tree, "sys/src/9/pc/cga.c", "cga");
        put(&tree, "sys/src/9/pc/io.h", "io");
        put(&h9, "amd64/9h", "conf");
        put(&h9, "amd64/h9.c", "h9");
        let v = check_trees(&tree, &h9, "amd64");
        assert!(v.problems.is_empty(), "{:?}", v.problems);

        let pc64 = "/sys/src/9/pc64";
        let good = [
            record_for("amd64", pc64, "../pc/cga.c", "cga"),
            record_for("amd64", pc64, "main.c", "main"),
            record_for("amd64", pc64, "io.h", "io"),
            record_for("amd64", pc64, "apbootstrap.i", "made"),
            record_for("amd64", pc64, "9h.rootc.c", "made"),
            record_for("amd64", pc64, "h9.c", "h9"),
        ];
        let mut v = check_trees(&tree, &h9, "amd64");
        check_records(&mut v, &good, &tree, &h9, "amd64");
        assert!(v.problems.is_empty(), "{:?}", v.problems);
        assert_eq!((v.tree, v.h9.len(), v.generated), (3, 1, 2));

        let bad = [
            record_for("amd64", pc64, "io.h", "changed"),
            record_for("amd64", pc64, "h9.c", "h9"),
        ];
        let mut v = check_trees(&tree, &h9, "amd64");
        check_records(&mut v, &bad, &tree, &h9, "amd64");
        assert_eq!(v.problems.len(), 1, "{:?}", v.problems);

        put(&h9, "amd64/cga.c", "mine");
        let v = check_trees(&tree, &h9, "amd64");
        assert_eq!(v.problems, ["9h/amd64/cga.c has the name of pc/cga.c"]);
        let _ = std::fs::remove_dir_all(&tree);
        let _ = std::fs::remove_dir_all(&h9);
    }

    /// The drivers that do not change with the machine are the same file in both directories.
    #[test]
    fn the_shared_drivers_are_the_same() {
        let h9 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../9h");
        for f in ["sdh9.c", "devhost.c", "h9mouse.c"] {
            let a = std::fs::read(h9.join("arm64").join(f)).unwrap();
            let b = std::fs::read(h9.join("amd64").join(f)).unwrap();
            assert!(a == b, "9h/arm64/{f} and 9h/amd64/{f} are not the same");
        }
    }
}
