//! The command line, parsed by hand as `rb` in rucc-bsd does it.
//!
//! A command takes at most one positional word, such as a pin name or a file, and then options. Every option
//! takes a value, given as the next word or after `=`, except the switches. A value may start with
//! a dash. Each command lists the options it accepts, so that a misspelt option fails before
//! anything runs.

use std::collections::BTreeMap;

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The subcommand.
    pub command: String,
    /// The positional word, which is a pin name where a command takes one.
    pub target: Option<String>,
    /// Options with values.
    pub values: BTreeMap<String, String>,
    /// Switches that were given.
    pub switches: Vec<String>,
}

/// What one command accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepts {
    /// Whether the command takes a positional word.
    pub positional: bool,
    /// The options that take a value.
    pub values: &'static [&'static str],
    /// The options that take no value.
    pub switches: &'static [&'static str],
}

impl Accepts {
    /// A command that takes nothing.
    pub const NOTHING: Self = Self {
        positional: false,
        values: &[],
        switches: &[],
    };
}

/// What each command of `r9` accepts, or `None` for a command `r9` does not have.
#[must_use]
pub fn accepted(command: &str) -> Option<Accepts> {
    Some(match command {
        "help" | "version" | "check" => Accepts::NOTHING,
        "sets" => Accepts {
            positional: false,
            values: &["as-of"],
            switches: &["write"],
        },
        "boot" => Accepts {
            positional: true,
            values: &[
                "row", "accel", "timeout", "run", "bootargs", "out", "firmware", "in", "collect",
                "image", "disk",
            ],
            switches: &["keep"],
        },
        "build" => Accepts {
            positional: true,
            values: &[
                "side", "row", "accel", "timeout", "firmware", "dir", "out", "image",
            ],
            switches: &["keep"],
        },
        "image" => Accepts {
            positional: true,
            values: &[
                "release", "tree", "row", "accel", "timeout", "firmware", "out",
            ],
            switches: &[],
        },
        "fetch" => Accepts {
            positional: true,
            values: &["row"],
            switches: &[],
        },
        "census" => Accepts {
            positional: true,
            values: &["side"],
            switches: &[],
        },
        "locate" => Accepts {
            positional: true,
            values: &["tree", "days"],
            switches: &[],
        },
        "demands" => Accepts {
            positional: true,
            values: &["root", "out", "top", "flags"],
            switches: &[],
        },
        _ => return None,
    })
}

/// The usage text.
pub const USAGE: &str = "\
r9: build pinned 9front trees with rucc and with kencc, boot them and test them

