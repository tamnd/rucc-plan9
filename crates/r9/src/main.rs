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
        "build" | "mix" => {
            let repo = Repo::find()?;
            let (passed, report) = r9::build::command(&repo, args)?;
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

/// `r9 optab` and `r9 objcheck`: the instruction tables of `6l`, and the check against them.
fn run_optab(args: &Args) -> Result<ExitCode, String> {
    match args.command.as_str() {
        "objcheck" => {
            let path = args
                .target
                .as_deref()
                .ok_or("give a .6 file or an archive of them")?;
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
                .ok_or("give --root, a tree with sys/src/cmd/6l")?;
            let t = r9::optab::load(std::path::Path::new(root))?;
            let text = t.rust(args.get("pin").unwrap_or("(not given)"));
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
        "tree-diff" => {
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
            let d = r9::treediff::diff(&a, &b);
            print!("{}", r9::treediff::report(&d, top));
            Ok(if d.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
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
            print!("{}", census.report(side));
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
