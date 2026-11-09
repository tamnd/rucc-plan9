//! `r9 image build`: make the `g-base` image of a pinned tree (plan 05.2 and 04.2).
//!
//! The guest is the release image on a fresh overlay. The host writes the pinned tree from the
//! history clone with `git archive`, so no host has a working tree of 9front. In the guest, the
//! pinned tree replaces `/sys/src`, `/sys/include`, `/sys/lib`, `/sys/man`, `/lib`, `/rc` and
//! the include directory of each `objtype`.
//!
//! Then the guest builds the compilers, loaders and assemblers of amd64 and arm64 three times.
//! Each build installs each tool before it builds the next one, so the first build is a mix of
//! the release's tools and the new tools. The second build uses only the tools of the first
//! build, and the third build uses only the tools of the second. The second and third builds
//! must give the same bytes: that is the fixed point of kencc (04.2 and open question 2). The
//! tools of the third build are G's tools. The guest also builds `r9-cc` with them.
//!
//! The overlay is kept in the cache as the `g-base` image, and `baselines/` gets a summary
//! with its hash.

use crate::boot::{self, Request};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The tools of G, in the order that they are built. Each is a directory of `/sys/src/cmd`.
pub const TOOLS: &[&str] = &["6c", "6l", "6a", "7c", "7l", "7a"];

/// The directories of the tree that the pinned tree replaces, besides the include directory of
/// each `objtype`.
pub const TREE: &[&str] = &["sys/src", "sys/include", "sys/lib", "sys/man", "lib", "rc"];

/// The size of the collect disk.
const COLLECT_MIB: u64 = 16;

/// The work directory in the guest.
const W: &str = "/tmp/r9";

/// The number of builds of the tools.
pub const STAGES: usize = 3;

/// The include directories of the `objtype`s in a list of the directories of a tree, such as
/// `amd64/include`.
#[must_use]
pub fn include_dirs(dirs: &str) -> Vec<String> {
    dirs.lines()
        .map(str::trim)
        .filter(|d| {
            let parts: Vec<&str> = d.split('/').collect();
            parts.len() == 2 && parts[1] == "include" && parts[0] != "sys"
        })
        .map(str::to_string)
        .collect()
}

/// The commands that the guest runs. `paths` are the directories that the pinned tree
/// replaces. Each command is one line at the shell prompt.
///
/// A file of the image that is not in the tree is removed only under `sys/src`, `sys/include`
/// and the include directories, where it can change a build: a generated file, or a source that
/// 9front deleted after the release. Other files that git does not have stay, such as the
/// machine's `/lib/ndb` and the firmware in `sys/src/boot`.
#[must_use]
pub fn commands(paths: &[String]) -> Vec<String> {
    let pruned: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|p| {
            p.starts_with("sys/src") || p.starts_with("sys/include") || p.ends_with("/include")
        })
        .collect();
    let tools = TOOLS.join(" ");
    let mut c = vec![
        format!("mkdir -p /tmp/r9in /usr/glenda/r9/bin {W} && cd /tmp/r9in && tar xf $r9in"),
        // The tree goes over the old directories, and then the stale files that are not in the tree
        // are removed. A directory is not removed: / is a union that forbids creation, and the
        // console's /bin has /rc/bin bound in it.
        format!(
            "cd / && tar xf /tmp/r9in/tree.tar && tar tf /tmp/r9in/tree.tar | grep -v '/$' | sort > {W}/new && walk -f {} | grep -v '^sys/src/boot/' | sort > {W}/old && comm -23 {W}/old {W}/new > {W}/stale && rm -f /tmp/r9in/tree.tar `{{cat {W}/stale}}",
            pruned.join(" ")
        ),
        format!("wc -l {W}/new {W}/stale; sed 10q {W}/stale"),
    ];
    // A tool whose build fails is named in $s, and the last test fails the command.
    for n in 1..=STAGES {
        let d = format!("{W}/stage{n}");
        c.push(format!(
            "mkdir -p {d}; s=(); for(d in {tools}) @{{cd /sys/src/cmd/$d && mk clean > /dev/null && mk install}} >> {d}.log >[2=1] || s=($s $d); for(t in {tools}) cp /$objtype/bin/$t {d}/$t; tail -3 {d}.log; echo failed: $s; ~ $#s 0"
        ));
    }
    let logs: Vec<String> = (1..=STAGES).map(|n| format!("stage{n}.log")).collect();
    c.push(format!("sha1sum -2 256 {W}/stage*/* > {W}/tools.sha256"));
    c.push(
        "cp /tmp/r9in/r9-build /usr/glenda/r9/bin/ && cd /tmp/r9in/r9-cc && mk install && mk clean"
            .into(),
    );
    c.push(format!(
        "cd {W} && tar cf $r9collect {} tools.sha256 stale && cd / && rm -rf /tmp/r9in {W}",
        logs.join(" ")
    ));
    c
}