usage:
  r9 help
      Print this text.
  r9 version
      Print the version of r9.
  r9 sets [--write] [--as-of TIME]
      Read what 9front publishes and expand sets.toml into pins. Print the pins, and say whether
      pins.toml has them. --write writes pins.toml. --as-of takes the pins at a UTC time, such as
      2026-11-09T06:00:00Z.
  r9 check
      Check sets.toml, pins.toml, rows.toml and machines.toml with no network. A hand edit of
      pins.toml fails here.
  r9 fetch [NAME] [--row OBJTYPE]
      Fetch a pin into the cache and check its hash: a tree by commit, or the images of a release.
      --row fetches the image of one row only. With no name, fetch the pins of the current set.
  r9 boot [NAME] [--row OBJTYPE] [--accel kvm|hvf|tcg] [--run FILE] [--timeout SECS]
          [--bootargs ARGS] [--firmware PATH] [--in PATH] [--collect MIB] [--image FILE]
          [--disk FILE] [--out DIR] [--keep]
      Boot a release image under QEMU on a fresh overlay. Answer the boot prompts, type each
      command of FILE (smoke/boot.rc by default) at the shell prompt, and halt the guest. The
      console goes to DIR/console.log and the results to DIR/steps.json. With no name, boot the
      newest release of the current set. The row is amd64 by default. --keep keeps the overlay.
      --in attaches a directory or a tar file as a raw tar disk after the root disk. --collect
      attaches a blank disk of MIB MiB after it. In the guest, $r9in names the input disk and
      $r9collect the collect disk. A tar archive that the guest writes to $r9collect is
      unpacked into DIR/collected. --image boots another qcow2 file, such as a kept overlay,
      in place of the release image, with a fresh overlay on top. --disk attaches a qcow2 file
      after the root disk. The writes of the guest to it are not kept.
  r9 build [NAME] --side G [--row OBJTYPE] [--accel kvm|hvf|tcg] [--dir DIR] [--timeout SECS]
          [--firmware PATH] [--image FILE] [--out DIR] [--keep]
      Boot the g-base image of the row, or the image of the release NAME when there is no
      g-base image, and run mk install in DIR (/sys/src by default) with r9-cc in the
      compiler slot. --image boots FILE. Collect the compile records, the mk log, the files that the build wrote
      with their SHA-256, and the generated C sources and headers into DIR/collected. Write the
      summary to build.toml and print the census. Side R comes at P1.
  r9 image build [--release NAME] [--tree NAME] [--row OBJTYPE] [--accel kvm|hvf|tcg]
          [--timeout SECS] [--firmware PATH] [--out DIR]
      Make the g-base image of a pinned tree (the first tree of pins.toml by default). Boot the
      release image, put the pinned tree in place of /sys/src, /sys/include, /sys/lib, /sys/man,
      /lib, /rc and each OBJTYPE/include, and build 6c, 6l, 6a, 7c, 7l and 7a three times. The
      last two builds must give the same bytes. Then build r9-cc, keep the overlay in the cache
      as the g-base image, and write its summary to baselines/.
  r9 census FILE [--side R|G]
      Count what reached the compiler slot in one build, from the compile.jsonl that r9-cc
      wrote: the calls by origin and by compiler, the calls that failed, the calls from pcc,
      the time by directory, and the sources compiled more than once. The census of an R build
      fails when a call went to kencc. The side comes from the records when --side is not given.
  r9 demands FILE --root DIR [--flags FLAGS] [--out FILE] [--top N]
      Compile each unit of a G build again with rucc -fsyntax-only on the host, and rank the
      first errors by the number of units they stop. FILE is the compile.jsonl of the build. DIR
      holds the guest's sys/src, sys/include and OBJTYPE/include. The ranks go to --out
      (demands.toml by default), and the first N (20 by default) are printed. rucc comes from
      R9_RUCC, or from the path. --flags gives more rucc flags, separated by spaces.
  r9 locate NAME --tree DIR [--days N]
      Find the commit that the release NAME was cut from. DIR holds the release's sys/src and
      sys/include, as a boot with --collect gives them. Each file is hashed as git hashes it, and
      compared with each commit of the pinned branch in the N days (90 by default) up to the day
      after the release date. Print the commit with the fewest different files, and the files
      that differ. The history is a blobless clone in the cache.

The repository is found by walking up to sets.toml, or from R9_ROOT. Downloads, images and
fetched trees go to R9_CACHE, or ~/.cache/r9.
";

