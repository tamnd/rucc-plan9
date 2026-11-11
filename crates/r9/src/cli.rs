//! The command line, parsed by hand as `rb` in rucc-bsd does it.
//!
//! A command takes at most one positional word, such as a pin name or a file, and then options. Every option
//! takes a value, given as the next word or after `=`, except the switches. A value may start with
//! a dash. Each command lists the options it accepts, so that a misspelt option fails before
//! anything runs.

use std::collections::BTreeMap;

/// A parsed command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
#[allow(
    clippy::too_many_lines,
    reason = "one table with a line for each option"
)]
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
            switches: &["keep", "net"],
        },
        "baseline" => Accepts {
            positional: true,
            values: &[
                "row", "runs", "suites", "pin", "image", "accel", "timeout", "firmware", "out",
            ],
            switches: &[],
        },
        "build" | "mix" => Accepts {
            positional: true,
            values: &[
                "side",
                "row",
                "accel",
                "timeout",
                "firmware",
                "dir",
                "out",
                "image",
                "rucc",
                "default",
                "file",
                "mix",
                "test",
                "test-timeout",
                "targets",
            ],
            switches: &["keep", "bisect"],
        },
        "fixedpoint" => Accepts {
            positional: false,
            values: &[
                "row", "r", "g", "accel", "timeout", "firmware", "dir", "out", "top",
            ],
            switches: &[],
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
        "ccd" => Accepts {
            positional: false,
            values: &["port", "rucc", "work", "default", "file", "mix"],
            switches: &[],
        },
        "locate" => Accepts {
            positional: true,
            values: &["tree", "days"],
            switches: &["write"],
        },
        "flags-diff" | "tree-diff" => Accepts {
            positional: true,
            values: &["with", "top"],
            switches: &[],
        },
        "rtt" => Accepts {
            positional: false,
            values: &["port", "count", "file", "wait"],
            switches: &[],
        },
        "objdiff" => Accepts {
            positional: true,
            values: &["with", "context"],
            switches: &[],
        },
        "optab" => Accepts {
            positional: false,
            values: &["root", "pin", "out"],
            switches: &[],
        },
        "sdiff" => Accepts {
            positional: true,
            values: &["with", "function", "width"],
            switches: &["all"],
        },
        "helpers" => Accepts {
            positional: true,
            values: &["src", "with"],
            switches: &[],
        },
        "objcheck" => Accepts {
            positional: true,
            values: &[],
            switches: &[],
        },
        "demands" => Accepts {
            positional: true,
            values: &["root", "out", "top", "flags"],
            switches: &["kencc"],
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
      2026-11-09T06:00:00Z. For the tree, --as-of uses the commit date that GitHub keeps, not
      the time of the push. Thus a commit that 9front pushed after TIME with an older date can
      be taken.
  r9 check
      Check sets.toml, pins.toml, rows.toml and machines.toml with no network. A hand edit of
      pins.toml fails here.
  r9 fetch [NAME] [--row OBJTYPE]
      Fetch a pin into the cache and check its hash: a tree by commit, or the images of a release.
      --row fetches the image of one row only. With no name, fetch the pins of the current set.
  r9 boot [NAME] [--row OBJTYPE] [--accel kvm|hvf|tcg] [--run FILE] [--timeout SECS]
          [--bootargs ARGS] [--firmware PATH] [--in PATH] [--collect MIB] [--image FILE]
          [--disk FILE] [--out DIR] [--keep] [--net]
      Boot a release image under QEMU on a fresh overlay. Answer the boot prompts, type each
      command of FILE (smoke/boot.rc by default) at the shell prompt, and halt the guest. The
      console goes to DIR/console.log and the results to DIR/steps.json. With no name, boot the
      newest release of the current set. The row is amd64 by default. --keep keeps the overlay.
      --in attaches a directory or a tar file as a raw tar disk after the root disk. --collect
      attaches a blank disk of MIB MiB after it. In the guest, $r9in names the input disk and
      $r9collect the collect disk. A tar archive that the guest writes to $r9collect is
      unpacked into DIR/collected. --image boots another qcow2 file, such as a kept overlay,
      in place of the release image, with a fresh overlay on top. --disk attaches a qcow2 file
      after the root disk. The writes of the guest to it are not kept. --net gives the guest a
      network card on the user network of QEMU, where the host is 10.0.2.2.
  r9 build [NAME] --side G|R [--row OBJTYPE] [--accel kvm|hvf|tcg] [--dir DIR] [--timeout SECS]
          [--firmware PATH] [--image FILE] [--out DIR] [--rucc PATH] [--keep] [--test CMD]
          [--test-timeout SECS] [--targets 'T ...']
      Boot the g-base image of the row, or the image of the release NAME when there is no
      g-base image, and run mk install in DIR (/sys/src by default) with r9-cc in the compiler
      slot. --image boots FILE. Collect the compile records, the mk log, the files that the
      build wrote with their SHA-256, and the generated C sources and headers into
      DIR/collected. Write the summary to build.toml and print the census. Side G runs kencc.
      Side R starts r9 ccd on a free port and sends each call to rucc (PATH, or R9_RUCC, or rucc
      on the path). R9_BRINGUP gives the bring-up words (cc:auto by default), so that a call
      that rucc fails goes to kencc. The line of each call is in ccd.log. --test runs the rc
      command CMD in the guest after the build, in a new note group. After SECS seconds (600
      by default) the guest kills the test, and the status is timeout. The status and the
      output go to test.status and test.log in DIR/collected. The command fails when the
      build or the test fails. --targets gives the mk targets in place of install, such as
      'clean install'. Each target is a separate mk, in the order given.
  r9 mix [NAME] [--file UNIT=R|G,...] [--mix FILE] [--default R|G] [r9 build options]
      Build with some units from rucc and the others from kencc (plan 05.7, 12.8). A unit is a
      source file of the guest, such as /sys/src/libc/port/strcmp.c, or a directory, which
      applies to each file under it. The longest unit that matches a file gives its side, and
      the other files get the default side (R by default). FILE has one UNIT=R or UNIT=G on each
      line. This is r9 build --side R, but r9 ccd gives the status mix: G to a call that has a
      source on side G, and r9-cc then runs kencc. ccd.log marks such a call with G. The mix
      goes to mix.txt in the output directory. r9 build and r9 ccd take the same options.
  r9 mix --bisect --test CMD [r9 build options]
      Find the unit whose rucc object makes the test fail (plan 12.8). Build DIR with kencc,
      and the test must pass. Build it with rucc, and the test must fail. The units are the
      files that rucc compiled. Then build half of the units that are left with rucc and the
      others with kencc, until one unit is left. When the test fails only with units from both
      halves, print the units that are left. Each build goes to stepN in the output directory.
      The exit status is 1 only when the test fails with kencc alone.
  r9 baseline [NAME] [--row OBJTYPE,...] [--runs N] [--suites boot,smoke,test] [--pin PIN]
          [--image FILE] [--accel kvm|hvf|tcg] [--timeout SECS] [--firmware PATH] [--out DIR]
      Run each suite N times (3 by default) on the g-base image of each row (amd64,arm64 by
      default), and keep what G gives. A test is graded when each run gives the same result, and
      flaky when the runs differ. Write DIR/PIN/OBJTYPE/SUITE.json (baselines/ by default) with
      the hash of each run, the graded results and the flaky ones, and print the flaky ones. The
      pin comes from the image name. --image boots FILE in place of the g-base image. The boots
      go to work/baseline.
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
  r9 demands FILE --root DIR [--flags FLAGS] [--out FILE] [--top N] [--kencc]
      Compile each unit of a G build again with rucc -fsyntax-only on the host, and rank the
      first errors by the number of units they stop. FILE is the compile.jsonl of the build. DIR
      holds the guest's sys/src, sys/include and OBJTYPE/include. The ranks go to --out
      (demands.toml by default), and the first N (20 by default) are printed. rucc comes from
      R9_RUCC, or from the path. --flags gives more rucc flags, separated by spaces. --kencc
      compiles each unit with the kencc driver of rucc and the Plan 9 target, as r9 ccd does,
      with DIR as the sysroot.
  r9 flags-diff A --with B [--top N]
      Compare the arguments that two builds gave the compiler slot (plan 05.5). A and B are the
      compile.jsonl files of the builds. A unit is the compiler name, the directory and the
      sources. Print the units with other arguments and the units that only one build has. At
      most N units (20 by default) are printed in each group. Exit with failure when the builds
      differ.
  r9 fixedpoint --r FILE [--g DIR] [--row OBJTYPE] [--accel kvm|hvf|tcg] [--timeout SECS]
          [--dir DIR] [--out DIR] [--top N]
      Build the tree again with the kencc that rucc built, and compare it with G (plan 12.4).
      FILE is the image that r9 build --side R --keep left, with the 6c, 6l and 6a of rucc.
      Run r9 build --side G --targets 'clean install' in the g-base image, then in FILE. The
      second build is Rk. Compare the files that the two builds wrote as r9 tree-diff does,
      and write the report to fixedpoint.txt. --g DIR uses an earlier G build with the same
      targets. The builds go to g and rk in --out (work/fixedpoint by default). Exit with
      failure when a build fails or when Rk differs from G.
  r9 tree-diff A --with B [--top N]
      Compare the files that two builds wrote. A and B are the directories of two builds, or
      their changed.sha256 files. Print the files that only one build wrote, the files with
      different contents, and the archives whose members are the same but whose dates differ.
      The raced group has the archives that differ only by a member that two ar commands put
      in them, and the APE programs that link such an archive. At most N names (20 by default)
      are printed in each group. Exit with failure when the builds differ.
  r9 objdiff A [--with B] [--context N]
      Compare two objects of 6c or 7c, or two archives of them, record by record (plan 07.6).
      The objects are decoded as 6l and 7l read them, and each symbol is printed as 6c -S and
      7c -S print it. Line numbers and the order of the symbol slots are not compared. Print
      the symbols that only one file has, and the lines that differ in each function, with N
      same lines (3 by default) around each change. Exit with failure when the records
      differ. With no --with, print the listing of A.
  r9 sdiff A --with B [--function NAME] [--width N] [--all]
      Show the code of two objects of 6c or 7c, or of two archives of them, side by side,
      function by function (plan 12.8). A is on the left and B is on the right. The lines are
      aligned as in r9 objdiff. The middle column is | for a changed line, < for a line that
      only A has and > for a line that only B has. Show only the functions that differ, or all
      of them with --all, or only NAME. Each side is N characters wide (60 by default). Exit
      with failure when a function differs.
  r9 helpers PATH --src DIR
  r9 helpers R --with G
      List the external symbols that the objects at PATH refer to and that no source under DIR
      names (plan 08.6). PATH is a .6 or .7 file, an archive, or a directory of them. A source is
      a file that ends in .c, .h, .s, .y, .l or .mp. The list is the set of calls that the
      compiler makes by itself, and rucc calls only these. With --with, R has the objects of
      rucc and G has the objects of kencc, in the same layout. Compare each R object with the
      G object of the same name, and list the symbols that R refers to and G does not. Exit
      with failure when there is one.
  r9 optab --root DIR [--pin PIN] [--out FILE]
      Read the instruction tables of 6l under DIR/sys/src/cmd/6l (plan 07.3): the Y classes of
      l.h, optab and its ytab arrays in optab.c, ymovtab in span.c, and ycover in obj.c. Write them as Rust to
      FILE, or print them. PIN goes in the header. The file crates/r9/src/optab6.rs is the
      output for the pin, and r9 objcheck uses it.
  r9 objcheck FILE
      Check a .6 file, or each .6 member of an archive, against the instruction tables of 6l
      (plan 07.6). Each instruction gets the classes of its operands as oclass in 6l/span.c
      gives them, after the changes that 6l makes when it reads an object. An instruction that
      no pair of its ytab or of ymovtab covers is the link error doasm: notfound. Print each
      such instruction, and exit with failure when there is one.
  r9 ccd [--port N] [--rucc PATH] [--work DIR] [--file UNIT=R|G,...] [--mix FILE] [--default R|G]
      Serve the compiler slot of a guest (plan 05.4). r9-cc in the guest sends each call of 6c
      and 7c to port N on the host (5640 by default), as tcp!10.0.2.2!N, with the name space of
      the build on a second connection. ccd runs rucc (PATH, or R9_RUCC, or rucc on the path)
      with the sysroot on that name space and sends the objects back. The outputs of each call
      go in a new directory under DIR (a temporary directory by default) while rucc runs. Each
      call prints one line. Boot the guest with --net. The mix options are as in r9 mix.
  r9 rtt [--port N] [--count N] [--file PATH] [--wait SECS]
      Time 9P round trips from a guest to a host process (open question 10). Listen on the port
      (5641 by default) and wait up to SECS (1800 by default) for r9rtt in the guest, which
      smoke/rtt.rc builds and runs. Then run COUNT (10000 by default) sequences of walk, open,
      read and clunk of PATH (/sys/include/libc.h by default), and print the times.
  r9 locate NAME --tree DIR [--days N] [--write]
      Find the commit that the release NAME was cut from. DIR holds the release's sys/src and
      sys/include, as a boot with --collect gives them. DIR can also be the collected.tar of
      that boot, which keeps the paths that differ only by case on a host that ignores case.
      Each file is hashed as git hashes it, and compared with each commit of the pinned branch
      in the N days (90 by default) up to the day after the release date. Print the commit with the fewest different files, and the files
      that differ. The history is a blobless clone in the cache. --write writes the commit to
      pins.toml when no file has different contents and no other commit ties. r9 sets --write
      keeps it.

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