/// The hash of each tool after each build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tool {
    /// The name, such as `6c`.
    pub name: String,
    /// The SHA-256 after each build, first to last.
    pub sha256: Vec<String>,
}

impl Tool {
    /// Whether the last two builds gave the same bytes.
    #[must_use]
    pub fn fixed(&self) -> bool {
        match self.sha256.as_slice() {
            [.., a, b] => !a.is_empty() && a == b,
            _ => false,
        }
    }
}

/// Read the tools from the output of `sha1sum -2 256` in the guest.
#[must_use]
pub fn tools(text: &str) -> Vec<Tool> {
    let hash = |path: String| {
        text.lines()
            .find_map(|l| {
                let (h, p) = l.split_once(char::is_whitespace)?;
                (p.trim() == path).then(|| h.to_string())
            })
            .unwrap_or_default()
    };
    TOOLS
        .iter()
        .map(|t| Tool {
            name: (*t).to_string(),
            sha256: (1..=STAGES)
                .map(|n| hash(format!("{W}/stage{n}/{t}")))
                .collect(),
        })
        .collect()
}

/// The summary of a `g-base` image, written to `baselines/`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// The tree of `pins.toml`, such as `front`.
    pub tree: String,
    /// The commit of the tree.
    pub commit: String,
    /// The release whose image was the start.
    pub release: String,
    /// The `objtype`.
    pub objtype: String,
    /// The image file in the cache.
    pub image: String,
    /// The SHA-256 of the image file. The file is a qcow2 overlay on the release image.
    pub sha256: String,
    /// Whether the last two builds of every tool gave the same bytes.
    pub fixed_point: bool,
    /// Whether the first build gave the same bytes as the last, for every tool.
    pub same_as_first: bool,
    /// The seconds of the whole boot.
    pub boot_seconds: f64,
    /// The tools.
    pub tool: Vec<Tool>,
}

/// The file name of the `g-base` image of `commit` for `objtype`.
#[must_use]
pub fn image_name(commit: &str, objtype: &str) -> String {
    format!("g-base-{}.{objtype}.qcow2", &commit[..commit.len().min(12)])
}

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("--git-dir").arg(dir);
    c
}