impl Args {
    /// Parse the words after the program name, with the commands `table` knows.
    pub fn parse(words: &[String], table: fn(&str) -> Option<Accepts>) -> Result<Self, String> {
        let mut words = words.iter();
        let command = match words.next() {
            None => "help".to_string(),
            Some(w) if w == "-h" || w == "--help" => "help".to_string(),
            Some(w) if w == "--version" => "version".to_string(),
            Some(w) => w.clone(),
        };
        let accepts =
            table(&command).ok_or_else(|| format!("unknown command {command}; see r9 help"))?;
        let mut target = None;
        let mut values = BTreeMap::new();
        let mut switches = Vec::new();
        while let Some(word) = words.next() {
            let Some(option) = word.strip_prefix("--") else {
                if accepts.positional && target.is_none() {
                    target = Some(word.clone());
                    continue;
                }
                return Err(format!("unexpected argument {word}"));
            };
            let (name, inline) = match option.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (option, None),
            };
            if name == "help" {
                switches.push("help".to_string());
                continue;
            }
            if accepts.switches.contains(&name) {
                if inline.is_some() {
                    return Err(format!("--{name} takes no value"));
                }
                switches.push(name.to_string());
                continue;
            }
            if !accepts.values.contains(&name) {
                return Err(format!("r9 {command} has no option --{name}"));
            }
            let value = match inline {
                Some(v) => v,
                None => words
                    .next()
                    .cloned()
                    .ok_or_else(|| format!("--{name} needs a value"))?,
            };
            values.insert(name.to_string(), value);
        }
        Ok(Self {
            command,
            target,
            values,
            switches,
        })
    }

    /// The value of an option, if given.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// Whether a switch was given.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(command: &str) -> Option<Accepts> {
        match command {
            "fetch" => Some(Accepts {
                positional: true,
                values: &["set"],
                switches: &["all"],
            }),
            "sets" => Some(Accepts {
                positional: false,
                values: &["as-of"],
                switches: &["write"],
            }),
            _ => accepted(command),
        }
    }

    fn parse(words: &[&str]) -> Result<Args, String> {
        let words: Vec<String> = words.iter().map(|s| (*s).to_string()).collect();
        Args::parse(&words, table)
    }

    #[test]
    fn a_pin_then_options() {
        let args = parse(&["fetch", "front", "--all"]).unwrap();
        assert_eq!(args.target.as_deref(), Some("front"));
        assert!(args.has("all"));
        assert!(!args.has("write"));
    }

    #[test]
    fn values_come_after_a_space_or_an_equals_sign() {
        let args = parse(&["fetch", "--set", "current"]).unwrap();
        assert_eq!(args.get("set"), Some("current"));
        let args = parse(&["sets", "--as-of=2026-11-08T06:00:00Z", "--write"]).unwrap();
        assert_eq!(args.get("as-of"), Some("2026-11-08T06:00:00Z"));
        assert!(args.has("write"));
    }

    #[test]
    fn a_value_may_start_with_a_dash() {
        let args = parse(&["fetch", "--set", "-x"]).unwrap();
        assert_eq!(args.get("set"), Some("-x"));
    }

    #[test]
    fn mistakes_are_named() {
        let unknown = parse(&["frobnicate"]).unwrap_err();
        assert!(unknown.contains("unknown command"), "{unknown}");
        let extra = parse(&["sets", "front"]).unwrap_err();
        assert!(extra.contains("unexpected"), "{extra}");
        let second = parse(&["fetch", "a", "b"]).unwrap_err();
        assert!(second.contains("unexpected argument b"), "{second}");
        let option = parse(&["fetch", "--row", "amd64"]).unwrap_err();
        assert!(option.contains("--row"), "{option}");
        let missing = parse(&["fetch", "--set"]).unwrap_err();
        assert!(missing.contains("needs a value"), "{missing}");
        let switch = parse(&["sets", "--write=yes"]).unwrap_err();
        assert!(switch.contains("takes no value"), "{switch}");
    }

    #[test]
    fn nothing_at_all_is_help_and_the_dash_forms_work() {
        assert_eq!(parse(&[]).unwrap().command, "help");
        assert_eq!(parse(&["-h"]).unwrap().command, "help");
        assert_eq!(parse(&["--version"]).unwrap().command, "version");
        assert!(parse(&["version", "--help"]).unwrap().has("help"));
    }

    #[test]
    fn every_command_in_the_usage_is_accepted() {
        for line in USAGE.lines() {
            let Some(rest) = line.trim_start().strip_prefix("r9 ") else {
                continue;
            };
            let command = rest.split_whitespace().next().unwrap_or_default();
            assert!(accepted(command).is_some(), "r9 {command} is in the usage");
        }
    }
}
