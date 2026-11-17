//! The `r9` command line. Every command is listed in [`cli::accepted`] and [`cli::USAGE`], and the
//! library does its work.

use r9::cli::{self, Args};
use r9::repo::Repo;
use r9_pins::web::Curl;
use std::process::ExitCode;

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let result = Args::parse(&words, cli::accepted).and_then(|args| run(&args));
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("r9: {message}");
            ExitCode::from(2)
        }
    }
}

/// Run one parsed command.
fn run(args: &Args) -> Result<ExitCode, String> {
    if args.command == "help" || args.has("help") {
        print!("{}", cli::USAGE);
        return Ok(ExitCode::SUCCESS);
    }
    match args.command.as_str() {
        "version" => {
            println!("r9 {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        "sets" => {
            let repo = Repo::find()?;
            let outcome = r9::sets::run(
                &repo,
                &Curl::from_env(),
                args.get("as-of"),
                args.has("write"),
            )?;
            print!("{}", outcome.report);
            Ok(ExitCode::SUCCESS)
        }
        "check" => {
            let repo = Repo::find()?;
            match r9::check::run(&repo) {
                Ok(report) => {
                    print!("{report}");
                    Ok(ExitCode::SUCCESS)
                }
                Err(problems) => {
                    eprintln!("r9 check: {problems}");
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        "boot" => {
            let repo = Repo::find()?;
            let req = r9::boot::request_from(&repo, args)?;
            eprintln!("r9 boot: {}", r9::boot::qemu_command(&req).join(" "));
            let outcome = r9::boot::run(&req)?;
            print!("{}", r9::boot::summary(&outcome, &req.work));
            Ok(if outcome.passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "baseline" => {
            let repo = Repo::find()?;
            let (ok, report) = r9::baseline::command(&repo, args)?;
            print!("{report}");
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "build" | "mix" | "diffrun" | "fixedpoint" | "2x2" => {
            let repo = Repo::find()?;
            let (passed, report) = match args.command.as_str() {
                "diffrun" => r9::diffrun::command(&repo, args)?,
                "fixedpoint" => r9::fixedpoint::command(&repo, args)?,
                "2x2" => r9::twobytwo::command(&repo, args)?,
                _ => r9::build::command(&repo, args)?,
            };
            print!("{report}");
            Ok(if passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "image" => {
            let repo = Repo::find()?;
            let (summary, report) = r9::image::command(&repo, args)?;
            print!("{report}");
            Ok(if summary.fixed_point {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "fetch" => {
            let repo = Repo::find()?;
            print!(
                "{}",
                r9::fetch::run(&repo, args.target.as_deref(), args.get("row"))?
            );
            Ok(ExitCode::SUCCESS)
        }
        "rtt" => {
            print!("{}", r9::rtt::command(args)?);
            Ok(ExitCode::SUCCESS)
        }
        "ccd" => {
            r9::ccd::command(args)?;
            Ok(ExitCode::SUCCESS)
        }
        "locate" => locate(args),
        _ => run_records(args),
    }
}

/// Run `r9 locate`.
fn locate(args: &Args) -> Result<ExitCode, String> {
    let repo = Repo::find()?;
    let name = args
        .target
        .as_deref()
        .ok_or("give the release, such as 11952")?;
    let tree = args
        .get("tree")
        .ok_or("give --tree, the release's tree on the host")?;
    let window = args
        .get("days")
        .map_or(Ok(90), str::parse)
        .map_err(|e| format!("--days: {e}"))?;
    print!(
        "{}",
        r9::locate::run(
            &repo,
            name,
            std::path::Path::new(tree),
            window,
            args.has("write")
        )?
    );
    Ok(ExitCode::SUCCESS)
}

/// `r9 optab` and `r9 objcheck`: the instruction tables of `6l` and `7l`, and the check against
/// them.
fn run_optab(args: &Args) -> Result<ExitCode, String> {
    match args.command.as_str() {
        "objcheck" => {
            let path = args
                .target
                .as_deref()
                .ok_or("give a .6 or .7 file, or an archive of them")?;
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let (report, bad) = r9::objcheck::file(&r9::objcheck::Checker::new(), path, &bytes)?;
            print!("{report}");
            Ok(if bad == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "optab" => {
            let root = args
                .get("root")
                .ok_or("give --root, a tree with sys/src/cmd/6l or sys/src/cmd/7l")?;
            let root = std::path::Path::new(root);
            let pin = args.get("pin").unwrap_or("(not given)");
            let text = match args.get("row").unwrap_or("amd64") {
                "amd64" => r9::optab::load(root)?.rust(pin),
                "arm64" => r9::optab7l::load(root)?.rust(pin),
                row => {
                    return Err(format!(
                        "r9 optab has no row {row}; the rows are amd64 and arm64"
                    ));
                }
            };
            match args.get("out") {
                Some(out) => std::fs::write(out, text).map_err(|e| format!("{out}: {e}"))?,
                None => print!("{text}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => unreachable!("run_optab takes objcheck and optab"),
    }
}

/// Run `r9 objdiff`: list one object, or compare two.
fn run_objdiff(args: &Args) -> Result<ExitCode, String> {
    let a = args
        .target
        .as_deref()
        .ok_or("give a .6 or .7 file, or an archive of them")?;
    let read = |p: &str| std::fs::read(p).map_err(|e| format!("{p}: {e}"));
    let bytes = read(a)?;
    let Some(b) = args.get("with") else {
        print!("{}", r9::obj::listing(a, &bytes)?);
        return Ok(ExitCode::SUCCESS);
    };
    let context = args
        .get("context")
        .map_or(Ok(3), str::parse)
        .map_err(|e| format!("--context: {e}"))?;
    let (report, same) = r9::obj::diff_files(a, &bytes, b, &read(b)?, context)?;
    print!("{report}");
    Ok(if same {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `r9 sdiff`: two objects side by side, function by function.
fn run_helpers(args: &Args) -> Result<ExitCode, String> {
    let path = args
        .target
        .as_deref()
        .ok_or("give a directory of objects and archives, or one file")?;
    if let Some(g) = args.get("with") {
        let r = r9::helpers::load_relative(std::path::Path::new(path))?;
        let g = r9::helpers::load_relative(std::path::Path::new(g))?;
        let (found, pairs, alone) = r9::helpers::added(&r, &g);
        print!("{}", r9::helpers::added_report(&found, pairs, &alone));
        return Ok(if found.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let src = args
        .get("src")
        .ok_or("give the tree with --src, or G objects with --with")?;
    let objects = r9::helpers::load(std::path::Path::new(path))?;
    let (ids, sources) = r9::helpers::source_identifiers(std::path::Path::new(src))?;
    let found = r9::helpers::helpers(&objects, &ids);
    print!("{}", r9::helpers::report(objects.len(), sources, &found));
    Ok(ExitCode::SUCCESS)
}

fn run_sdiff(args: &Args) -> Result<ExitCode, String> {
    let a = args
        .target
        .as_deref()
        .ok_or("give a .6 or .7 file, or an archive of them")?;
    let b = args.get("with").ok_or("give the other file with --with")?;
    let read = |p: &str| std::fs::read(p).map_err(|e| format!("{p}: {e}"));
    let width = args
        .get("width")
        .map_or(Ok(60), str::parse)
        .map_err(|e| format!("--width: {e}"))?;
    let opt = r9::sdiff::Options {
        function: args.get("function"),
        width,
        all: args.has("all"),
    };
    let (report, differ) = r9::sdiff::files(a, &read(a)?, b, &read(b)?, opt)?;
    print!("{report}");
    Ok(if differ {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// `r9 tree-diff`: compare the files of two builds, or only their names and modes.
/// The bring-up of the build that wrote `records`, from the `build.toml` of `r9 build` next to
/// the `collected` directory. `None` when there is no such file, or when an older r9 wrote it.
fn census_bringup(records: &std::path::Path) -> Option<String> {
    let dir = records.parent()?.parent()?;
    let text = std::fs::read_to_string(dir.join("build.toml")).ok()?;
    let table: toml::Table = text.parse().ok()?;
    table.get("bringup")?.as_str().map(str::to_owned)
}

fn tree_diff(args: &Args) -> Result<ExitCode, String> {
    let a = args
        .target
        .as_deref()
        .ok_or("give the directory of the first build")?;
    let b = args
        .get("with")
        .ok_or("give --with, the directory of the second build")?;
    let top = args
        .get("top")
        .map_or(Ok(20), str::parse)
        .map_err(|e| format!("--top: {e}"))?;
    let a = r9::treediff::load(std::path::Path::new(a))?;
    let b = r9::treediff::load(std::path::Path::new(b))?;
    if args.has("names") {
        let n = r9::treediff::names(&a, &b);
        print!("{}", r9::treediff::report_names(&n, top));
        return Ok(if n.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let d = r9::treediff::diff(&a, &b);
    print!("{}", r9::treediff::report(&d, top));
    Ok(if d.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Run a command that reads the compile records of a build.
fn run_records(args: &Args) -> Result<ExitCode, String> {
    match args.command.as_str() {
        "flags-diff" => {
            let a = args
                .target
                .as_deref()
                .ok_or("give the compile.jsonl file of the first build")?;
            let b = args
                .get("with")
                .ok_or("give --with, the compile.jsonl file of the second build")?;
            let top = args
                .get("top")
                .map_or(Ok(20), str::parse)
                .map_err(|e| format!("--top: {e}"))?;
            let a = r9::census::load(std::path::Path::new(a))?;
            let b = r9::census::load(std::path::Path::new(b))?;
            let d = r9::flagsdiff::diff(&a, &b);
            print!("{}", r9::flagsdiff::report(&d, top));
            Ok(if d.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "warn-diff" => {
            print!("{}", r9::warndiff::command(args)?);
            Ok(ExitCode::SUCCESS)
        }
        "9h" => run_9h(args),
        "gui" => run_gui(args),
        "hostcheck" => run_hostcheck(args),
        "tree-diff" => tree_diff(args),
        "objdiff" => run_objdiff(args),
        "sdiff" => run_sdiff(args),
        "helpers" => run_helpers(args),
        "objcheck" | "optab" => run_optab(args),
        "census" => {
            let path = args
                .target
                .as_deref()
                .ok_or("give the compile.jsonl file of a build")?;
            let records = r9::census::load(std::path::Path::new(path))?;
            let side = r9::census::Side::pick(args.get("side"), &records)?;
            let census = r9::census::take(&records);
            let bringup = census_bringup(std::path::Path::new(path));
            print!("{}", census.report(side, bringup.as_deref()));
            Ok(if census.problems(side).is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "demands" => {
            let path = args
                .target
                .as_deref()
                .ok_or("give the compile.jsonl file of a G build")?;
            let root = args
                .get("root")
                .ok_or("give --root, the guest's tree on the host")?;
            let top = args
                .get("top")
                .map_or(Ok(20), str::parse)
                .map_err(|e| format!("--top: {e}"))?;
            let records = r9::census::load(std::path::Path::new(path))?;
            let rucc = r9::demands::find_rucc(args.get("flags"));
            let mode = if args.has("kencc") {
                r9::demands::Mode::Kencc
            } else {
                r9::demands::Mode::Syntax
            };
            let demands = r9::demands::run(&rucc, std::path::Path::new(root), &records, mode);
            demands.write(std::path::Path::new(
                args.get("out").unwrap_or("demands.toml"),
            ))?;
            print!("{}", demands.report(top));
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other}; see r9 help")),
    }
}

/// `r9 hostcheck`.
fn run_hostcheck(args: &Args) -> Result<ExitCode, String> {
    let repo = Repo::find()?;
    let out = args.get("out").map_or_else(
        || repo.root.join("work").join("hostcheck"),
        std::path::PathBuf::from,
    );
    let mut req = r9::h9boot::request_from(&repo, args, false)?;
    req.work.clone_from(&out);
    if args.get("timeout").is_none() {
        req.timeout = std::time::Duration::from_secs(600);
    }
    let mut x11 = r9::hostcheck::X11::default();
    let mut control = r9::hostcheck::Control::default();
    let mut wayland = r9::hostcheck::Wayland::new(&req.monitor);
    let mut cocoa = r9::hostcheck::Cocoa::default();
    let default = if cfg!(target_os = "macos") {
        "cocoa"
    } else {
        "x11"
    };
    let driver: &mut dyn r9::hostcheck::Driver = match args.get("host").unwrap_or(default) {
        "x11" => &mut x11,
        "wayland" => &mut wayland,
        "cocoa" => &mut cocoa,
        "control" => &mut control,
        h => {
            return Err(format!(
                "--host {h}: the hosts are x11, wayland, cocoa and control"
            ));
        }
    };
    let record = r9::hostcheck::run(&req, driver)?;
    print!("{}", r9::hostcheck::report(&record));
    if record.passed {
        println!("PASS hostcheck {} in {}", record.host, out.display());
        Ok(ExitCode::SUCCESS)
    } else {
        println!("FAIL hostcheck {}; see {}", record.host, out.display());
        Ok(ExitCode::FAILURE)
    }
}

/// `r9 gui run` and `r9 gui compare`.
fn run_gui(args: &Args) -> Result<ExitCode, String> {
    use std::path::{Path, PathBuf};
    let path = |k: &str| {
        args.get(k)
            .map(PathBuf::from)
            .ok_or_else(|| format!("give --{k}"))
    };
    match args.target.as_deref() {
        Some("run") => {
            let repo = Repo::find()?;
            let script = path("script")?;
            let name = script
                .file_stem()
                .map_or_else(|| "gui".into(), |s| s.to_string_lossy().to_string());
            let out = args.get("out").map_or_else(
                || repo.root.join("work").join("gui").join(&name),
                PathBuf::from,
            );
            // The data files of the scripts, such as images and ROMs, are in the directory data
            // next to the script.
            let data = script.parent().unwrap_or(Path::new(".")).join("data");
            let data = data.is_dir().then(|| data.canonicalize().unwrap_or(data));
            let record = match args.get("on").unwrap_or("9h") {
                "9h" => {
                    let mut req = r9::h9boot::request_from(&repo, args, false)?;
                    req.work.clone_from(&out);
                    req.snap = Some(out.join("screen.png"));
                    if req.hostfs.is_none() {
                        req.hostfs = data;
                    }
                    if args.get("timeout").is_none() {
                        req.timeout = std::time::Duration::from_secs(600);
                    }
                    r9::gui::run(&req, &script)?
                }
                "qemu" => {
                    let mut req = gq_request(&repo, args, &out)?;
                    let bases = repo.root.join("work").join("gq");
                    req.base = r9::qgui::prepare(&req, data.as_deref(), &bases)?;
                    r9::gui::run_q(&req, &script)?
                }
                o => return Err(format!("--on {o}: give 9h or qemu")),
            };
            for s in &record.snaps {
                println!("{} {}", s.sha256, s.name);
            }
            match &record.failure {
                None => {
                    println!("PASS {} in {}", script.display(), out.display());
                    Ok(ExitCode::SUCCESS)
                }
                Some(f) => {
                    println!("FAIL {}: {f}", script.display());
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        Some("compare") => {
            let (want, got) = (path("want")?, path("got")?);
            let diff = args
                .get("diff")
                .map_or_else(|| got.join("diff"), PathBuf::from);
            match r9::gui::compare(Path::new(&want), Path::new(&got), &diff)? {
                None => {
                    println!("SAME {} {}", want.display(), got.display());
                    Ok(ExitCode::SUCCESS)
                }
                Some(d) => {
                    print!("DIFFERENT at snap {:?}: {}", d.snap, d.what);
                    if d.pixels > 0 {
                        print!(", {} pixels", d.pixels);
                    }
                    if let Some([x0, y0, x1, y1]) = d.bounds {
                        print!(" in {x0},{y0} to {x1},{y1}; see {}", diff.display());
                    }
                    println!();
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        Some(w) => Err(format!("gui {w}: the gui commands are: run and compare")),
        None => Err("give a gui command: run or compare".into()),
    }
}

/// The request of `r9 gui run --on qemu`: the root image of `--root` on the row of `--row`, amd64
/// by default, with no commands.
fn gq_request(
    repo: &Repo,
    args: &Args,
    out: &std::path::Path,
) -> Result<r9::boot::Request, String> {
    use std::path::PathBuf;
    let objtype = args.get("row").unwrap_or("amd64").to_string();
    if objtype != "amd64" {
        return Err(format!(
            "--row {objtype}: G/Q runs only on amd64, because the arm64 kernel of 9front under QEMU has no screen"
        ));
    }
    let rows = r9_pins::rows::Rows::load(&repo.root)?;
    let row = rows.get(&objtype)?.clone();
    let base = PathBuf::from(
        args.get("root")
            .ok_or("give --root, the qcow2 root image")?,
    );
    if !base.is_file() {
        return Err(format!("{} is not a file", base.display()));
    }
    let base = base.canonicalize().unwrap_or(base);
    let accel = args
        .get("accel")
        .map_or_else(|| r9::boot::accel_for(&row), str::to_string);
    let timeout = args.get("timeout").map_or(Ok(600), |t| {
        t.parse()
            .map_err(|_| format!("--timeout {t}: give a number of seconds"))
    })?;
    Ok(r9::boot::Request {
        objtype,
        row,
        base,
        work: out.to_path_buf(),
        accel,
        firmware: None,
        bootargs: String::new(),
        commands: Vec::new(),
        extra: None,
        disks: Vec::new(),
        input: None,
        collect_mib: None,
        timeout: std::time::Duration::from_secs(timeout),
        keep: args.has("keep"),
        net: false,
    })
}

/// `r9 9h verify`, `r9 9h boot` and `r9 9h smoke`.
fn run_9h(args: &Args) -> Result<ExitCode, String> {
    let smoke = match args.target.as_deref() {
        Some("verify") => false,
        Some(w @ ("boot" | "smoke")) => w == "smoke",
        Some(w) => {
            return Err(format!(
                "9h {w}: the 9h commands are: verify, boot and smoke"
            ));
        }
        None => return Err("give a 9h command: verify, boot or smoke".into()),
    };
    let repo = Repo::find()?;
    if args.target.as_deref() != Some("verify") {
        let req = r9::h9boot::request_from(&repo, args, smoke)?;
        let outcome = r9::h9boot::run(&req)?;
        print!("{}", r9::boot::summary(&outcome, &req.work));
        return Ok(if outcome.passed {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let tree = match args.get("tree") {
        Some(t) => std::path::PathBuf::from(t),
        None => r9::h9verify::pinned_tree(&repo, &repo.root.join("work/9h/verify"))?,
    };
    let tree = tree.as_path();
    let row = args.get("row").unwrap_or("arm64");
    let h9 = repo.root.join("9h");
    let mut v = r9::h9verify::check_trees(tree, &h9, row);
    if let Some(path) = args.get("records") {
        let records = r9::census::load(std::path::Path::new(path))?;
        r9::h9verify::check_records(&mut v, &records, tree, &h9, row);
    }
    print!("{}", v.report(args.get("records").is_some()));
    Ok(if v.problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
