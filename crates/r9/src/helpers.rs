//! `r9 helpers`: the calls that the compiler makes by itself (plan 08.6).
//!
//! An object of G refers to an external symbol for one of two reasons. The source names the
//! symbol, or the compiler made the reference, such as a call of `_profin` or of a `vlong`
//! helper. `r9 helpers` lists each external symbol that the objects refer to and that no source
//! of the tree has as an identifier. That list is the set of calls that the compiler makes by
//! itself. rucc can call only these symbols, with the same convention as kencc.
//!
//! The sources are the files under the tree that end in `.c`, `.h`, `.s`, `.y`, `.l` or `.mp`. An
//! identifier is a word of letters, digits and `_` that does not start with a digit. The match is
//! on words, so a name in a comment or a string counts too. Thus the list can only be too short,
//! never too long.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::obj::{self, Object};

/// The extensions of the files that are sources. An `.mp` file is the input of mpc, which makes C
/// code with names such as `edwards_add`.
const SOURCES: &[&str] = &["c", "h", "s", "y", "l", "mp"];

/// Add each identifier of `text` to `set`.
pub fn identifiers(text: &[u8], set: &mut BTreeSet<String>) {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < text.len() {
        if !word(text[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < text.len() && word(text[i]) {
            i += 1;
        }
        if !text[start].is_ascii_digit() {
            set.insert(String::from_utf8_lossy(&text[start..i]).into_owned());
        }
    }
}

/// The files under `root`, in order, that `keep` accepts. Symbolic links are not followed.
fn walk(root: &Path, keep: &dyn Fn(&Path) -> bool) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for e in entries.filter_map(Result::ok) {
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            if t.is_dir() {
                dirs.push(p);
            } else if t.is_file() && keep(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The identifiers of the sources under `root`, and the number of source files.
pub fn source_identifiers(root: &Path) -> Result<(BTreeSet<String>, usize), String> {
    let files = walk(root, &|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SOURCES.contains(&e))
    })?;
    let mut set = BTreeSet::new();
    for f in &files {
        let text = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
        identifiers(&text, &mut set);
    }
    Ok((set, files.len()))
}

/// The external symbols that the operands of `o` name.
#[must_use]
pub fn references(o: &Object) -> Vec<String> {
    let extern_kind = match o.arch {
        obj::Arch::Amd64 => obj::d6::EXTERN,
        obj::Arch::Arm64 => obj::d7::EXTERN,
    };
    let mut names: Vec<String> = o
        .progs
        .iter()
        .flat_map(|p| [&p.from, &p.to])
        .filter_map(|a| a.sym.as_ref())
        .filter(|s| s.kind == extern_kind && !s.name.is_empty())
        .map(|s| s.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// One symbol that no source names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    /// The name of the symbol.
    pub name: String,
    /// The number of objects that refer to it.
    pub objects: usize,
    /// The first object that refers to it.
    pub first: String,
}

/// The symbols that `objects` refer to and that are not in `ids`, by name.
#[must_use]
pub fn helpers(objects: &[(String, Object)], ids: &BTreeSet<String>) -> Vec<Helper> {
    let mut found: BTreeMap<String, Helper> = BTreeMap::new();
    for (name, o) in objects {
        for r in references(o) {
            if ids.contains(&r) {
                continue;
            }
            found
                .entry(r.clone())
                .or_insert_with(|| Helper {
                    name: r,
                    objects: 0,
                    first: name.clone(),
                })
                .objects += 1;
        }
    }
    found.into_values().collect()
}

/// Decode the objects at `path`: one object or archive, or each of them under a directory.
pub fn load(path: &Path) -> Result<Vec<(String, Object)>, String> {
    let files = if path.is_dir() {
        walk(path, &|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "a" | "6" | "7"))
        })?
    } else {
        vec![path.to_path_buf()]
    };
    let mut out = Vec::new();
    for f in files {
        let name = f.display().to_string();
        let bytes = std::fs::read(&f).map_err(|e| format!("{name}: {e}"))?;
        for (member, o) in obj::objects(&name, &bytes)? {
            let n = if member.is_empty() {
                name.clone()
            } else {
                format!("{name}({member})")
            };
            out.push((n, o));
        }
    }
    Ok(out)
}

/// The report of `r9 helpers`.
#[must_use]
pub fn report(objects: usize, sources: usize, found: &[Helper]) -> String {
    let mut s = String::new();
    for h in found {
        let _ = writeln!(s, "{}\t{} objects\tfirst in {}", h.name, h.objects, h.first);
    }
    let _ = writeln!(
        s,
        "{} symbols that no source names, in {objects} objects and {sources} source files",
        found.len()
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obj::tests::amd64_object;

    #[test]
    fn identifiers_are_words_that_do_not_start_with_a_digit() {
        let mut set = BTreeSet::new();
        identifiers(b"x = _tos+2; 9p(a_b1); /* main */ TEXT\tf(SB)", &mut set);
        let mut v: Vec<&str> = set.iter().map(String::as_str).collect();
        v.sort_unstable();
        assert_eq!(v, ["SB", "TEXT", "_tos", "a_b1", "f", "main", "x"]);
    }

    #[test]
    fn a_symbol_that_no_source_names_is_a_helper() {
        // The object refers to main only, in its TEXT.
        let o = decode(&amd64_object());
        assert_eq!(references(&o), ["main"]);
        let objects = vec![("a.6".to_string(), o.clone()), ("b.6".to_string(), o)];
        let mut ids = BTreeSet::new();
        let found = helpers(&objects, &ids);
        assert_eq!(
            found,
            [Helper {
                name: "main".into(),
                objects: 2,
                first: "a.6".into()
            }]
        );
        assert!(
            report(2, 0, &found)
                .ends_with("1 symbols that no source names, in 2 objects and 0 source files\n")
        );
        ids.insert("main".into());
        assert!(helpers(&objects, &ids).is_empty());
    }

    fn decode(b: &[u8]) -> Object {
        obj::decode(obj::Arch::Amd64, b).unwrap()
    }
}
