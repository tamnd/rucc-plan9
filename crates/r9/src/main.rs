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
        "build" => {
            let repo = Repo::find()?;
            let (summary, report) = r9::build::command(&repo, args)?;
            print!("{report}");
            Ok(if summary.passed {
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
        "locate" => {
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
                r9::locate::run(&repo, name, std::path::Path::new(tree), window)?
            );
            Ok(ExitCode::SUCCESS)
        }
        _ => run_records(args),
    }
}

/// Run a command that reads the compile records of a build.
fn run_records(args: &Args) -> Result<ExitCode, String> {
    match args.command.as_str() {
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
            let demands = r9::demands::run(&rucc, std::path::Path::new(root), &records);
            demands.write(std::path::Path::new(
                args.get("out").unwrap_or("demands.toml"),
            ))?;
            print!("{}", demands.report(top));
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other}; see r9 help")),
    }
}