fn output(mut c: Command) -> Result<String, String> {
    let out = c.output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

/// The directories that the pinned tree replaces at `commit`.
pub fn tree_paths(history: &Path, commit: &str) -> Result<Vec<String>, String> {
    let mut c = git(history);
    c.args(["ls-tree", "-r", "-d", "--name-only", commit]);
    let mut paths: Vec<String> = TREE.iter().map(|p| (*p).to_string()).collect();
    paths.extend(include_dirs(&output(c)?));
    Ok(paths)
}

/// Write the tar archive of `paths` at `commit` to `out`. The history clone has no file
/// contents, so the blobs are fetched first in one request.
pub fn archive(history: &Path, commit: &str, paths: &[String], out: &Path) -> Result<(), String> {
    let mut c = git(history);
    c.args(["ls-tree", "-r", "--object-only", commit, "--"])
        .args(paths);
    let blobs = output(c)?;
    let mut fetch = git(history)
        .args(["-c", "fetch.negotiationAlgorithm=noop", "fetch", "-q"])
        .args(["--no-tags", "--no-write-fetch-head", "--stdin", "origin"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if let Some(mut stdin) = fetch.stdin.take() {
        use std::io::Write as _;
        stdin
            .write_all(blobs.as_bytes())
            .map_err(|e| format!("git fetch: {e}"))?;
    }
    let status = fetch.wait().map_err(|e| format!("git fetch: {e}"))?;
    if !status.success() {
        return Err("git fetch of the blobs of the tree failed".into());
    }
    // The tree and not the commit, so that the archive has no global header with the commit.
    let mut c = git(history);
    c.args(["archive", "--format=tar", "-o"])
        .arg(out)
        .arg(format!("{commit}^{{tree}}"))
        .arg("--")
        .args(paths);
    output(c).map(|_| ())
}

/// The SHA-256 of a file, from `shasum` or `sha256sum`.
fn sha256(path: &Path) -> Result<String, String> {
    for tool in [&["shasum", "-a", "256"][..], &["sha256sum"][..]] {
        if let Ok(out) = Command::new(tool[0]).args(&tool[1..]).arg(path).output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            return Ok(text
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string());
        }
    }
    Err(format!("cannot hash {}", path.display()))
}

/// Run `r9 image build`. `req` is a boot request for the release, and `tree` and `commit` name
/// the pinned tree. Return the summary and the report to print.
pub fn build(
    repo: &crate::repo::Repo,
    release: &str,
    tree: &str,
    commit: &str,
    history: &Path,
    mut req: Request,
) -> Result<(Summary, String), String> {
    let input = req.work.join("in");
    if input.exists() {
        std::fs::remove_dir_all(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    }
    std::fs::create_dir_all(input.join("r9-cc"))
        .map_err(|e| format!("{}: {e}", input.display()))?;
    let paths = tree_paths(history, commit)?;
    archive(history, commit, &paths, &input.join("tree.tar"))?;
    let guest = repo.root.join("guest");
    for f in ["r9-build", "r9-cc/mkfile", "r9-cc/r9-cc.c"] {
        std::fs::copy(guest.join(f), input.join(f)).map_err(|e| format!("{f}: {e}"))?;
    }
    req.commands = commands(&paths);
    req.input = Some(input.clone());
    req.collect_mib = Some(COLLECT_MIB);
    req.keep = true;
    eprintln!("r9 image build: {}", boot::qemu_command(&req).join(" "));
    let outcome = boot::run(&req)?;
    let _ = std::fs::remove_dir_all(&input);
    let mut report = boot::summary(&outcome, &req.work);
    let collected = req.work.join("collected");
    let hashes = std::fs::read_to_string(collected.join("tools.sha256")).unwrap_or_default();
    let tool = tools(&hashes);
    let fixed_point = tool.iter().all(Tool::fixed);
    let same_as_first = tool.iter().all(|t| t.sha256.first() == t.sha256.last());
    if !outcome.passed {
        report.push_str("the image build failed; the overlay is kept in the work directory\n");
        return Err(report);
    }
    let images = repo.cache.join("images");
    let name = image_name(commit, &req.objtype);
    let image = images.join(&name);
    std::fs::rename(boot::overlay(&req), &image)
        .map_err(|e| format!("{}: {e}", image.display()))?;
    let summary = Summary {
        tree: tree.to_string(),
        commit: commit.to_string(),
        release: release.to_string(),
        objtype: req.objtype.clone(),
        image: name,
        sha256: sha256(&image)?,
        fixed_point,
        same_as_first,
        boot_seconds: outcome.seconds,
        tool,
    };
    let text = toml::to_string(&summary).map_err(|e| e.to_string())?;
    let dir = repo.root.join("baselines");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!(
        "g-base-{}-{}.toml",
        &commit[..commit.len().min(12)],
        req.objtype
    ));
    std::fs::write(
        &path,
        format!("# Written by r9 image build. Do not edit.\n{text}"),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    for t in &summary.tool {
        let short: Vec<&str> = t.sha256.iter().map(|h| &h[..h.len().min(12)]).collect();
        let _ = writeln!(
            report,
            "{:<3} {} {}",
            t.name,
            short.join(" "),
            if t.fixed() { "fixed" } else { "NOT FIXED" }
        );
    }
    let _ = writeln!(
        report,
        "fixed point {}; first build {} the last; image {}; summary in {}",
        if fixed_point { "holds" } else { "fails" },
        if same_as_first {
            "is the same as"
        } else {
            "differs from"
        },
        image.display(),
        path.display()
    );
    Ok((summary, report))
}

/// `r9 image` from the command line.
pub fn command(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
) -> Result<(Summary, String), String> {
    match args.target.as_deref() {
        Some("build") => {}
        Some(other) => {
            return Err(format!(
                "r9 image {other}: only build is here; sync comes later"
            ));
        }
        None => return Err("give the action, as in r9 image build".into()),
    }
    let pins = r9_pins::pins::PinsFile::load(&repo.root)?
        .ok_or("there is no pins.toml; run r9 sets --write")?;
    let (tree_name, tree) = match args.get("tree") {
        Some(t) => pins
            .tree
            .get_key_value(t)
            .ok_or_else(|| format!("pins.toml has no tree {t}"))?,
        None => pins.tree.iter().next().ok_or("pins.toml has no tree")?,
    };
    let mut boot_args = args.clone();
    boot_args.target = args.get("release").map(str::to_string);
    let mut req = boot::request_from(repo, &boot_args)?;
    let release = req
        .base
        .file_name()
        .and_then(|f| f.to_str())
        .and_then(|f| f.strip_prefix("9front-"))
        .and_then(|f| f.split('.').next())
        .unwrap_or_default()
        .to_string();
    if args.get("out").is_none() {
        req.work = repo.root.join("work").join("image").join(format!(
            "{}-{}",
            &tree.commit[..tree.commit.len().min(12)],
            req.objtype
        ));
    }
    if args.get("timeout").is_none() {
        req.timeout = std::time::Duration::from_secs(3600);
    }
    let history = crate::locate::history(&repo.cache, &tree.repo)?;
    build(repo, &release, tree_name, &tree.commit, &history, req)
}

/// The path of the `g-base` image of the first tree for `objtype`, when it is in the cache.
#[must_use]
pub fn g_base(repo: &crate::repo::Repo, objtype: &str) -> Option<PathBuf> {
    let pins = r9_pins::pins::PinsFile::load(&repo.root).ok()??;
    let tree = pins.tree.values().next()?;
    let path = repo
        .cache
        .join("images")
        .join(image_name(&tree.commit, objtype));
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_include_directories_are_found() {
        let dirs =
            "386\n386/include\namd64/include\nsys/include\nsys/src/9/include\narm64/include\n";
        assert_eq!(
            include_dirs(dirs),
            vec!["386/include", "amd64/include", "arm64/include"]
        );
    }

    #[test]
    fn the_commands_replace_the_tree_and_build_twice() {
        let paths = vec![
            "sys/src".to_string(),
            "rc".to_string(),
            "lib".to_string(),
            "amd64/include".to_string(),
        ];
        let c = commands(&paths);
        assert!(c[1].starts_with("cd / && tar xf /tmp/r9in/tree.tar && tar tf /tmp/r9in/tree.tar"));
        assert!(c[1].contains(
            "walk -f sys/src amd64/include | grep -v '^sys/src/boot/' | sort > /tmp/r9/old"
        ));
        assert!(c[1].ends_with("rm -f /tmp/r9in/tree.tar `{cat /tmp/r9/stale}"));
        assert!(c[3].starts_with("mkdir -p /tmp/r9/stage1; s=(); for(d in 6c 6l 6a 7c 7l 7a) @{cd /sys/src/cmd/$d && mk clean > /dev/null && mk install} >> /tmp/r9/stage1.log >[2=1] || s=($s $d)"));
        assert!(c[5].contains("cp /$objtype/bin/$t /tmp/r9/stage3/$t"));
        assert!(c[5].ends_with("~ $#s 0"));
        assert!(c.last().unwrap().contains("tar cf $r9collect"));
        assert!(c.iter().all(|l| !l.contains('\n')));
    }

    #[test]
    fn the_tools_are_read_from_the_hashes() {
        let text = "zz\t/tmp/r9/stage1/6c\naa\t/tmp/r9/stage2/6c\naa\t/tmp/r9/stage3/6c\nbb /tmp/r9/stage2/7c\ncc /tmp/r9/stage3/7c\n";
        let t = tools(text);
        assert_eq!(t[0].name, "6c");
        assert_eq!(t[0].sha256, vec!["zz", "aa", "aa"]);
        assert!(t[0].fixed());
        assert_eq!(t[3].name, "7c");
        assert!(!t[3].fixed());
        assert!(!t[1].fixed());
    }

    #[test]
    fn the_image_name_has_a_short_commit() {
        assert_eq!(
            image_name("2859ae04e800737307470538a942ab8f8261253b", "arm64"),
            "g-base-2859ae04e800.arm64.qcow2"
        );
    }
}
