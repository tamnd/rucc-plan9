//! `r9 fetch`: put a pinned image or tree in the cache and prove it is the pinned one.
//!
//! An image is downloaded with curl, or with aria2c from its torrent when 9front no longer serves
//! it over HTTP. Its SHA-256 must equal the one the release page gives. Then it is unpacked, and
//! the `.gz` file is removed to save disk space. A mark file next to the image records the hash
//! that was checked. A tree is fetched with git, one commit deep, and `git rev-parse HEAD` must
//! give the pinned commit. Work is done under a `.part` name and renamed when every check passed,
//! so a file in the cache is a whole one.

use crate::pins::{Image, Tree, hex};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A fetched image or tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The unpacked image, or the directory of the tree.
    pub path: PathBuf,
    /// Whether the cache had it already.
    pub cached: bool,
}

/// The path of an unpacked image in the cache.
#[must_use]
pub fn image_path(image: &Image, cache: &Path) -> PathBuf {
    let name = image.file.strip_suffix(".gz").unwrap_or(&image.file);
    cache.join("images").join(name)
}

/// The path of a fetched tree in the cache.
#[must_use]
pub fn tree_path(tree: &Tree, cache: &Path) -> PathBuf {
    cache.join("trees").join(&tree.commit)
}

fn mark_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".sha256");
    PathBuf::from(name)
}

/// Fetch and unpack `image`, unless the cache has it.
pub fn image(image: &Image, cache: &Path) -> Result<Fetched, String> {
    let path = image_path(image, cache);
    let mark = mark_path(&path);
    if path.is_file() && std::fs::read_to_string(&mark).is_ok_and(|m| m.trim() == image.sha256) {
        return Ok(Fetched { path, cached: true });
    }
    let dir = path.parent().unwrap_or(cache);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let gz = dir.join(format!("{}.part", image.file));
    let _ = std::fs::remove_file(&gz);
    download(image, &gz)?;
    let actual = sha256(&gz)?;
    if actual != image.sha256 {
        let _ = std::fs::remove_file(&gz);
        return Err(format!(
            "{}: the SHA-256 is {actual}, but the release page gives {}",
            image.file, image.sha256
        ));
    }
    let part = dir.join(format!(
        "{}.part",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let out = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let status = Command::new("gzip")
        .arg("-dc")
        .arg(&gz)
        .stdout(out)
        .status()
        .map_err(|e| format!("cannot run gzip: {e}"))?;
    let _ = std::fs::remove_file(&gz);
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("gzip -dc {} failed", image.file));
    }
    std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&mark, format!("{}\n", image.sha256))
        .map_err(|e| format!("{}: {e}", mark.display()))?;
    Ok(Fetched {
        path,
        cached: false,
    })
}

/// Download the `.gz` of `image` to `to`.
fn download(image: &Image, to: &Path) -> Result<(), String> {
    if image.url.is_empty() && image.torrent.is_empty() {
        return Err(format!("{} has neither a URL nor a torrent", image.file));
    }
    if !image.url.is_empty() {
        return run(Command::new("curl")
            .args(["-fsSL", "--retry", "5", "--retry-all-errors", "-o"])
            .arg(to)
            .arg(&image.url));
    }
    let dir = to.parent().unwrap_or(Path::new("."));
    let seed = dir.join(format!("{}.aria2", image.file));
    run(Command::new("aria2c")
        .args([
            "--seed-time=0",
            "--follow-torrent=mem",
            "--summary-interval=0",
        ])
        .arg(format!("--dir={}", dir.display()))
        .arg(&image.torrent))
    .map_err(|e| {
        format!(
            "{} is only on a torrent, and aria2c failed or is missing: {e}",
            image.file
        )
    })?;
    let _ = std::fs::remove_file(seed);
    let got = dir.join(&image.file);
    std::fs::rename(&got, to).map_err(|e| format!("{}: {e}", got.display()))
}

/// Fetch `tree` at its commit with git, unless the cache has it.
pub fn tree(tree: &Tree, cache: &Path) -> Result<Fetched, String> {
    let path = tree_path(tree, cache);
    if path.join(".git").is_dir() && head(&path).is_ok_and(|h| h == tree.commit) {
        return Ok(Fetched { path, cached: true });
    }
    let part = path.with_extension("part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let url = format!("https://github.com/{}.git", tree.repo);
    run(Command::new("git")
        .arg("-C")
        .arg(&part)
        .args(["init", "-q"]))?;
    run(Command::new("git").arg("-C").arg(&part).args([
        "fetch",
        "-q",
        "--depth",
        "1",
        &url,
        &tree.commit,
    ]))?;
    run(Command::new("git")
        .arg("-C")
        .arg(&part)
        .args(["checkout", "-q", "FETCH_HEAD"]))?;
    let got = head(&part)?;
    if got != tree.commit {
        return Err(format!(
            "git gave {got} for the pin {}; the remote does not have it",
            tree.commit
        ));
    }
    let _ = std::fs::remove_dir_all(&path);
    std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Fetched {
        path,
        cached: false,
    })
}

fn head(dir: &Path) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!("{} is not a git tree", dir.display()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run(command: &mut Command) -> Result<(), String> {
    let out = command
        .output()
        .map_err(|e| format!("cannot run {}: {e}", command.get_program().display()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed: {}",
            command.get_program().display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The SHA-256 of a file, in lower case hex.
pub fn sha256(file: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("{}: {e}", file.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("r9-fetch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_hash_of_a_file_is_known() {
        let dir = scratch("hash");
        let f = dir.join("abc");
        std::fs::write(&f, "abc").unwrap();
        assert_eq!(
            sha256(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_marked_image_is_not_fetched_again() {
        let cache = scratch("cache");
        let image = Image {
            file: "9front-1.amd64.qcow2.gz".into(),
            objtype: "amd64".into(),
            kind: "qcow2".into(),
            sha256: "a".repeat(64),
            url: String::new(),
            torrent: String::new(),
        };
        let path = image_path(&image, &cache);
        assert!(path.ends_with("images/9front-1.amd64.qcow2"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "qcow").unwrap();
        std::fs::write(mark_path(&path), "a".repeat(64)).unwrap();
        assert!(super::image(&image, &cache).unwrap().cached);
        std::fs::write(mark_path(&path), "b".repeat(64)).unwrap();
        let err = super::image(&image, &cache).unwrap_err();
        assert!(err.contains("neither a URL nor a torrent"), "{err}");
        let _ = std::fs::remove_dir_all(cache);
    }
}
