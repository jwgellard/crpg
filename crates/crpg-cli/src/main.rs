#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `crpgc` — the campaign toolchain CLI: validate, migrate, pack, run,
//! replay and diff. No Godot, no rendering. T009b ships the `replay`
//! subcommand; the rest arrive with their tasks.

mod apply;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use crpg_testkit::ReplayError;

/// Usage line printed for the `replay` subcommand, kept in sync with the
/// contract in `tasks/T009b.md`.
const REPLAY_USAGE: &str = "crpgc replay <replay-path> [--golden <golden-path>]";

/// Parsed command line. One variant per subcommand; T009b has exactly one.
enum Command {
    Replay {
        replay_path: PathBuf,
        golden_path: PathBuf,
    },
}

/// A failure with a process exit code. Usage errors are `2` (clap's
/// convention, kept so a later T013 parser swap stays script-compatible);
/// replay-domain failures are `1`; success is `0`.
#[derive(Debug)]
enum CliError {
    Usage(String),
    Replay(ReplayError),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            CliError::Usage(_) => 2,
            CliError::Replay(_) => 1,
        }
    }
}

/// Parses `argv[1..]` (subcommand and arguments) into a [`Command`].
fn parse_args(args: &[String]) -> Result<Command, CliError> {
    let sub = args
        .first()
        .ok_or_else(|| CliError::Usage(format!("missing subcommand; try '{REPLAY_USAGE}'")))?;
    match sub.as_str() {
        "replay" => parse_replay(&args[1..]),
        other => Err(CliError::Usage(format!("unknown subcommand '{other}'"))),
    }
}

/// Parses the `replay` subcommand's arguments. `--golden` may appear once;
/// omitted, the golden defaults to the replay path with its extension
/// replaced by `.golden` (`foo.replay` -> `foo.golden`).
fn parse_replay(args: &[String]) -> Result<Command, CliError> {
    let replay = args
        .first()
        .ok_or_else(|| CliError::Usage(format!("{REPLAY_USAGE}: missing <replay-path>")))?;
    let mut golden: Option<&String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--golden" => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::Usage("--golden needs a value".to_string()))?;
                if golden.replace(value).is_some() {
                    return Err(CliError::Usage("--golden given more than once".to_string()));
                }
            }
            other if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown flag '{other}'")));
            }
            other => {
                return Err(CliError::Usage(format!("unexpected argument '{other}'")));
            }
        }
    }
    let golden_path = match golden {
        Some(path) => PathBuf::from(path),
        None => {
            let mut default = PathBuf::from(replay);
            default.set_extension("golden");
            default
        }
    };
    Ok(Command::Replay {
        replay_path: PathBuf::from(replay),
        golden_path,
    })
}

/// Runs a parsed command through the testkit replay API. The binary owns
/// paths, argument parsing, and the exit code; every replay-semantics
/// decision lives in `crpg-testkit::play_and_verify` or the apply.
fn run(command: Command) -> Result<(), CliError> {
    match command {
        Command::Replay {
            replay_path,
            golden_path,
        } => crpg_testkit::play_and_verify(&replay_path, &golden_path, apply::reference_intents())
            .map(|_| ())
            .map_err(CliError::Replay),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match parse_args(&args).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            match &err {
                CliError::Usage(msg) => eprintln!("crpgc: {msg}"),
                CliError::Replay(e) => eprintln!("crpgc replay: {e}"),
            }
            ExitCode::from(err.exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_args_is_usage() {
        assert!(matches!(parse_args(&[]), Err(CliError::Usage(_))));
    }

    #[test]
    fn unknown_subcommand_is_usage() {
        assert!(matches!(
            parse_args(&args(&["migrate"])),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn replay_missing_path_is_usage() {
        assert!(matches!(
            parse_args(&args(&["replay"])),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn replay_defaults_golden_to_sibling() {
        let command = parse_args(&args(&["replay", "run/replay_basic.replay"])).expect("parses");
        match command {
            Command::Replay {
                replay_path,
                golden_path,
            } => {
                assert_eq!(replay_path, PathBuf::from("run/replay_basic.replay"));
                assert_eq!(golden_path, PathBuf::from("run/replay_basic.golden"));
            }
        }
    }

    #[test]
    fn replay_explicit_golden_wins() {
        let command = parse_args(&args(&[
            "replay",
            "run/replay_basic.replay",
            "--golden",
            "g.golden",
        ]))
        .expect("parses");
        match command {
            Command::Replay { golden_path, .. } => {
                assert_eq!(golden_path, PathBuf::from("g.golden"));
            }
        }
    }

    #[test]
    fn replay_flag_after_positional_is_accepted() {
        assert!(parse_args(args(&["replay", "a.replay", "--golden", "g"]).as_slice()).is_ok());
    }

    #[test]
    fn unknown_flag_is_usage() {
        assert!(matches!(
            parse_args(&args(&["replay", "a.replay", "--bogus"])),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn golden_without_value_is_usage() {
        assert!(matches!(
            parse_args(&args(&["replay", "a.replay", "--golden"])),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn duplicate_golden_is_usage() {
        assert!(matches!(
            parse_args(&args(&[
                "replay", "a.replay", "--golden", "g", "--golden", "h"
            ])),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn exit_codes_follow_the_contract() {
        assert_eq!(CliError::Usage("x".into()).exit_code(), 2);
        assert!(matches!(
            CliError::Replay(ReplayError::UnsupportedVersion { found: 2 }).exit_code(),
            1
        ));
    }
}
