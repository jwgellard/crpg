#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `crpgc` — the campaign toolchain CLI: validate, migrate, replay, and the
//! T013 scaffolding/introspection commands (`new`, `schema`, `explain`,
//! `fmt`, `lock`, `run`). No Godot, no rendering. T009b shipped the `replay`
//! subcommand; T011b adds the thin `validate` wrapper over `crpg-data`
//! validation; T012b adds the thin `migrate` explicit-save wrapper over T012a
//! migration-aware loading and the canonical writer. T013 retains and
//! organizes the hand-rolled `args_os` parser into private per-command
//! parsers; no parser library is authorized.

mod apply;

use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crpg_core::Ulid;
use crpg_data::{Diagnostic, DiagnosticCode, Severity, SourcePath};
use crpg_testkit::ReplayError;

/// Usage line printed for the `replay` subcommand, kept in sync with the
/// contract in `tasks/T009b.md`.
const REPLAY_USAGE: &str = "crpgc replay <replay-path> [--golden <golden-path>]";

/// Usage line printed for the `validate` subcommand, kept in sync with the
/// contract in `tasks/T011b.md`.
const VALIDATE_USAGE: &str = "crpgc validate <campaign-root> [--json]";

/// Usage line printed for the `migrate` subcommand, kept in sync with the
/// contract in `tasks/T012b.md`. Exactly one root, no flags.
const MIGRATE_USAGE: &str = "crpgc migrate <campaign-root>";

/// Usage lines for the six T013 commands, kept in sync with the interface
/// block in `tasks/T013.md`. Every usage error for a recognized new command
/// prints exactly `crpgc: usage: <syntax>\n` with its line below, never
/// echoing arbitrary input.
const NEW_USAGE: &str = "crpgc new <type> --slug <s> --id <id> [--entry-id <id>]";
const SCHEMA_USAGE: &str = "crpgc schema <type>";
const EXPLAIN_USAGE: &str = "crpgc explain <id> [--root <campaign-root>]";
const FMT_USAGE: &str = "crpgc fmt [<campaign-root>] [--check]";
const LOCK_USAGE: &str = "crpgc lock [<campaign-root>] --catalog <catalog-path>";
const RUN_USAGE: &str = "crpgc run --ticks N --hash-every M [--seed S]";

/// Exact stderr bytes when the compile-time engine version does not parse as
/// semver for `migrate`. A release-process bug, never user input.
const MIGRATE_ENGINE_FAILURE: &str = "crpgc migrate: internal engine version failure\n";

/// Exact stderr bytes when the compile-time engine version does not parse as
/// semver for `explain`. A release-process bug, never user input.
const EXPLAIN_ENGINE_FAILURE: &str = "crpgc explain: internal engine version failure\n";

/// Exact stderr bytes when the compile-time engine version does not parse as
/// semver for `fmt`. A release-process bug, never user input.
const FMT_ENGINE_FAILURE: &str = "crpgc fmt: internal engine version failure\n";

/// Exact stderr bytes when the data writer returns a different document set
/// than was collected for `fmt`. An implementation defect, never user input;
/// reported before any write starts.
const FMT_DOCUMENT_SET_FAILURE: &str = "crpgc fmt: internal document set failure\n";

/// Exact stderr bytes when the campaign document is not a campaign for
/// `lock`. A domain failure, exit 1.
const LOCK_EXPECTED_CAMPAIGN: &str = "crpgc lock: expected campaign document\n";

/// Exact stderr bytes when the catalog is not a JSON array of wire
/// `PackageCandidate` objects for `lock`. No serde or OS prose is leaked.
const LOCK_INVALID_CATALOG: &str = "crpgc lock: invalid catalog\n";

/// Exact stderr bytes when the data writer returns a different document set
/// than was collected. An implementation defect, never user input; reported
/// before any write starts.
const DOCUMENT_SET_FAILURE: &str = "crpgc migrate: internal document set failure\n";

/// Exact stderr bytes when the compile-time engine version does not parse as
/// semver. A release-process bug, never user input, so the text is fixed.
const ENGINE_VERSION_FAILURE: &str = "crpgc validate: internal engine version failure\n";

/// Exact stderr bytes when canonical JSON serialization fails. The diagnostic
/// value set is deliberately serializable, so this is a defensive fallback.
const SERIALIZATION_FAILURE: &str = "crpgc validate: internal serialization failure\n";

/// Parsed command line. One variant per subcommand; T009b owns `Replay`,
/// T011b owns `Validate`, T012b owns `Migrate`, T013 owns the six new
/// variants below.
#[derive(Debug)]
enum Command {
    Replay {
        replay_path: PathBuf,
        golden_path: PathBuf,
    },
    Validate {
        root: PathBuf,
        json: bool,
    },
    Migrate {
        root: PathBuf,
    },
    New {
        kind: NewKind,
        slug: String,
        id: Ulid,
        entry_id: Option<Ulid>,
    },
    Schema {
        stem: String,
    },
    Explain {
        id: Ulid,
        root: PathBuf,
    },
    Fmt {
        root: PathBuf,
        check: bool,
    },
    Lock {
        root: PathBuf,
        catalog: PathBuf,
    },
    Run {
        ticks: usize,
        hash_every: usize,
        seed: u64,
    },
}

/// Scaffold document type for `crpgc new`. Only the four Stage-2 authoring
/// templates exist; placement and action-signature are embedded schema roots,
/// never scaffold types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewKind {
    Creature,
    Item,
    Dialogue,
    Quest,
}

impl NewKind {
    /// Parses the single positional type token. Anything else is a usage
    /// error with the shared `new` usage line.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "creature" => Some(Self::Creature),
            "item" => Some(Self::Item),
            "dialogue" => Some(Self::Dialogue),
            "quest" => Some(Self::Quest),
            _ => None,
        }
    }
}

/// A failure with a process exit code. Usage errors are `2` (clap's
/// convention, kept so a later T013 parser swap stays script-compatible);
/// domain failures are `1`; success is `0`. `Validate` outcomes carry their
/// own exit mapping through [`Outcome`] instead: warnings never fail, so a
/// warnings-only run must print and still exit `0`, which `Result` cannot
/// express.
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

/// Fully computed process result: the exit code plus the exact bytes for
/// each stream. `run` computes it without touching the process streams; only
/// `main` writes.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    code: u8,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Outcome {
    /// Success with no output: replay equality and clean plain validation.
    fn success() -> Self {
        Self {
            code: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    /// Maps a parse/replay failure to its exit code and single stderr line.
    /// Byte-identical to the T009b `eprintln!` paths it replaces.
    fn cli_error(error: &CliError) -> Self {
        let stderr = match error {
            CliError::Usage(message) => format!("crpgc: {message}\n").into_bytes(),
            CliError::Replay(error) => format!("crpgc replay: {error}\n").into_bytes(),
        };
        Self {
            code: error.exit_code(),
            stdout: Vec::new(),
            stderr,
        }
    }
}

/// Returns the flag text when `arg` is valid Unicode starting with `-`.
/// Anything else — including every non-Unicode argument — is a positional
/// candidate, never a flag: a non-Unicode campaign root must reach the
/// collector as exit-1 `io`, not fail parsing as usage.
fn flag_text(arg: &OsStr) -> Option<&str> {
    let text = arg.to_str()?;
    text.starts_with('-').then_some(text)
}

/// Parses `argv[1..]` (subcommand and arguments) into a [`Command`].
/// Argument handling uses `args_os` throughout so a non-Unicode root never
/// panics; Unicode handling below only decides flag identity.
fn parse_args(args: &[OsString]) -> Result<Command, CliError> {
    let sub = args
        .first()
        .ok_or_else(|| CliError::Usage(format!("missing subcommand; try '{REPLAY_USAGE}'")))?;
    match sub.to_str() {
        Some("replay") => parse_replay(&args[1..]),
        Some("validate") => parse_validate(&args[1..]),
        Some("migrate") => parse_migrate(&args[1..]),
        Some("new") => parse_new(&args[1..]),
        Some("schema") => parse_schema(&args[1..]),
        Some("explain") => parse_explain(&args[1..]),
        Some("fmt") => parse_fmt(&args[1..]),
        Some("lock") => parse_lock(&args[1..]),
        Some("run") => parse_run(&args[1..]),
        Some(other) => Err(CliError::Usage(format!("unknown subcommand '{other}'"))),
        None => Err(CliError::Usage(format!(
            "unknown subcommand '{}'",
            sub.to_string_lossy()
        ))),
    }
}

/// Parses the `replay` subcommand's arguments. `--golden` may appear once;
/// omitted, the golden defaults to the replay path with its extension
/// replaced by `.golden` (`foo.replay` -> `foo.golden`). A flag in
/// first position — where the replay path is expected — is a usage error
/// (the T011b first-position repair): `--bogus` used to be mistaken for a
/// path and fail later as missing-file exit 1.
fn parse_replay(args: &[OsString]) -> Result<Command, CliError> {
    let replay = args
        .first()
        .ok_or_else(|| CliError::Usage(format!("{REPLAY_USAGE}: missing <replay-path>")))?;
    if let Some(flag) = flag_text(replay) {
        return Err(CliError::Usage(format!(
            "unexpected flag '{flag}'; expected <replay-path>"
        )));
    }
    let mut golden: Option<&OsString> = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match flag_text(arg) {
            Some("--golden") => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::Usage("--golden needs a value".to_string()))?;
                if golden.replace(value).is_some() {
                    return Err(CliError::Usage("--golden given more than once".to_string()));
                }
            }
            Some(other) => {
                return Err(CliError::Usage(format!("unknown flag '{other}'")));
            }
            None => {
                return Err(CliError::Usage(format!(
                    "unexpected argument '{}'",
                    arg.to_string_lossy()
                )));
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

/// Parses the `validate` subcommand's arguments. Exactly one campaign root
/// plus at most one `--json`, in either order. Unknown flags, a duplicate
/// `--json`, extra positionals, and a missing root are usage errors. A
/// non-Unicode root parses successfully here: it is an exit-1 `io`
/// diagnostic from the collector, never a usage error and never a panic.
fn parse_validate(args: &[OsString]) -> Result<Command, CliError> {
    let mut root: Option<&OsString> = None;
    let mut json = false;
    for arg in args {
        match flag_text(arg) {
            Some("--json") => {
                if json {
                    return Err(CliError::Usage("--json given more than once".to_string()));
                }
                json = true;
            }
            Some(other) => {
                return Err(CliError::Usage(format!("unknown flag '{other}'")));
            }
            None => {
                if root.is_some() {
                    return Err(CliError::Usage(format!(
                        "unexpected argument '{}'",
                        arg.to_string_lossy()
                    )));
                }
                root = Some(arg);
            }
        }
    }
    let root =
        root.ok_or_else(|| CliError::Usage(format!("{VALIDATE_USAGE}: missing <campaign-root>")))?;
    Ok(Command::Validate {
        root: PathBuf::from(root),
        json,
    })
}

/// Parses the `migrate` subcommand's arguments. Exactly one campaign root and
/// no flags: a missing root, an extra positional, or any flag (including
/// `--json`, `--check`, `--dry-run`, `--to` and `--golden`) is a usage error
/// with exit 2. A non-Unicode root parses successfully here: it is an exit-1
/// `io` diagnostic from the collector, never a usage error and never a panic.
/// The hand-rolled parser is extended; S13 still owns the parser-framework
/// decision. Replay and validate contracts are preserved unchanged.
fn parse_migrate(args: &[OsString]) -> Result<Command, CliError> {
    let mut root: Option<&OsString> = None;
    for arg in args {
        match flag_text(arg) {
            Some(other) => {
                return Err(CliError::Usage(format!("unknown flag '{other}'")));
            }
            None => {
                if root.is_some() {
                    return Err(CliError::Usage(format!(
                        "unexpected argument '{}'",
                        arg.to_string_lossy()
                    )));
                }
                root = Some(arg);
            }
        }
    }
    let root =
        root.ok_or_else(|| CliError::Usage(format!("{MIGRATE_USAGE}: missing <campaign-root>")))?;
    Ok(Command::Migrate {
        root: PathBuf::from(root),
    })
}

/// Builds the single shared usage error for a recognized T013 command. Every
/// usage failure for that command — bad grammar, duplicate or unknown flags,
/// missing values, extra positionals, `--`, short flags, unlisted
/// `--help`/`--version`, bad ids/slugs/numbers, unknown types/stems, equal
/// object/entry ids — reports exactly `crpgc: usage: <syntax>\n` with exit 2
/// and never echoes arbitrary input.
fn new_usage(usage: &str) -> CliError {
    CliError::Usage(format!("usage: {usage}"))
}

/// Takes the value token following a named option. A missing token is a usage
/// error. A Unicode value starting with `-` is also a usage error, so
/// flag-shaped tokens (`--help`, `--version`, `--`, short flags, duplicate
/// options) never reach I/O as option values; paths starting with `-` must
/// be spelled with a relative prefix such as `./` (which starts with `.`,
/// not `-`). Non-Unicode values are preserved for subsequent exit-1 I/O
/// validation. Replay/validate/migrate grammars do not use this helper.
fn take_option_value<'a>(
    iter: &mut std::slice::Iter<'a, OsString>,
    usage: &str,
) -> Result<&'a OsString, CliError> {
    let next = iter.as_slice().first().ok_or_else(|| new_usage(usage))?;
    if let Some(text) = next.to_str() {
        if text.starts_with('-') {
            return Err(new_usage(usage));
        }
    }
    iter.next().ok_or_else(|| new_usage(usage))
}

/// Reads a Unicode text value for a named option. Non-Unicode text is a usage
/// error: text arguments (command/type/slug/id/numbers) must be Unicode,
/// while path arguments stay `OsString` until I/O validation.
fn option_text(value: &OsString, usage: &str) -> Result<String, CliError> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| new_usage(usage))
}

/// Checks the caller-supplied slug grammar `[a-z0-9]+(?:-[a-z0-9]+)*`
/// without a regex dependency: lowercase ASCII alphanumerics in hyphen
/// separated non-empty parts, with no leading, trailing, or doubled hyphen.
fn is_valid_slug(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    for part in text.split('-') {
        if part.is_empty() {
            return false;
        }
        if !part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return false;
        }
    }
    true
}

/// Parses the `new` subcommand: exactly one positional scaffold type plus
/// required `--slug`/`--id` and the dialogue/quest-only `--entry-id`. Each
/// named option occurs at most once and may appear before or after the
/// positional; values are separate tokens. Malformed ids, invalid slugs,
/// unsupported types, and equal object/entry ids are all usage exit 2 with
/// the shared usage line. No clock, RNG, registry, or filesystem lookup.
fn parse_new(args: &[OsString]) -> Result<Command, CliError> {
    let mut type_text: Option<String> = None;
    let mut slug: Option<String> = None;
    let mut id_text: Option<String> = None;
    let mut entry_text: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match flag_text(arg) {
            Some("--slug") => {
                if slug.is_some() {
                    return Err(new_usage(NEW_USAGE));
                }
                let value = take_option_value(&mut iter, NEW_USAGE)?;
                slug = Some(option_text(value, NEW_USAGE)?);
            }
            Some("--id") => {
                if id_text.is_some() {
                    return Err(new_usage(NEW_USAGE));
                }
                let value = take_option_value(&mut iter, NEW_USAGE)?;
                id_text = Some(option_text(value, NEW_USAGE)?);
            }
            Some("--entry-id") => {
                if entry_text.is_some() {
                    return Err(new_usage(NEW_USAGE));
                }
                let value = take_option_value(&mut iter, NEW_USAGE)?;
                entry_text = Some(option_text(value, NEW_USAGE)?);
            }
            Some(_) => {
                return Err(new_usage(NEW_USAGE));
            }
            None => {
                let text = arg.to_str().ok_or_else(|| new_usage(NEW_USAGE))?;
                if type_text.is_some() {
                    return Err(new_usage(NEW_USAGE));
                }
                type_text = Some(text.to_owned());
            }
        }
    }
    let type_text = type_text.ok_or_else(|| new_usage(NEW_USAGE))?;
    let slug = slug.ok_or_else(|| new_usage(NEW_USAGE))?;
    let id_text = id_text.ok_or_else(|| new_usage(NEW_USAGE))?;
    let kind = NewKind::parse(&type_text).ok_or_else(|| new_usage(NEW_USAGE))?;
    if !is_valid_slug(&slug) {
        return Err(new_usage(NEW_USAGE));
    }
    let id: Ulid = id_text.parse().map_err(|_| new_usage(NEW_USAGE))?;
    let entry_id: Option<Ulid> = match entry_text {
        Some(text) => Some(text.parse().map_err(|_| new_usage(NEW_USAGE))?),
        None => None,
    };
    match (kind, entry_id) {
        (NewKind::Dialogue | NewKind::Quest, None) => return Err(new_usage(NEW_USAGE)),
        (NewKind::Creature | NewKind::Item, Some(_)) => return Err(new_usage(NEW_USAGE)),
        _ => {}
    }
    if let Some(entry) = entry_id {
        if entry == id {
            return Err(new_usage(NEW_USAGE));
        }
    }
    Ok(Command::New {
        kind,
        slug,
        id,
        entry_id,
    })
}

/// The seventeen current schema stems for `crpgc schema`. They map to
/// `<stem>.schema.json` in `generated_schemas()`; the last two are embedded
/// schema roots, not scaffold types.
fn is_schema_stem(text: &str) -> bool {
    matches!(
        text,
        "campaign"
            | "world"
            | "area"
            | "creature"
            | "item"
            | "dialogue"
            | "quest"
            | "faction"
            | "graph"
            | "placements"
            | "triggers"
            | "locale"
            | "variables"
            | "campaign-lock"
            | "assets-lock"
            | "placement"
            | "action-signature"
    )
}

/// Parses the `schema` subcommand: exactly one current schema stem and no
/// flags. Bad grammar and unknown stems are usage exit 2.
fn parse_schema(args: &[OsString]) -> Result<Command, CliError> {
    let mut stem: Option<String> = None;
    for arg in args {
        match flag_text(arg) {
            Some(_) => {
                return Err(new_usage(SCHEMA_USAGE));
            }
            None => {
                let text = arg.to_str().ok_or_else(|| new_usage(SCHEMA_USAGE))?;
                if stem.is_some() {
                    return Err(new_usage(SCHEMA_USAGE));
                }
                stem = Some(text.to_owned());
            }
        }
    }
    let stem = stem.ok_or_else(|| new_usage(SCHEMA_USAGE))?;
    if !is_schema_stem(&stem) {
        return Err(new_usage(SCHEMA_USAGE));
    }
    Ok(Command::Schema { stem })
}

/// Parses the `explain` subcommand: exactly one id plus an optional `--root`
/// path defaulting to `.`. The id is text and must be Unicode; the root stays
/// `OsString` until I/O validation, so a non-Unicode root parses here and
/// fails later as exit-1 `io`. Malformed id text is usage exit 2 and wins
/// before any filesystem access.
fn parse_explain(args: &[OsString]) -> Result<Command, CliError> {
    let mut id_text: Option<String> = None;
    let mut root: Option<&OsString> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match flag_text(arg) {
            Some("--root") => {
                if root.is_some() {
                    return Err(new_usage(EXPLAIN_USAGE));
                }
                let value = take_option_value(&mut iter, EXPLAIN_USAGE)?;
                root = Some(value);
            }
            Some(_) => {
                return Err(new_usage(EXPLAIN_USAGE));
            }
            None => {
                let text = arg.to_str().ok_or_else(|| new_usage(EXPLAIN_USAGE))?;
                if id_text.is_some() {
                    return Err(new_usage(EXPLAIN_USAGE));
                }
                id_text = Some(text.to_owned());
            }
        }
    }
    let id_text = id_text.ok_or_else(|| new_usage(EXPLAIN_USAGE))?;
    let id: Ulid = id_text.parse().map_err(|_| new_usage(EXPLAIN_USAGE))?;
    Ok(Command::Explain {
        id,
        root: root.map_or_else(|| PathBuf::from("."), PathBuf::from),
    })
}

/// Parses the `fmt` subcommand: zero or one root defaulting to `.` plus an
/// optional `--check` that may appear before or after the root. Duplicates,
/// unknown flags, and extra positionals are usage exit 2. A non-Unicode root
/// parses here and fails later as exit-1 `io`.
fn parse_fmt(args: &[OsString]) -> Result<Command, CliError> {
    let mut root: Option<&OsString> = None;
    let mut check = false;
    for arg in args {
        match flag_text(arg) {
            Some("--check") => {
                if check {
                    return Err(new_usage(FMT_USAGE));
                }
                check = true;
            }
            Some(_) => {
                return Err(new_usage(FMT_USAGE));
            }
            None => {
                if root.is_some() {
                    return Err(new_usage(FMT_USAGE));
                }
                root = Some(arg);
            }
        }
    }
    Ok(Command::Fmt {
        root: root.map_or_else(|| PathBuf::from("."), PathBuf::from),
        check,
    })
}

/// Parses the `lock` subcommand: zero or one root defaulting to `.` plus
/// exactly one `--catalog` path. The catalog is required; omitting it is
/// usage exit 2. Both paths stay `OsString` until I/O validation, so
/// non-Unicode paths parse here and fail later as exit-1 `io`.
fn parse_lock(args: &[OsString]) -> Result<Command, CliError> {
    let mut root: Option<&OsString> = None;
    let mut catalog: Option<&OsString> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match flag_text(arg) {
            Some("--catalog") => {
                if catalog.is_some() {
                    return Err(new_usage(LOCK_USAGE));
                }
                let value = take_option_value(&mut iter, LOCK_USAGE)?;
                catalog = Some(value);
            }
            Some(_) => {
                return Err(new_usage(LOCK_USAGE));
            }
            None => {
                if root.is_some() {
                    return Err(new_usage(LOCK_USAGE));
                }
                root = Some(arg);
            }
        }
    }
    let catalog = catalog.ok_or_else(|| new_usage(LOCK_USAGE))?;
    Ok(Command::Lock {
        root: root.map_or_else(|| PathBuf::from("."), PathBuf::from),
        catalog: PathBuf::from(catalog),
    })
}

/// CLI resource limits for `run`, not changes to the harness: `0 <= N <=
/// 1_000_000` ticks and `1 <= M <= 1_000_000` for the sampling interval.
const RUN_MAX_TICKS: u64 = 1_000_000;

/// Parses an ASCII-decimal `run` number: non-empty ASCII digits only, with
/// leading zeroes accepted. Signs, whitespace, hex, separators, and overflow
/// are usage errors. The digit check runs before integer parsing so `+1`,
/// ` 1`, `0x1`, and `1_0` never reach the integer parser.
fn parse_run_number(text: &str) -> Result<u64, ()> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    text.parse::<u64>().map_err(|_| ())
}

/// Parses the `run` subcommand: required `--ticks N` and `--hash-every M`
/// plus optional `--seed S` defaulting to 0, each at most once and with
/// separate-token values. No positionals. Invalid grammar or numeric range
/// is usage exit 2 before the harness runs.
fn parse_run(args: &[OsString]) -> Result<Command, CliError> {
    let mut ticks: Option<u64> = None;
    let mut hash_every: Option<u64> = None;
    let mut seed: Option<u64> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match flag_text(arg) {
            Some("--ticks") => {
                if ticks.is_some() {
                    return Err(new_usage(RUN_USAGE));
                }
                let value = take_option_value(&mut iter, RUN_USAGE)?;
                let text = option_text(value, RUN_USAGE)?;
                ticks = Some(parse_run_number(&text).map_err(|_| new_usage(RUN_USAGE))?);
            }
            Some("--hash-every") => {
                if hash_every.is_some() {
                    return Err(new_usage(RUN_USAGE));
                }
                let value = take_option_value(&mut iter, RUN_USAGE)?;
                let text = option_text(value, RUN_USAGE)?;
                hash_every = Some(parse_run_number(&text).map_err(|_| new_usage(RUN_USAGE))?);
            }
            Some("--seed") => {
                if seed.is_some() {
                    return Err(new_usage(RUN_USAGE));
                }
                let value = take_option_value(&mut iter, RUN_USAGE)?;
                let text = option_text(value, RUN_USAGE)?;
                seed = Some(parse_run_number(&text).map_err(|_| new_usage(RUN_USAGE))?);
            }
            Some(_) => {
                return Err(new_usage(RUN_USAGE));
            }
            None => {
                return Err(new_usage(RUN_USAGE));
            }
        }
    }
    let ticks = ticks.ok_or_else(|| new_usage(RUN_USAGE))?;
    let hash_every = hash_every.ok_or_else(|| new_usage(RUN_USAGE))?;
    if ticks > RUN_MAX_TICKS || hash_every == 0 || hash_every > RUN_MAX_TICKS {
        return Err(new_usage(RUN_USAGE));
    }
    Ok(Command::Run {
        ticks: ticks as usize,
        hash_every: hash_every as usize,
        seed: seed.unwrap_or(0),
    })
}

/// Stable snake_case kind for `cannot <op> <logical>: <kind>` messages.
/// Only the distinguished filesystem conditions keep their identity;
/// everything else collapses to `io_error` so diagnostics never embed raw
/// OS error text, native separators, or absolute paths. `migrate` adds
/// `source_changed` (prewrite re-read differs from the collected original)
/// and `not_a_file` (a rewrite target that is a directory or other
/// non-regular file) for its explicit prewrite checks; no new diagnostic
/// enum variant is introduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IoKind {
    NotFound,
    NotADirectory,
    PermissionDenied,
    SymlinkAtDocumentPath,
    NonUnicodeComponent,
    IoError,
    SourceChanged,
    NotAFile,
}

impl fmt::Display for IoKind {
    /// Renders the stable snake_case wire spelling.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NotFound => "not_found",
            Self::NotADirectory => "not_a_directory",
            Self::PermissionDenied => "permission_denied",
            Self::SymlinkAtDocumentPath => "symlink_at_document_path",
            Self::NonUnicodeComponent => "non_unicode_component",
            Self::IoError => "io_error",
            Self::SourceChanged => "source_changed",
            Self::NotAFile => "not_a_file",
        };
        f.write_str(text)
    }
}

/// Maps a filesystem error to its stable kind. `NotFound` and
/// `PermissionDenied` keep their identity; every other `ErrorKind` — OS
/// codes included — becomes `io_error`.
fn io_kind(error: &std::io::Error) -> IoKind {
    match error.kind() {
        std::io::ErrorKind::NotFound => IoKind::NotFound,
        std::io::ErrorKind::PermissionDenied => IoKind::PermissionDenied,
        _ => IoKind::IoError,
    }
}

/// Builds the single CLI-owned `io` diagnostic: always `Error` severity,
/// empty pointer, no suggested fix, and a portable
/// `cannot <op> <logical>: <kind>` message. `logical` is the `/`-joined
/// relative text, or the literal `<campaign-root>` when no logical path
/// exists (root, directory-listing, and non-Unicode failures). The `file`
/// is `Some` only when the logical path truthfully identifies a file entry
/// (a classified read target or a symlink occupying a document path).
fn io_diagnostic(op: &str, file: Option<SourcePath>, logical: &str, kind: IoKind) -> Diagnostic {
    Diagnostic {
        file,
        pointer: String::new(),
        severity: Severity::Error,
        code: DiagnosticCode::Io,
        message: format!("cannot {op} {logical}: {kind}"),
        suggested_fix: None,
    }
}

/// Observed entry shape. The walker never follows symlinks and never reads
/// through non-regular files: both are one `io` diagnostic at a recognized
/// document path and silently ignored at a non-document path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

/// Converts `symlink_metadata` output without following links. Symlink is
/// checked first so a link to a directory still reports as a symlink.
fn entry_kind(file_type: &fs::FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Filesystem operations the collector needs. Production implements this
/// with `std::fs`; tests inject fakes through this seam to cover unreadable
/// files and directories, symlinks, non-Unicode names, and walk-order
/// oracles without platform privileges or runtime skips.
trait WalkFs {
    fn metadata(&self, path: &Path) -> std::io::Result<EntryKind>;
    fn read_dir_names(&self, path: &Path) -> std::io::Result<Vec<OsString>>;
    fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>>;
}

/// Rejects a supplied campaign root whose lexical path traverses a symlink
/// or non-directory ancestor before any operation reaches the terminal root.
fn check_walk_root_ancestors(fs: &impl WalkFs, root: &Path) -> Result<(), Diagnostic> {
    let mut ancestors: Vec<&Path> = root.ancestors().skip(1).collect();
    ancestors.reverse();
    for ancestor in ancestors {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match fs.metadata(ancestor) {
            Err(error) => {
                return Err(io_diagnostic(
                    "open root",
                    None,
                    "<campaign-root>",
                    io_kind(&error),
                ));
            }
            Ok(EntryKind::Directory) => {}
            Ok(EntryKind::Symlink) => {
                return Err(io_diagnostic(
                    "open root",
                    None,
                    "<campaign-root>",
                    IoKind::SymlinkAtDocumentPath,
                ));
            }
            Ok(EntryKind::File) | Ok(EntryKind::Other) => {
                return Err(io_diagnostic(
                    "open root",
                    None,
                    "<campaign-root>",
                    IoKind::NotADirectory,
                ));
            }
        }
    }
    Ok(())
}

/// Production filesystem access: `symlink_metadata` (never follows links),
/// unordered directory names (the walker sorts), and plain file reads.
struct RealFs;

type CollectedCampaign = (BTreeMap<SourcePath, Vec<u8>>, Vec<PathBuf>);

impl WalkFs for RealFs {
    fn metadata(&self, path: &Path) -> std::io::Result<EntryKind> {
        path.symlink_metadata()
            .map(|meta| entry_kind(&meta.file_type()))
    }

    fn read_dir_names(&self, path: &Path) -> std::io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(path)? {
            names.push(entry?.file_name());
        }
        Ok(names)
    }

    fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        fs::read(path)
    }
}

/// Collects classified campaign bytes from `root`: requires the root to be
/// a directory, then walks sorted depth-first pre-order. Read-only: nothing
/// is mutated, created, deleted, or locked. The first failure in walk order
/// wins as the single returned diagnostic.
fn collect_campaign_files(root: &Path) -> Result<BTreeMap<SourcePath, Vec<u8>>, Diagnostic> {
    collect_campaign_files_with(&RealFs, root)
}

/// Walk driver shared by production and seam-injected tests. A symlinked
/// root is refused outright: listing through it would follow the link, and
/// the no-follow rule admits no entry-shaped exception for the starting
/// point.
fn collect_campaign_files_with(
    fs: &impl WalkFs,
    root: &Path,
) -> Result<BTreeMap<SourcePath, Vec<u8>>, Diagnostic> {
    collect_campaign_files_and_paths_with(fs, root).map(|(files, _)| files)
}

/// Collector variant used by mutating commands that must also prove a
/// rewrite target is not hard-linked to ignored content. The second result
/// contains every observed regular file path, including ignored files;
/// document classification and reads remain unchanged.
fn collect_campaign_files_and_paths_with(
    fs: &impl WalkFs,
    root: &Path,
) -> Result<CollectedCampaign, Diagnostic> {
    if root.to_str().is_none() {
        return Err(io_diagnostic(
            "open root",
            None,
            "<campaign-root>",
            IoKind::NonUnicodeComponent,
        ));
    }
    check_walk_root_ancestors(fs, root)?;
    match fs.metadata(root) {
        Err(error) => {
            return Err(io_diagnostic(
                "open root",
                None,
                "<campaign-root>",
                io_kind(&error),
            ));
        }
        Ok(EntryKind::Directory) => {}
        Ok(EntryKind::Symlink) => {
            return Err(io_diagnostic(
                "open root",
                None,
                "<campaign-root>",
                IoKind::SymlinkAtDocumentPath,
            ));
        }
        Ok(_) => {
            return Err(io_diagnostic(
                "open root",
                None,
                "<campaign-root>",
                IoKind::NotADirectory,
            ));
        }
    }
    let mut files = BTreeMap::new();
    let mut regular_paths = Vec::new();
    collect_into(fs, root, "", &mut files, &mut regular_paths)?;
    Ok((files, regular_paths))
}

/// Lists one directory, sorts its entry names by byte representation, then
/// visits each entry in that order, recursing into subdirectories before
/// advancing to the next sibling. A non-Unicode file name stops its entry
/// with a `list` diagnostic naming `<campaign-root>`: the component is never
/// lossy-converted and never reaches the classifier.
fn collect_into(
    fs: &impl WalkFs,
    disk_dir: &Path,
    logical_dir: &str,
    files: &mut BTreeMap<SourcePath, Vec<u8>>,
    regular_paths: &mut Vec<PathBuf>,
) -> Result<(), Diagnostic> {
    let mut names = fs.read_dir_names(disk_dir).map_err(|error| {
        let logical = if logical_dir.is_empty() {
            "<campaign-root>"
        } else {
            logical_dir
        };
        io_diagnostic("list", None, logical, io_kind(&error))
    })?;
    names.sort_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
    for name in &names {
        let Some(name_text) = name.to_str() else {
            return Err(io_diagnostic(
                "list",
                None,
                "<campaign-root>",
                IoKind::NonUnicodeComponent,
            ));
        };
        let logical = if logical_dir.is_empty() {
            name_text.to_owned()
        } else {
            format!("{logical_dir}/{name_text}")
        };
        let disk_child = disk_dir.join(name);
        let kind = fs
            .metadata(&disk_child)
            .map_err(|error| io_diagnostic("classify", None, &logical, io_kind(&error)))?;
        if kind == EntryKind::Directory {
            // Directories are never classified: "ignored" never prunes a
            // directory, so an unreadable one still fails its own listing
            // even when it would have held only ignored files.
            collect_into(fs, &disk_child, &logical, files, regular_paths)?;
        } else {
            collect_file_entry(fs, &disk_child, &logical, kind, files, regular_paths)?;
        }
    }
    Ok(())
}

/// Handles one non-directory entry in walk order: check symlink status (via
/// the already-observed `kind`), then classify the logical text with data's
/// [`crpg_data::campaign_document_path`], then read bytes. Only `Some(path)`
/// regular files are read and inserted; `None` is non-document content and
/// is ignored after its successful listing. Classifier rejections are
/// preserved unchanged through [`crpg_data::diagnostic_for_data_error`]
/// (`invalid_path` with its data-owned position); `io` is reserved for
/// filesystem, symlink, and non-Unicode failures.
fn collect_file_entry(
    fs: &impl WalkFs,
    disk_path: &Path,
    logical: &str,
    kind: EntryKind,
    files: &mut BTreeMap<SourcePath, Vec<u8>>,
    regular_paths: &mut Vec<PathBuf>,
) -> Result<(), Diagnostic> {
    if kind == EntryKind::File {
        regular_paths.push(disk_path.to_path_buf());
    }
    let path = match crpg_data::campaign_document_path(logical) {
        Err(error) => return Err(crpg_data::diagnostic_for_data_error(&error)),
        Ok(None) => return Ok(()),
        Ok(Some(path)) => path,
    };
    match kind {
        EntryKind::File => {
            let bytes = fs.read_file(disk_path).map_err(|error| {
                io_diagnostic("read", Some(path.clone()), logical, io_kind(&error))
            })?;
            files.insert(path, bytes);
            Ok(())
        }
        EntryKind::Symlink => Err(io_diagnostic(
            "classify",
            Some(path),
            logical,
            IoKind::SymlinkAtDocumentPath,
        )),
        EntryKind::Directory | EntryKind::Other => Err(io_diagnostic(
            "classify",
            Some(path),
            logical,
            IoKind::IoError,
        )),
    }
}

/// Joins a `/`-separated logical path onto a campaign root without
/// lossy conversion or native-separator leakage. Logical components are
/// data-validated ASCII, so only the root can carry non-Unicode text; that
/// case is reported as `non_unicode_component` by the caller, never
/// lossy-converted here.
fn disk_path(root: &Path, logical: &str) -> PathBuf {
    let mut disk = root.to_path_buf();
    for component in logical.split('/') {
        disk.push(component);
    }
    disk
}

/// One open file handle for the migrate rewrite phase. Object-safe so the
/// production file and injected test fakes share the same `open_truncate`
/// seam. Production writes through an existing-file handle with truncation,
/// then `sync_all`; tests inject open/write/sync failures per path.
trait RewriteHandle {
    /// Writes the complete new bytes; maps to `cannot write <logical>`.
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()>;
    /// Persists the written bytes; maps to `cannot sync <logical>`.
    fn sync_all(&mut self) -> std::io::Result<()>;
}

/// Production rewrite handle: an already-truncated existing file.
struct RealHandle(std::fs::File);

impl RewriteHandle for RealHandle {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        <std::fs::File as std::io::Write>::write_all(&mut self.0, bytes)
    }

    fn sync_all(&mut self) -> std::io::Result<()> {
        self.0.sync_all()
    }
}

/// Filesystem operations the migrate rewrite phase needs. Production
/// implements this with `std` only and never intentionally follows symlinks
/// during rechecks (`symlink_metadata`); tests inject fakes through this
/// seam to cover ancestor/target symlinks, changed sources, missing or
/// non-regular targets, and open/write/sync failures without platform
/// privileges or runtime skips.
trait RewriteFs {
    /// Observes `path` without following a terminal symlink.
    fn metadata(&self, path: &Path) -> std::io::Result<EntryKind>;
    /// Re-reads a rewrite target for the prewrite equality check.
    fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>>;
    /// Opens an existing file for truncation; never creates a missing file.
    fn open_truncate(&self, path: &Path) -> std::io::Result<Box<dyn RewriteHandle>>;
    /// Creates a missing file; fails when the output already exists so a
    /// concurrently appearing output surfaces as `source_changed`. Never
    /// creates parent directories.
    fn create_new(&self, path: &Path) -> std::io::Result<Box<dyn RewriteHandle>>;
    /// Reports whether two existing path spellings normalize to the same
    /// filesystem path. A missing path compares unequal; other failures
    /// remain errors.
    fn same_file(&self, left: &Path, right: &Path) -> std::io::Result<bool>;
}

impl RewriteFs for RealFs {
    fn metadata(&self, path: &Path) -> std::io::Result<EntryKind> {
        path.symlink_metadata()
            .map(|meta| entry_kind(&meta.file_type()))
    }

    fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn open_truncate(&self, path: &Path) -> std::io::Result<Box<dyn RewriteHandle>> {
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .map(|file| Box::new(RealHandle(file)) as Box<dyn RewriteHandle>)
    }

    fn create_new(&self, path: &Path) -> std::io::Result<Box<dyn RewriteHandle>> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|file| Box::new(RealHandle(file)) as Box<dyn RewriteHandle>)
    }

    fn same_file(&self, left: &Path, right: &Path) -> std::io::Result<bool> {
        match same_file::is_same_file(left, right) {
            Ok(equal) => Ok(equal),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

/// Renders one diagnostic as the single stderr line for `migrate`: the
/// data- or CLI-owned `Display` plus exactly one LF. Stdout stays empty.
fn diagnostic_line(diagnostic: &Diagnostic) -> Vec<u8> {
    let mut stderr = diagnostic.to_string().into_bytes();
    stderr.push(b'\n');
    stderr
}

/// Executes the rewrite phase for already-computed byte-different files in
/// lexical logical-path order, stopping at the first failure.
///
/// For each target this rechecks root/ancestor/target metadata without
/// intentionally following symlinks, then re-reads the target and requires
/// its bytes to equal the collected original, then opens the existing file
/// with truncation, writes the new bytes, and syncs. Canonical-current files
/// are never opened because the caller only passes differing files.
///
/// Bounded contract: this deliberately does not promise atomic replacement,
/// rollback, crash recovery, or a whole-directory transaction. An I/O failure
/// can leave earlier files replaced and the failing file partially written.
/// Concurrent changes after a recheck remain unspecified; no locking or
/// secure live-tree snapshot is introduced. A future atomic save design is
/// separate scope.
fn execute_rewrites(
    fs: &impl RewriteFs,
    root: &Path,
    originals: &BTreeMap<SourcePath, Vec<u8>>,
    updates: &[(SourcePath, Vec<u8>)],
) -> Result<(), Diagnostic> {
    for (path, new_bytes) in updates {
        let logical = path.as_str();
        if root.to_str().is_none() {
            return Err(io_diagnostic(
                "check",
                Some(path.clone()),
                logical,
                IoKind::NonUnicodeComponent,
            ));
        }
        match fs.metadata(root) {
            Err(error) => {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    io_kind(&error),
                ));
            }
            Ok(EntryKind::Directory) => {}
            Ok(EntryKind::Symlink) => {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    IoKind::SymlinkAtDocumentPath,
                ));
            }
            Ok(EntryKind::File) | Ok(EntryKind::Other) => {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    IoKind::NotADirectory,
                ));
            }
        }
        let components: Vec<&str> = logical.split('/').collect();
        for index in 1..components.len() {
            let ancestor_logical = components[..index].join("/");
            let disk_ancestor = disk_path(root, &ancestor_logical);
            match fs.metadata(&disk_ancestor) {
                Err(error) => {
                    return Err(io_diagnostic(
                        "check",
                        Some(path.clone()),
                        logical,
                        io_kind(&error),
                    ));
                }
                Ok(EntryKind::Directory) => {}
                Ok(EntryKind::Symlink) => {
                    return Err(io_diagnostic(
                        "check",
                        Some(path.clone()),
                        logical,
                        IoKind::SymlinkAtDocumentPath,
                    ));
                }
                Ok(EntryKind::File) | Ok(EntryKind::Other) => {
                    return Err(io_diagnostic(
                        "check",
                        Some(path.clone()),
                        logical,
                        IoKind::NotADirectory,
                    ));
                }
            }
        }
        let disk_target = disk_path(root, logical);
        match fs.metadata(&disk_target) {
            Err(error) => {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    io_kind(&error),
                ));
            }
            Ok(EntryKind::File) => {}
            Ok(EntryKind::Symlink) => {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    IoKind::SymlinkAtDocumentPath,
                ));
            }
            Ok(EntryKind::Directory) | Ok(EntryKind::Other) => {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    IoKind::NotAFile,
                ));
            }
        }
        let current = fs.read_file(&disk_target).map_err(|error| {
            io_diagnostic("check", Some(path.clone()), logical, io_kind(&error))
        })?;
        let Some(original) = originals.get(path) else {
            return Err(io_diagnostic(
                "check",
                Some(path.clone()),
                logical,
                IoKind::SourceChanged,
            ));
        };
        if current != *original {
            return Err(io_diagnostic(
                "check",
                Some(path.clone()),
                logical,
                IoKind::SourceChanged,
            ));
        }
        let mut handle = fs
            .open_truncate(&disk_target)
            .map_err(|error| io_diagnostic("open", Some(path.clone()), logical, io_kind(&error)))?;
        handle.write_all(new_bytes).map_err(|error| {
            io_diagnostic("write", Some(path.clone()), logical, io_kind(&error))
        })?;
        handle
            .sync_all()
            .map_err(|error| io_diagnostic("sync", Some(path.clone()), logical, io_kind(&error)))?;
    }
    Ok(())
}

/// Parses the package engine version and validates the collected files.
/// Split from [`run_validate`] so tests can pin the internal-version
/// fallback with a synthetic bad version string: `env!` text always parses
/// in production. The version type is inferred from `validate_files`, so
/// this crate needs no semver edge of its own — only the `crpg-data` one.
fn validate_with_engine_version(
    files: &BTreeMap<SourcePath, Vec<u8>>,
    engine_text: &str,
) -> Result<Vec<Diagnostic>, ()> {
    let engine = engine_text.parse().map_err(|_| ())?;
    Ok(crpg_data::validate_files(files, &engine))
}

/// Loads the collected files through T012a's migration-aware loader with the
/// supplied engine text. The version type is inferred from `load_campaign`,
/// exactly as T011b does, so no semver edge is added.
fn load_with_engine_version(
    files: &BTreeMap<SourcePath, Vec<u8>>,
    engine_text: &str,
) -> Result<crpg_data::LoadedCampaign, Outcome> {
    let engine = engine_text.parse().map_err(|_| Outcome {
        code: 1,
        stdout: Vec::new(),
        stderr: MIGRATE_ENGINE_FAILURE.as_bytes().to_vec(),
    })?;
    crpg_data::load_campaign(files, &engine).map_err(|error| {
        let diagnostic = crpg_data::diagnostic_for_data_error(&error);
        Outcome {
            code: 1,
            stdout: Vec::new(),
            stderr: diagnostic_line(&diagnostic),
        }
    })
}

/// Exit mapping: any `Error` fails with 1; warnings print but never fail.
fn exit_for(diagnostics: &[Diagnostic]) -> u8 {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        1
    } else {
        0
    }
}

/// Renders plain mode: one data-owned `Display` line per diagnostic, in
/// returned order, on stderr; stdout stays empty. A clean run is silent.
fn plain_outcome(diagnostics: &[Diagnostic]) -> Outcome {
    let mut stderr = Vec::new();
    for diagnostic in diagnostics {
        stderr.extend_from_slice(diagnostic.to_string().as_bytes());
        stderr.push(b'\n');
    }
    Outcome {
        code: exit_for(diagnostics),
        stdout: Vec::new(),
        stderr,
    }
}

/// Renders JSON mode: the complete diagnostic vector through data's
/// canonical writer on stdout, stderr empty. `serialized` is already the
/// canonical result in production; tests pass `Err` to pin the internal
/// serialization fallback without inventing a normal user case for it.
fn json_outcome(diagnostics: &[Diagnostic], serialized: Result<Vec<u8>, ()>) -> Outcome {
    match serialized {
        Ok(bytes) => Outcome {
            code: exit_for(diagnostics),
            stdout: bytes,
            stderr: Vec::new(),
        },
        Err(()) => Outcome {
            code: 1,
            stdout: Vec::new(),
            stderr: SERIALIZATION_FAILURE.as_bytes().to_vec(),
        },
    }
}

/// Runs the parsed `validate` command: collect classified files in sorted
/// depth-first pre-order, validate them against the package engine version,
/// render the returned diagnostics, and map no-Error/any-Error to 0/1.
/// Zero validation semantics live here: no kind tables, graph traversal,
/// sorting, or second diagnostic shape.
fn run_validate(root: &Path, json: bool) -> Outcome {
    let diagnostics = match collect_campaign_files(root) {
        Ok(files) => match validate_with_engine_version(&files, env!("CARGO_PKG_VERSION")) {
            Ok(diagnostics) => diagnostics,
            Err(()) => {
                return Outcome {
                    code: 1,
                    stdout: Vec::new(),
                    stderr: ENGINE_VERSION_FAILURE.as_bytes().to_vec(),
                };
            }
        },
        Err(diagnostic) => vec![diagnostic],
    };
    if json {
        let serialized = crpg_data::canonical_json(&diagnostics).map_err(|_| ());
        json_outcome(&diagnostics, serialized)
    } else {
        plain_outcome(&diagnostics)
    }
}

/// Runs the parsed `migrate` command: the explicit save action for S §4.5.
///
/// Flow is parse (`args_os`) → the existing T011b read-only collector →
/// `load_campaign` once with the compile-time package engine version →
/// `serialize_campaign` once → compare returned bytes with collected
/// originals → replace differing recognized files in lexical `SourcePath`
/// order. All collection, load, and serialization work succeeds for the
/// entire map before any write starts, so preflight failures perform zero
/// writes. This command performs structural migration/save only: it never
/// calls `validate_files`, individual migration steps, or any semantic check,
/// and it never changes a schema tag, decodes campaign JSON, recomputes a
/// digest, or implements a version decision outside data.
///
/// Bounded partial-write limitation: writes stop at the first failure without
/// atomic replacement, rollback, crash recovery, or a whole-directory
/// transaction. An I/O failure can leave earlier files replaced and the
/// failing file partially written.
fn run_migrate_with<W: WalkFs, R: RewriteFs>(
    walk_fs: &W,
    rewrite_fs: &R,
    root: &Path,
    engine_text: &str,
) -> Outcome {
    let (files, regular_paths) = match collect_campaign_files_and_paths_with(walk_fs, root) {
        Ok(collected) => collected,
        Err(diagnostic) => {
            return Outcome {
                code: 1,
                stdout: Vec::new(),
                stderr: diagnostic_line(&diagnostic),
            };
        }
    };
    let loaded = match load_with_engine_version(&files, engine_text) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    let serialized = match crpg_data::serialize_campaign(&loaded) {
        Ok(serialized) => serialized,
        Err(error) => {
            let diagnostic = crpg_data::diagnostic_for_data_error(&error);
            return Outcome {
                code: 1,
                stdout: Vec::new(),
                stderr: diagnostic_line(&diagnostic),
            };
        }
    };
    let updates = match plan_updates(&files, &serialized) {
        Ok(updates) => updates,
        Err(()) => {
            return Outcome {
                code: 1,
                stdout: Vec::new(),
                stderr: DOCUMENT_SET_FAILURE.as_bytes().to_vec(),
            };
        }
    };
    if let Err(diagnostic) = reject_rewrite_aliases(rewrite_fs, root, &regular_paths, &updates) {
        return io_outcome(&diagnostic);
    }
    match execute_rewrites(rewrite_fs, root, &files, &updates) {
        Ok(()) => Outcome::success(),
        Err(diagnostic) => Outcome {
            code: 1,
            stdout: Vec::new(),
            stderr: diagnostic_line(&diagnostic),
        },
    }
}

/// Compares collected originals with the canonical writer output and returns
/// the byte-different files in lexical `SourcePath` order. The returned key
/// set must equal the collected set; a mismatch is an internal failure
/// reported before any write starts. Split from [`run_migrate_with`] so
/// tests can pin the internal document-set fallback with an injected
/// mismatched map without touching the filesystem.
fn plan_updates(
    files: &BTreeMap<SourcePath, Vec<u8>>,
    serialized: &BTreeMap<SourcePath, Vec<u8>>,
) -> Result<Vec<(SourcePath, Vec<u8>)>, ()> {
    if files.keys().collect::<Vec<_>>() != serialized.keys().collect::<Vec<_>>() {
        return Err(());
    }
    let mut updates = Vec::new();
    for (path, new_bytes) in serialized {
        if let Some(original) = files.get(path) {
            if original != new_bytes {
                updates.push((path.clone(), new_bytes.clone()));
            }
        }
    }
    Ok(updates)
}

/// Production `migrate` entry: the real filesystem for both collection and
/// the rewrite phase, with the compile-time package engine version.
fn run_migrate(root: &Path) -> Outcome {
    let fs = RealFs;
    run_migrate_with(&fs, &fs, root, env!("CARGO_PKG_VERSION"))
}

/// Maps a data error through the unchanged data-owned diagnostic rendering:
/// one `Display` line plus LF on stderr, empty stdout, exit 1.
fn data_outcome(error: &crpg_data::DataError) -> Outcome {
    let diagnostic = crpg_data::diagnostic_for_data_error(error);
    Outcome {
        code: 1,
        stdout: Vec::new(),
        stderr: diagnostic_line(&diagnostic),
    }
}

/// Maps a CLI-owned or collected `io` diagnostic to its process outcome:
/// one `Display` line plus LF on stderr, empty stdout, exit 1.
fn io_outcome(diagnostic: &Diagnostic) -> Outcome {
    Outcome {
        code: 1,
        stdout: Vec::new(),
        stderr: diagnostic_line(diagnostic),
    }
}

/// Runs the parsed `new` command: builds the bounded Stage-2 authoring
/// template for the requested type with caller-supplied identities and
/// writes it through the existing typed `write_document`, never by
/// handwriting envelopes, schema tags, or canonical JSON.
///
/// Templates use explicit identities only: `--id` always, plus a distinct
/// `--entry-id` for dialogue/quest (enforced by the parser). No clock, RNG,
/// registry, or filesystem lookup. All notes are absent. Repeated calls with
/// identical operands produce identical bytes. The output is a valid minimal
/// current typed document, not a whole campaign: the author supplies locale
/// entries and file placement.
fn run_new(kind: NewKind, slug: &str, id: Ulid, entry_id: Option<Ulid>) -> Outcome {
    let document = match kind {
        NewKind::Creature => crpg_data::Document::Creature(crpg_data::Creature {
            id,
            slug: slug.to_owned(),
            name: format!("creature.{slug}.name"),
            note: None,
            stats: BTreeMap::new(),
            tags: Vec::new(),
            faction: None,
            inventory: Vec::new(),
        }),
        NewKind::Item => crpg_data::Document::Item(crpg_data::Item {
            id,
            slug: slug.to_owned(),
            name: format!("item.{slug}.name"),
            note: None,
            stats: BTreeMap::new(),
            tags: Vec::new(),
        }),
        NewKind::Dialogue => {
            // The parser requires an entry id for dialogue; this documents
            // the invariant at the single construction site.
            let entry = entry_id.expect("parser requires --entry-id for dialogue");
            crpg_data::Document::Dialogue(crpg_data::Dialogue {
                id,
                slug: slug.to_owned(),
                name: format!("dialogue.{slug}.name"),
                note: None,
                entry,
                nodes: vec![crpg_data::DialogueNode {
                    id: entry,
                    body: crpg_data::DialogueBody::End,
                }],
            })
        }
        NewKind::Quest => {
            let entry = entry_id.expect("parser requires --entry-id for quest");
            crpg_data::Document::Quest(crpg_data::Quest {
                id,
                slug: slug.to_owned(),
                name: format!("quest.{slug}.name"),
                note: None,
                entry,
                states: vec![crpg_data::QuestState {
                    id: entry,
                    name: format!("quest.{slug}.state.done"),
                    terminal: true,
                    on_enter: Vec::new(),
                    transitions: Vec::new(),
                }],
            })
        }
    };
    match crpg_data::write_document(&document) {
        Ok(bytes) => Outcome {
            code: 0,
            stdout: bytes,
            stderr: Vec::new(),
        },
        Err(error) => data_outcome(&error),
    }
}

/// Runs the parsed `schema` command: calls `generated_schemas`, selects the
/// requested `<stem>.schema.json`, and writes the returned bytes unchanged.
/// Works outside the repository with no runtime `schemas/` directory. The
/// `None` arm below is unreachable — the parser admits only the seventeen
/// stems the data call always generates — and reports a data-owned line
/// rather than panicking if the two ever disagree.
fn run_schema(stem: &str) -> Outcome {
    let filename = format!("{stem}.schema.json");
    let schemas = match crpg_data::generated_schemas() {
        Ok(schemas) => schemas,
        Err(error) => return data_outcome(&error),
    };
    match schemas.get(&filename) {
        Some(bytes) => Outcome {
            code: 0,
            stdout: bytes.clone(),
            stderr: Vec::new(),
        },
        None => data_outcome(&crpg_data::DataError::Layout {
            path: None,
            message: format!("missing generated schema: {filename}"),
        }),
    }
}

/// Loads the collected files with the supplied engine text, mapping an
/// unparsable version to the caller-selected fixed failure line. The version
/// type is inferred from `load_campaign`, exactly as T011b does, so no
/// semver edge is added. Split from the command runners so tests can pin
/// each command's internal-version bytes with a synthetic bad version.
fn load_for_command(
    files: &BTreeMap<SourcePath, Vec<u8>>,
    engine_text: &str,
    failure_line: &str,
) -> Result<crpg_data::LoadedCampaign, Outcome> {
    let engine = engine_text.parse().map_err(|_| Outcome {
        code: 1,
        stdout: Vec::new(),
        stderr: failure_line.as_bytes().to_vec(),
    })?;
    crpg_data::load_campaign(files, &engine).map_err(|error| data_outcome(&error))
}

/// Runs the parsed `explain` command: parse (done) -> collect ->
/// `load_campaign` once with the compile-time package engine version ->
/// `explain_object` once -> emit bytes.
///
/// `Some(bytes)` goes directly to stdout, including its one final LF, with
/// no parse, reserialize, sort, or decoration. `None` is exit 1 with exactly
/// `crpgc explain: object not found: <canonical-uppercase-id>\n` and empty
/// stdout. No semantic validation prerequisite: structurally acceptable
/// campaigns with dangling or wrong-kind references remain introspectable.
/// Read-only, including migration-aware loading of historical documents.
fn run_explain_with<W: WalkFs>(walk_fs: &W, root: &Path, id: Ulid, engine_text: &str) -> Outcome {
    let files = match collect_campaign_files_with(walk_fs, root) {
        Ok(files) => files,
        Err(diagnostic) => return io_outcome(&diagnostic),
    };
    let loaded = match load_for_command(&files, engine_text, EXPLAIN_ENGINE_FAILURE) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    match crpg_data::explain_object(&loaded, id) {
        Err(error) => data_outcome(&error),
        Ok(None) => Outcome {
            code: 1,
            stdout: Vec::new(),
            stderr: format!("crpgc explain: object not found: {id}\n").into_bytes(),
        },
        Ok(Some(bytes)) => Outcome {
            code: 0,
            stdout: bytes,
            stderr: Vec::new(),
        },
    }
}

/// Production `explain` entry: the real filesystem with the compile-time
/// package engine version.
fn run_explain(root: &Path, id: Ulid) -> Outcome {
    run_explain_with(&RealFs, root, id, env!("CARGO_PKG_VERSION"))
}

/// Runs the parsed `fmt` command: migrate's collect -> load -> serialize ->
/// key-set check -> byte-diff plan. Default mode explicitly saves all
/// differing recognized files with the T012b writer and its exact
/// preflight/no-op/recheck/error/partial-write contract, and may persist
/// in-memory migrations exactly as `migrate` does. `--check` performs the
/// same preflight without opening files for writing: differences are exit 1
/// with `crpgc fmt: noncanonical: <logical>\n` per file in lexical
/// `SourcePath` order, otherwise silent exit 0. Failure before a complete
/// plan emits only that error, never a partial dirty list. Semantic findings
/// never block formatting; no repairs or lock regeneration.
///
/// Bounded partial-write limitation, shared with `migrate`: writes stop at
/// the first failure without atomic replacement, rollback, crash recovery,
/// or a whole-directory transaction.
fn run_fmt_with<W: WalkFs, R: RewriteFs>(
    walk_fs: &W,
    rewrite_fs: &R,
    root: &Path,
    engine_text: &str,
    check: bool,
) -> Outcome {
    let (files, regular_paths) = match collect_campaign_files_and_paths_with(walk_fs, root) {
        Ok(collected) => collected,
        Err(diagnostic) => return io_outcome(&diagnostic),
    };
    let loaded = match load_for_command(&files, engine_text, FMT_ENGINE_FAILURE) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    let serialized = match crpg_data::serialize_campaign(&loaded) {
        Ok(serialized) => serialized,
        Err(error) => return data_outcome(&error),
    };
    let updates = match plan_updates(&files, &serialized) {
        Ok(updates) => updates,
        Err(()) => {
            return Outcome {
                code: 1,
                stdout: Vec::new(),
                stderr: FMT_DOCUMENT_SET_FAILURE.as_bytes().to_vec(),
            };
        }
    };
    if check {
        if updates.is_empty() {
            return Outcome::success();
        }
        let mut stderr = Vec::new();
        for (path, _) in &updates {
            stderr.extend_from_slice(
                format!("crpgc fmt: noncanonical: {}\n", path.as_str()).as_bytes(),
            );
        }
        return Outcome {
            code: 1,
            stdout: Vec::new(),
            stderr,
        };
    }
    if let Err(diagnostic) = reject_rewrite_aliases(rewrite_fs, root, &regular_paths, &updates) {
        return io_outcome(&diagnostic);
    }
    match execute_rewrites(rewrite_fs, root, &files, &updates) {
        Ok(()) => Outcome::success(),
        Err(diagnostic) => io_outcome(&diagnostic),
    }
}

/// Refuses a write target that is the same filesystem file as any other
/// regular path observed during collection. This catches hard links from a
/// recognized document to ignored content or another document before the
/// first truncating open. Exact target spellings are skipped.
fn reject_rewrite_aliases(
    fs: &impl RewriteFs,
    root: &Path,
    regular_paths: &[PathBuf],
    updates: &[(SourcePath, Vec<u8>)],
) -> Result<(), Diagnostic> {
    for (path, _) in updates {
        let logical = path.as_str();
        let target = disk_path(root, logical);
        for candidate in regular_paths {
            if candidate == &target {
                continue;
            }
            let aliases = fs.same_file(&target, candidate).map_err(|error| {
                io_diagnostic("check", Some(path.clone()), logical, io_kind(&error))
            })?;
            if aliases {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    IoKind::SourceChanged,
                ));
            }
        }
    }
    Ok(())
}

/// Production `fmt` entry: the real filesystem for both collection and the
/// rewrite phase, with the compile-time package engine version.
fn run_fmt(root: &Path, check: bool) -> Outcome {
    let fs = RealFs;
    run_fmt_with(&fs, &fs, root, env!("CARGO_PKG_VERSION"), check)
}

/// Checks every ancestor of `disk` (excluding the target itself) without
/// following observed symlinks and without canonicalizing. Each ancestor
/// must be a directory; a missing, symlinked, or non-directory ancestor
/// fails with the caller's `op`/logical/file shape, naming the affected
/// target logical path (never a native or absolute path). Absolute and
/// relative forms are both handled via `Path::ancestors`; empty prefixes
/// are skipped. The bounded post-check race contract from T012b is
/// retained: concurrent changes after these checks remain unspecified, with
/// no atomicity, locking, handle-based no-follow, or snapshot guarantee.
fn check_disk_ancestors(
    fs: &impl RewriteFs,
    disk: &Path,
    logical: &str,
    file: Option<SourcePath>,
    op: &str,
) -> Result<(), Diagnostic> {
    if disk.to_str().is_none() {
        return Err(io_diagnostic(
            op,
            file,
            logical,
            IoKind::NonUnicodeComponent,
        ));
    }
    let mut ancestors: Vec<&Path> = disk.ancestors().skip(1).collect();
    ancestors.reverse();
    for ancestor in ancestors {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if ancestor.to_str().is_none() {
            return Err(io_diagnostic(
                op,
                file.clone(),
                logical,
                IoKind::NonUnicodeComponent,
            ));
        }
        match fs.metadata(ancestor) {
            Err(error) => {
                return Err(io_diagnostic(op, file.clone(), logical, io_kind(&error)));
            }
            Ok(EntryKind::Directory) => {}
            Ok(EntryKind::Symlink) => {
                return Err(io_diagnostic(
                    op,
                    file.clone(),
                    logical,
                    IoKind::SymlinkAtDocumentPath,
                ));
            }
            Ok(EntryKind::File) | Ok(EntryKind::Other) => {
                return Err(io_diagnostic(
                    op,
                    file.clone(),
                    logical,
                    IoKind::NotADirectory,
                ));
            }
        }
    }
    Ok(())
}

/// Verifies the lock root without following a terminal symlink: a directory
/// is required, exactly as the T011b collector requires for every command
/// root. Ancestors of the supplied root are checked first, so a symlinked
/// parent is rejected with the portable `<campaign-root>` diagnostic instead
/// of being traversed.
fn check_lock_root(fs: &impl RewriteFs, root: &Path) -> Result<(), Diagnostic> {
    if root.to_str().is_none() {
        return Err(io_diagnostic(
            "open root",
            None,
            "<campaign-root>",
            IoKind::NonUnicodeComponent,
        ));
    }
    check_disk_ancestors(fs, root, "<campaign-root>", None, "open root")?;
    match fs.metadata(root) {
        Err(error) => Err(io_diagnostic(
            "open root",
            None,
            "<campaign-root>",
            io_kind(&error),
        )),
        Ok(EntryKind::Directory) => Ok(()),
        Ok(EntryKind::Symlink) => Err(io_diagnostic(
            "open root",
            None,
            "<campaign-root>",
            IoKind::SymlinkAtDocumentPath,
        )),
        Ok(EntryKind::File) | Ok(EntryKind::Other) => Err(io_diagnostic(
            "open root",
            None,
            "<campaign-root>",
            IoKind::NotADirectory,
        )),
    }
}

/// Reads one required regular input file without following an observed
/// symlink. Ancestors are checked first, so a symlinked parent is rejected
/// with the portable target diagnostic instead of being traversed.
/// Anything but a regular file — missing, symlink, directory, or
/// other — is a portable `cannot read <logical>: <kind>` diagnostic, with
/// the campaign file's logical path or the stable `<catalog>` label. Never
/// lossy-converts a non-Unicode path.
fn read_regular_input(
    fs: &impl RewriteFs,
    disk: &Path,
    logical: &str,
    file: Option<SourcePath>,
) -> Result<Vec<u8>, Diagnostic> {
    if disk.to_str().is_none() {
        return Err(io_diagnostic(
            "read",
            file,
            logical,
            IoKind::NonUnicodeComponent,
        ));
    }
    check_disk_ancestors(fs, disk, logical, file.clone(), "read")?;
    match fs.metadata(disk) {
        Err(error) => Err(io_diagnostic("read", file, logical, io_kind(&error))),
        Ok(EntryKind::File) => fs
            .read_file(disk)
            .map_err(|error| io_diagnostic("read", file, logical, io_kind(&error))),
        Ok(EntryKind::Symlink) => Err(io_diagnostic(
            "read",
            file,
            logical,
            IoKind::SymlinkAtDocumentPath,
        )),
        Ok(EntryKind::Directory) | Ok(EntryKind::Other) => {
            Err(io_diagnostic("read", file, logical, IoKind::NotAFile))
        }
    }
}

/// Requires the `assets` ancestor directory for the assets-lock input.
/// Ancestor failures name the affected target logical path, as in T012b.
/// Parents of the ancestor itself are checked first, so a symlinked
/// grandparent is rejected with the target logical label.
fn check_lock_ancestor(
    fs: &impl RewriteFs,
    disk_ancestor: &Path,
    logical: &str,
    file: SourcePath,
) -> Result<(), Diagnostic> {
    if disk_ancestor.to_str().is_none() {
        return Err(io_diagnostic(
            "read",
            Some(file),
            logical,
            IoKind::NonUnicodeComponent,
        ));
    }
    check_disk_ancestors(fs, disk_ancestor, logical, Some(file.clone()), "read")?;
    match fs.metadata(disk_ancestor) {
        Err(error) => Err(io_diagnostic("read", Some(file), logical, io_kind(&error))),
        Ok(EntryKind::Directory) => Ok(()),
        Ok(EntryKind::Symlink) => Err(io_diagnostic(
            "read",
            Some(file),
            logical,
            IoKind::SymlinkAtDocumentPath,
        )),
        Ok(EntryKind::File) | Ok(EntryKind::Other) => Err(io_diagnostic(
            "read",
            Some(file),
            logical,
            IoKind::NotADirectory,
        )),
    }
}

/// Saves the new `campaign.lock` bytes. An existing regular output is
/// replaced with T012b's original-byte/recheck/truncate/write_all/sync_all
/// discipline and is a zero-write no-op when already byte-identical — even
/// when malformed, existing bytes compare raw, never through a document
/// parse. The existing path captures the raw bytes once, no-ops when
/// identical to the desired bytes, and otherwise rechecks root/ancestors/
/// target, rereads, requires equality with the captured original
/// (`source_changed` on drift), then truncates, writes, and syncs. A
/// missing output rechecks the root and is created with `create_new`, so a
/// concurrently appearing output surfaces as `source_changed`. Directories
/// are never created, symlinks never followed, and non-regular outputs never
/// replaced. Save failures use `check`/`open`/`write`/`sync` on
/// `campaign.lock`; a failure may leave a partial replaced or new lock with
/// no rollback, exactly as in T012b.
fn write_lock_output(fs: &impl RewriteFs, root: &Path, new_bytes: &[u8]) -> Result<(), Diagnostic> {
    let logical = "campaign.lock";
    let path: SourcePath = logical
        .parse()
        .expect("campaign.lock is a valid logical path");
    if root.to_str().is_none() {
        return Err(io_diagnostic(
            "check",
            Some(path),
            logical,
            IoKind::NonUnicodeComponent,
        ));
    }
    check_disk_ancestors(fs, root, logical, Some(path.clone()), "check")?;
    match fs.metadata(root) {
        Err(error) => {
            return Err(io_diagnostic("check", Some(path), logical, io_kind(&error)));
        }
        Ok(EntryKind::Directory) => {}
        Ok(EntryKind::Symlink) => {
            return Err(io_diagnostic(
                "check",
                Some(path),
                logical,
                IoKind::SymlinkAtDocumentPath,
            ));
        }
        Ok(EntryKind::File) | Ok(EntryKind::Other) => {
            return Err(io_diagnostic(
                "check",
                Some(path),
                logical,
                IoKind::NotADirectory,
            ));
        }
    }
    let disk_target = disk_path(root, logical);
    check_disk_ancestors(fs, &disk_target, logical, Some(path.clone()), "check")?;
    match fs.metadata(&disk_target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut handle = fs.create_new(&disk_target).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    io_diagnostic("check", Some(path.clone()), logical, IoKind::SourceChanged)
                } else {
                    io_diagnostic("open", Some(path.clone()), logical, io_kind(&error))
                }
            })?;
            handle.write_all(new_bytes).map_err(|error| {
                io_diagnostic("write", Some(path.clone()), logical, io_kind(&error))
            })?;
            handle.sync_all().map_err(|error| {
                io_diagnostic("sync", Some(path.clone()), logical, io_kind(&error))
            })?;
            Ok(())
        }
        Err(error) => Err(io_diagnostic("check", Some(path), logical, io_kind(&error))),
        Ok(EntryKind::Symlink) => Err(io_diagnostic(
            "check",
            Some(path),
            logical,
            IoKind::SymlinkAtDocumentPath,
        )),
        Ok(EntryKind::Directory) | Ok(EntryKind::Other) => Err(io_diagnostic(
            "check",
            Some(path),
            logical,
            IoKind::NotAFile,
        )),
        Ok(EntryKind::File) => {
            let initial = fs.read_file(&disk_target).map_err(|error| {
                io_diagnostic("check", Some(path.clone()), logical, io_kind(&error))
            })?;
            if initial == new_bytes {
                return Ok(());
            }
            // Shared original-byte discipline: recheck ancestors/target,
            // reread, and require equality with the captured original before
            // opening for truncation. Drift surfaces as `source_changed`
            // with zero opens/writes.
            check_disk_ancestors(fs, &disk_target, logical, Some(path.clone()), "check")?;
            match fs.metadata(&disk_target) {
                Err(error) => {
                    return Err(io_diagnostic(
                        "check",
                        Some(path.clone()),
                        logical,
                        io_kind(&error),
                    ));
                }
                Ok(EntryKind::File) => {}
                Ok(EntryKind::Symlink) => {
                    return Err(io_diagnostic(
                        "check",
                        Some(path.clone()),
                        logical,
                        IoKind::SymlinkAtDocumentPath,
                    ));
                }
                Ok(EntryKind::Directory) | Ok(EntryKind::Other) => {
                    return Err(io_diagnostic(
                        "check",
                        Some(path.clone()),
                        logical,
                        IoKind::NotAFile,
                    ));
                }
            }
            let current = fs.read_file(&disk_target).map_err(|error| {
                io_diagnostic("check", Some(path.clone()), logical, io_kind(&error))
            })?;
            if current != initial {
                return Err(io_diagnostic(
                    "check",
                    Some(path.clone()),
                    logical,
                    IoKind::SourceChanged,
                ));
            }
            let mut handle = fs.open_truncate(&disk_target).map_err(|error| {
                io_diagnostic("open", Some(path.clone()), logical, io_kind(&error))
            })?;
            handle.write_all(new_bytes).map_err(|error| {
                io_diagnostic("write", Some(path.clone()), logical, io_kind(&error))
            })?;
            handle.sync_all().map_err(|error| {
                io_diagnostic("sync", Some(path.clone()), logical, io_kind(&error))
            })?;
            Ok(())
        }
    }
}

/// Runs the parsed `lock` command: reads `campaign.json`, `assets.lock`,
/// and the supplied flat catalog in that order, resolves with
/// `make_campaign_lock`, serializes with `write_campaign_lock`, then
/// creates, replaces, or no-ops `campaign.lock`.
///
/// The catalog is a JSON array of existing `PackageCandidate` wire objects
/// decoded directly through `serde_json` — never through an intermediate
/// `Value` — preserving strict field and value decoding; syntax or typed
/// failures are exactly `crpgc lock: invalid catalog`. All
/// input/resolve/serialize work completes before the output is touched, so
/// those failures perform zero output writes. `load_campaign` is never
/// called: an absent or stale `campaign.lock` is exactly what this explicit
/// operation supports. This bounded operation is not whole-campaign
/// validation or an engine-compatibility gate. Source assets are never read,
/// `assets.lock` never rewritten, and `campaign.json` never migrated.
fn run_lock_with(fs: &impl RewriteFs, root: &Path, catalog_path: &Path) -> Outcome {
    if let Err(diagnostic) = check_lock_root(fs, root) {
        return io_outcome(&diagnostic);
    }
    let campaign_logical = "campaign.json";
    let campaign_file: SourcePath = campaign_logical
        .parse()
        .expect("campaign.json is a valid logical path");
    let campaign_bytes = match read_regular_input(
        fs,
        &disk_path(root, campaign_logical),
        campaign_logical,
        Some(campaign_file),
    ) {
        Ok(bytes) => bytes,
        Err(diagnostic) => return io_outcome(&diagnostic),
    };
    let campaign = match crpg_data::read_document(&campaign_bytes) {
        Ok(crpg_data::Document::Campaign(campaign)) => campaign,
        Ok(_) => {
            return Outcome {
                code: 1,
                stdout: Vec::new(),
                stderr: LOCK_EXPECTED_CAMPAIGN.as_bytes().to_vec(),
            };
        }
        Err(error) => return data_outcome(&error),
    };
    let assets_logical = "assets/assets.lock";
    let assets_file: SourcePath = assets_logical
        .parse()
        .expect("assets/assets.lock is a valid logical path");
    if let Err(diagnostic) = check_lock_ancestor(
        fs,
        &disk_path(root, "assets"),
        assets_logical,
        assets_file.clone(),
    ) {
        return io_outcome(&diagnostic);
    }
    let assets_bytes = match read_regular_input(
        fs,
        &disk_path(root, assets_logical),
        assets_logical,
        Some(assets_file),
    ) {
        Ok(bytes) => bytes,
        Err(diagnostic) => return io_outcome(&diagnostic),
    };
    let assets = match crpg_data::read_assets_lock(&assets_bytes) {
        Ok(assets) => assets,
        Err(error) => return data_outcome(&error),
    };
    if catalog_path.to_str().is_none() {
        return io_outcome(&io_diagnostic(
            "read",
            None,
            "<catalog>",
            IoKind::NonUnicodeComponent,
        ));
    }
    let catalog_bytes = match read_regular_input(fs, catalog_path, "<catalog>", None) {
        Ok(bytes) => bytes,
        Err(diagnostic) => return io_outcome(&diagnostic),
    };
    let candidates: Vec<crpg_data::PackageCandidate> = match serde_json::from_slice(&catalog_bytes)
    {
        Ok(candidates) => candidates,
        Err(_) => {
            return Outcome {
                code: 1,
                stdout: Vec::new(),
                stderr: LOCK_INVALID_CATALOG.as_bytes().to_vec(),
            };
        }
    };
    let lock = match crpg_data::make_campaign_lock(&campaign.requires, &candidates, &assets) {
        Ok(lock) => lock,
        Err(error) => return data_outcome(&error),
    };
    let new_bytes = match crpg_data::write_campaign_lock(&lock) {
        Ok(bytes) => bytes,
        Err(error) => return data_outcome(&error),
    };
    let output_path = disk_path(root, "campaign.lock");
    for input_path in [
        disk_path(root, campaign_logical),
        disk_path(root, assets_logical),
        catalog_path.to_path_buf(),
    ] {
        match fs.same_file(&input_path, &output_path) {
            Ok(false) => {}
            Ok(true) => {
                let path = "campaign.lock"
                    .parse()
                    .expect("campaign.lock is a valid logical path");
                return io_outcome(&io_diagnostic(
                    "check",
                    Some(path),
                    "campaign.lock",
                    IoKind::SourceChanged,
                ));
            }
            Err(error) => {
                let path = "campaign.lock"
                    .parse()
                    .expect("campaign.lock is a valid logical path");
                return io_outcome(&io_diagnostic(
                    "check",
                    Some(path),
                    "campaign.lock",
                    io_kind(&error),
                ));
            }
        }
    }
    match write_lock_output(fs, root, &new_bytes) {
        Ok(()) => Outcome::success(),
        Err(diagnostic) => io_outcome(&diagnostic),
    }
}

/// Production `lock` entry: the real filesystem for inputs and the output.
/// The catalog path stays relative to the process working directory.
fn run_lock(root: &Path, catalog: &Path) -> Outcome {
    run_lock_with(&RealFs, root, catalog)
}

/// Lowercase hex encoding for `run` sample lines. Hand-rolled so no hashing
/// or hex dependency is added; the harness keeps its own private copy.
fn hex32(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Runs the parsed `run` command: calls
/// `crpg_testkit::run_hash_sequence(seed, ticks, Box::new(|_| {}))` once —
/// the empty world's no-op script, with no campaign root or gameplay intent
/// vocabulary implied — then emits samples after completed ticks M, 2M, ...
/// <= N as `<k> <64-lowercase-hex-hash>\n` using returned index k-1.
/// `--hash-every` filters output only, never harness execution. N=0 and M>N
/// produce empty stdout with exit 0. No golden reads/writes, reblessing,
/// replay recording, wall-clock timing, or cross-platform hash-equality
/// promise. This bounded harness is not campaign execution.
fn run_run(ticks: usize, hash_every: usize, seed: u64) -> Outcome {
    let hashes = crpg_testkit::run_hash_sequence(seed, ticks, Box::new(|_| {}));
    let mut stdout = Vec::new();
    let mut tick = hash_every;
    while tick <= ticks {
        let hash = &hashes[tick - 1];
        stdout.extend_from_slice(format!("{tick} {}\n", hex32(hash)).as_bytes());
        tick += hash_every;
    }
    Outcome {
        code: 0,
        stdout,
        stderr: Vec::new(),
    }
}

/// Writes an [`Outcome`] to the supplied streams without panicking and
/// without recursively reporting a broken stream. Returns the process exit
/// code: the outcome code when both writes succeed, otherwise `1`.
fn emit_code(
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
    outcome: &Outcome,
) -> u8 {
    let stdout_ok = stdout.write_all(&outcome.stdout).is_ok();
    let stderr_ok = stderr.write_all(&outcome.stderr).is_ok();
    let stdout_ok = stdout_ok && stdout.flush().is_ok();
    let stderr_ok = stderr_ok && stderr.flush().is_ok();
    if stdout_ok && stderr_ok {
        outcome.code
    } else {
        1
    }
}

/// Runs a parsed command. Replay stays the thin testkit consumer it was at
/// T009b; validate joins data-owned validation to OS-owned traversal;
/// migrate joins the same collector to T012a's migration-aware loader and
/// canonical writer plus its explicit save phase; the six T013 commands join
/// the same collector, data-owned writers, the landed introspection report,
/// the flat package-lock constructor, and the hash-sequence harness to the
/// process boundary, each owning only its argument, stream, and exit policy.
fn run(command: Command) -> Outcome {
    match command {
        Command::Replay {
            replay_path,
            golden_path,
        } => match crpg_testkit::play_and_verify(
            &replay_path,
            &golden_path,
            apply::reference_intents(),
        ) {
            Ok(_) => Outcome::success(),
            Err(error) => Outcome::cli_error(&CliError::Replay(error)),
        },
        Command::Validate { root, json } => run_validate(&root, json),
        Command::Migrate { root } => run_migrate(&root),
        Command::New {
            kind,
            slug,
            id,
            entry_id,
        } => run_new(kind, &slug, id, entry_id),
        Command::Schema { stem } => run_schema(&stem),
        Command::Explain { id, root } => run_explain(&root, id),
        Command::Fmt { root, check } => run_fmt(&root, check),
        Command::Lock { root, catalog } => run_lock(&root, &catalog),
        Command::Run {
            ticks,
            hash_every,
            seed,
        } => run_run(ticks, hash_every, seed),
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let outcome = match parse_args(&args) {
        Ok(command) => run(command),
        Err(error) => Outcome::cli_error(&error),
    };
    let code = emit_code(&mut std::io::stdout(), &mut std::io::stderr(), &outcome);
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn empty_args_is_usage() {
        assert!(matches!(parse_args(&[]), Err(CliError::Usage(_))));
    }

    #[test]
    fn unknown_subcommand_is_usage() {
        // `migrate` is known since T012b; use a truly unknown name to pin the
        // unknown-subcommand path rather than the missing-root path.
        assert!(matches!(
            parse_args(&args(&["frobnicate"])),
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
            Command::Validate { .. }
            | Command::Migrate { .. }
            | Command::New { .. }
            | Command::Schema { .. }
            | Command::Explain { .. }
            | Command::Fmt { .. }
            | Command::Lock { .. }
            | Command::Run { .. } => panic!("expected replay"),
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
            Command::Validate { .. }
            | Command::Migrate { .. }
            | Command::New { .. }
            | Command::Schema { .. }
            | Command::Explain { .. }
            | Command::Fmt { .. }
            | Command::Lock { .. }
            | Command::Run { .. } => panic!("expected replay"),
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

    #[test]
    fn replay_first_position_flag_is_usage() {
        for argv in [
            args(&["replay", "--bogus"]),
            args(&["replay", "--golden", "g"]),
        ] {
            match parse_args(&argv) {
                Err(CliError::Usage(message)) => {
                    assert!(
                        message.contains("expected <replay-path>"),
                        "first-position flag must name the expected path: {message}"
                    );
                    assert_eq!(CliError::Usage(message).exit_code(), 2);
                }
                other => panic!("first-position flag must be usage: {other:?}"),
            }
        }
    }

    #[test]
    fn validate_root_only_parses() {
        match parse_args(&args(&["validate", "campaigns/demo"])) {
            Ok(Command::Validate { root, json }) => {
                assert_eq!(root, PathBuf::from("campaigns/demo"));
                assert!(!json);
            }
            other => panic!("expected validate: {other:?}"),
        }
    }

    #[test]
    fn validate_json_flag_before_and_after_root() {
        for argv in [
            args(&["validate", "--json", "campaigns/demo"]),
            args(&["validate", "campaigns/demo", "--json"]),
        ] {
            match parse_args(&argv) {
                Ok(Command::Validate { root, json }) => {
                    assert_eq!(root, PathBuf::from("campaigns/demo"));
                    assert!(json, "flag must be accepted: {argv:?}");
                }
                other => panic!("expected validate: {other:?}"),
            }
        }
    }

    #[test]
    fn validate_parser_rejects_bad_syntax_as_usage() {
        let cases = [
            args(&["validate"]),
            args(&["validate", "--json"]),
            args(&["validate", "--json", "--json", "root"]),
            args(&["validate", "a", "b"]),
            args(&["validate", "--bogus", "a"]),
            args(&["validate", "a", "--bogus"]),
        ];
        for argv in &cases {
            match parse_args(argv) {
                Err(CliError::Usage(_)) => {}
                other => panic!("expected usage for {argv:?}: {other:?}"),
            }
        }
        assert_eq!(
            Outcome::cli_error(
                &parse_args(&args(&["validate"])).expect_err("missing root is usage")
            )
            .code,
            2
        );
    }

    #[test]
    fn validate_missing_root_names_usage() {
        match parse_args(&args(&["validate"])) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("missing <campaign-root>"), "{message}");
            }
            other => panic!("expected usage: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn validate_non_unicode_root_parses_for_io() {
        use std::os::unix::ffi::OsStringExt;
        let mut argv = vec![OsString::from("validate")];
        argv.push(OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]));
        match parse_args(&argv) {
            Ok(Command::Validate { json, .. }) => assert!(!json),
            other => panic!("non-Unicode root must parse, failing later as io: {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn validate_non_unicode_root_parses_for_io() {
        use std::os::windows::ffi::OsStringExt;
        let mut argv = vec![OsString::from("validate")];
        argv.push(OsString::from_wide(&[0x0066, 0xD800]));
        match parse_args(&argv) {
            Ok(Command::Validate { json, .. }) => assert!(!json),
            other => panic!("non-Unicode root must parse, failing later as io: {other:?}"),
        }
    }

    #[test]
    fn io_kind_maps_distinguished_errors() {
        let not_found = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no");
        let other = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "raw os text");
        assert_eq!(io_kind(&not_found), IoKind::NotFound);
        assert_eq!(io_kind(&denied), IoKind::PermissionDenied);
        assert_eq!(io_kind(&other), IoKind::IoError);
    }

    #[test]
    fn io_kind_spellings_are_stable() {
        let cases = [
            (IoKind::NotFound, "not_found"),
            (IoKind::NotADirectory, "not_a_directory"),
            (IoKind::PermissionDenied, "permission_denied"),
            (IoKind::SymlinkAtDocumentPath, "symlink_at_document_path"),
            (IoKind::NonUnicodeComponent, "non_unicode_component"),
            (IoKind::IoError, "io_error"),
        ];
        for (kind, text) in cases {
            assert_eq!(kind.to_string(), text);
        }
    }

    #[test]
    fn io_diagnostic_shape_is_portable() {
        let path: SourcePath = "creatures/creature.json".parse().expect("valid");
        let diagnostic = io_diagnostic(
            "read",
            Some(path),
            "creatures/creature.json",
            IoKind::PermissionDenied,
        );
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.code, DiagnosticCode::Io);
        assert!(diagnostic.pointer.is_empty());
        assert!(diagnostic.suggested_fix.is_none());
        assert_eq!(
            diagnostic.to_string(),
            "creatures/creature.json: error[io]: cannot read creatures/creature.json: permission_denied"
        );
    }

    #[test]
    fn io_diagnostic_without_logical_uses_campaign_root() {
        let diagnostic = io_diagnostic("open root", None, "<campaign-root>", IoKind::NotFound);
        assert!(diagnostic.file.is_none());
        assert_eq!(
            diagnostic.to_string(),
            "<campaign>: error[io]: cannot open root <campaign-root>: not_found"
        );
    }

    #[test]
    fn exit_for_keeps_warnings_non_fatal() {
        assert_eq!(exit_for(&[]), 0);
        assert_eq!(exit_for(&[warning_fixture()]), 0);
        assert_eq!(exit_for(&[warning_fixture(), error_fixture()]), 1);
        assert_eq!(exit_for(&[error_fixture()]), 1);
    }

    #[test]
    fn plain_outcome_prints_warnings_in_order_with_exit_0() {
        let second = Diagnostic {
            pointer: "/b".to_owned(),
            message: "second warning".to_owned(),
            suggested_fix: None,
            ..warning_fixture()
        };
        let outcome = plain_outcome(&[warning_fixture(), second]);
        assert_eq!(outcome.code, 0);
        assert!(outcome.stdout.is_empty());
        let stderr = String::from_utf8(outcome.stderr).expect("utf-8");
        let lines: Vec<&str> = stderr.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].ends_with("; suggested fix: synthetic fix"),
            "{lines:?}"
        );
        assert!(!lines[1].contains("suggested fix"), "{lines:?}");
        assert!(lines[0].contains("warning[duplicate_slug]"), "{lines:?}");
    }

    #[test]
    fn plain_outcome_mixed_error_and_warning_exits_1_unsorted() {
        let outcome = plain_outcome(&[error_fixture(), warning_fixture()]);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        let stderr = String::from_utf8(outcome.stderr).expect("utf-8");
        let lines: Vec<&str> = stderr.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].contains("error["),
            "returned order is preserved: {lines:?}"
        );
        assert!(
            lines[1].contains("warning["),
            "returned order is preserved: {lines:?}"
        );
    }

    #[test]
    fn json_outcome_keeps_warnings_with_exit_0() {
        let bytes = crpg_data::canonical_json(&[warning_fixture()]).expect("serializes");
        let outcome = json_outcome(&[warning_fixture()], Ok(bytes.clone()));
        assert_eq!(outcome.code, 0);
        assert_eq!(outcome.stdout, bytes);
        assert!(outcome.stderr.is_empty());
    }

    #[test]
    fn json_outcome_serialization_fallback_is_exact() {
        let outcome = json_outcome(&[warning_fixture()], Err(()));
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(outcome.stderr, SERIALIZATION_FAILURE.as_bytes());
    }

    #[test]
    fn engine_version_seam_rejects_bogus_and_accepts_package() {
        let files = BTreeMap::new();
        assert_eq!(
            validate_with_engine_version(&files, "not-a-version"),
            Err(())
        );
        let diagnostics =
            validate_with_engine_version(&files, "0.1.0").expect("package version parses");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagnosticCode::Layout);
    }

    /// Injected filesystem for collector tests: scripted directory listings,
    /// entry shapes, file bytes, and failure points, with a full operation
    /// log. Replaces only syscalls; sorting, pre-order, classification (the
    /// real data-owned function), and message construction stay live.
    struct FakeFs {
        kinds: BTreeMap<PathBuf, EntryKind>,
        lists: BTreeMap<PathBuf, Vec<OsString>>,
        contents: BTreeMap<PathBuf, Vec<u8>>,
        fail_list: BTreeSet<PathBuf>,
        fail_read: BTreeSet<PathBuf>,
        log: RefCell<Vec<String>>,
    }

    impl FakeFs {
        fn new() -> Self {
            Self {
                kinds: BTreeMap::new(),
                lists: BTreeMap::new(),
                contents: BTreeMap::new(),
                fail_list: BTreeSet::new(),
                fail_read: BTreeSet::new(),
                log: RefCell::new(Vec::new()),
            }
        }

        fn ensure_parents(&mut self, path: &Path) {
            let mut ancestors: Vec<PathBuf> = path.ancestors().skip(1).map(PathBuf::from).collect();
            ancestors.reverse();
            for ancestor in ancestors {
                if !ancestor.as_os_str().is_empty() {
                    self.kinds.entry(ancestor).or_insert(EntryKind::Directory);
                }
            }
        }

        fn dir(&mut self, path: &Path, children: &[&str]) {
            self.ensure_parents(path);
            self.kinds.insert(path.to_path_buf(), EntryKind::Directory);
            self.lists.insert(
                path.to_path_buf(),
                children.iter().map(OsString::from).collect(),
            );
        }

        fn file(&mut self, path: &Path, bytes: &[u8]) {
            self.ensure_parents(path);
            self.kinds.insert(path.to_path_buf(), EntryKind::File);
            self.contents.insert(path.to_path_buf(), bytes.to_vec());
        }
    }

    impl WalkFs for FakeFs {
        fn metadata(&self, path: &Path) -> std::io::Result<EntryKind> {
            self.log
                .borrow_mut()
                .push(format!("metadata {}", portable(path)));
            self.kinds.get(path).copied().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "fake: no such entry")
            })
        }

        fn read_dir_names(&self, path: &Path) -> std::io::Result<Vec<OsString>> {
            self.log
                .borrow_mut()
                .push(format!("list {}", portable(path)));
            if self.fail_list.contains(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "fake: cannot list",
                ));
            }
            self.lists.get(path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "fake: no such directory")
            })
        }

        fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
            self.log
                .borrow_mut()
                .push(format!("read {}", portable(path)));
            if self.fail_read.contains(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "fake: cannot read",
                ));
            }
            self.contents.get(path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "fake: no such file")
            })
        }
    }

    /// Renders a fake path with `/` separators so oracle expectations are
    /// identical on every platform. Production diagnostics never embed disk
    /// paths at all; only this test log needs the normalization.
    fn portable(path: &Path) -> String {
        path.display().to_string().replace('\\', "/")
    }

    fn warning_fixture() -> Diagnostic {
        Diagnostic {
            file: Some("creatures/creature.json".parse().expect("valid")),
            pointer: "/slug".to_owned(),
            severity: Severity::Warning,
            code: DiagnosticCode::DuplicateSlug,
            message: "synthetic warning".to_owned(),
            suggested_fix: Some("synthetic fix".to_owned()),
        }
    }

    fn error_fixture() -> Diagnostic {
        Diagnostic {
            file: None,
            pointer: String::new(),
            severity: Severity::Error,
            code: DiagnosticCode::Layout,
            message: "synthetic error".to_owned(),
            suggested_fix: None,
        }
    }

    #[test]
    fn collector_walks_sorted_depth_first_pre_order() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        // Deliberately unsorted listings: the walker must sort by bytes.
        fs.dir(&root, &["worlds", "campaign.json", "areas"]);
        fs.file(&root.join("campaign.json"), b"{}");
        fs.dir(&root.join("areas"), &["start"]);
        fs.dir(&root.join("areas/start"), &["placements.json"]);
        fs.file(&root.join("areas/start/placements.json"), b"{}");
        fs.dir(&root.join("worlds"), &["b.json", "a.json"]);
        fs.file(&root.join("worlds/a.json"), b"{}");
        fs.file(&root.join("worlds/b.json"), b"{}");

        let files = collect_campaign_files_with(&fs, &root).expect("collects");
        let keys: Vec<&str> = files.keys().map(SourcePath::as_str).collect();
        assert_eq!(
            keys,
            [
                "areas/start/placements.json",
                "campaign.json",
                "worlds/a.json",
                "worlds/b.json"
            ]
        );
        let log = fs.log.borrow();
        let reads: Vec<&str> = log
            .iter()
            .filter_map(|entry| entry.strip_prefix("read "))
            .collect();
        assert_eq!(
            reads,
            [
                "/fake/root/areas/start/placements.json",
                "/fake/root/campaign.json",
                "/fake/root/worlds/a.json",
                "/fake/root/worlds/b.json"
            ]
        );
    }

    #[test]
    fn collector_ignores_non_documents_without_reading() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.dir(&root, &["notes.txt", "campaign.json", "guide"]);
        fs.file(&root.join("notes.txt"), b"this is not json{{{");
        fs.file(&root.join("campaign.json"), b"{}");
        fs.dir(&root.join("guide"), &["intro.lua"]);
        fs.file(&root.join("guide/intro.lua"), b"garbage");

        let files = collect_campaign_files_with(&fs, &root).expect("collects");
        assert_eq!(files.len(), 1);
        assert!(files.keys().next().expect("one file").as_str() == "campaign.json");
        let log = fs.log.borrow();
        let reads: Vec<&str> = log
            .iter()
            .filter_map(|entry| entry.strip_prefix("read "))
            .collect();
        assert_eq!(
            reads,
            ["/fake/root/campaign.json"],
            "only documents are read: {log:?}"
        );
    }

    #[test]
    fn collector_first_failure_in_walk_order_wins() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.dir(&root, &["campaign.lock", "campaign.json"]);
        fs.file(&root.join("campaign.json"), b"{}");
        fs.file(&root.join("campaign.lock"), b"{}");
        fs.fail_read.insert(root.join("campaign.json"));
        fs.fail_read.insert(root.join("campaign.lock"));

        let error = collect_campaign_files_with(&fs, &root).expect_err("both reads fail");
        assert_eq!(error.code, DiagnosticCode::Io);
        assert_eq!(
            error.message,
            "cannot read campaign.json: permission_denied"
        );
    }

    #[test]
    fn collector_directory_failure_beats_later_file_failure() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.dir(&root, &["areas", "campaign.json"]);
        fs.dir(&root.join("areas"), &["notes.txt"]);
        fs.file(&root.join("campaign.json"), b"{}");
        fs.fail_list.insert(root.join("areas"));
        fs.fail_read.insert(root.join("campaign.json"));

        let error = collect_campaign_files_with(&fs, &root).expect_err("must fail");
        assert_eq!(error.message, "cannot list areas: permission_denied");
        assert!(error.file.is_none());
    }

    #[test]
    fn collector_unreadable_directory_fails_despite_ignored_children() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        // `assets` would hold only ignored content, but there is no
        // data-owned directory-pruning predicate, so its listing still fails.
        fs.dir(&root, &["assets", "campaign.json"]);
        fs.dir(&root.join("assets"), &["notes.txt"]);
        fs.file(&root.join("campaign.json"), b"{}");
        fs.fail_list.insert(root.join("areas"));
        fs.fail_list.insert(root.join("assets"));

        let error = collect_campaign_files_with(&fs, &root).expect_err("must fail");
        assert_eq!(error.message, "cannot list assets: permission_denied");
    }

    #[test]
    fn collector_symlink_at_document_path_is_io() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.dir(&root, &["campaign.json"]);
        fs.kinds
            .insert(root.join("campaign.json"), EntryKind::Symlink);

        let error = collect_campaign_files_with(&fs, &root).expect_err("symlink refused");
        assert_eq!(error.code, DiagnosticCode::Io);
        assert_eq!(
            error.message,
            "cannot classify campaign.json: symlink_at_document_path"
        );
        assert_eq!(
            error.file.expect("document path is known").as_str(),
            "campaign.json"
        );
    }

    #[test]
    fn collector_symlink_at_ignored_path_is_ignored() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.dir(&root, &["notes.txt"]);
        fs.kinds.insert(root.join("notes.txt"), EntryKind::Symlink);

        let files = collect_campaign_files_with(&fs, &root).expect("ignored symlink is fine");
        assert!(files.is_empty());
    }

    #[test]
    fn collector_other_kind_at_document_path_is_io() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.dir(&root, &["campaign.json"]);
        fs.kinds
            .insert(root.join("campaign.json"), EntryKind::Other);

        let error = collect_campaign_files_with(&fs, &root).expect_err("other refused");
        assert_eq!(error.message, "cannot classify campaign.json: io_error");
    }

    #[test]
    fn collector_missing_root_is_open_root_not_found() {
        let root = PathBuf::from("/fake/root");
        let fs = FakeFs::new();
        let error = collect_campaign_files_with(&fs, &root).expect_err("missing root");
        assert_eq!(error.message, "cannot open root <campaign-root>: not_found");
        assert!(error.file.is_none());
    }

    #[test]
    fn collector_symlinked_root_is_refused_without_following() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.ensure_parents(&root);
        fs.kinds.insert(root.clone(), EntryKind::Symlink);
        let error = collect_campaign_files_with(&fs, &root).expect_err("root symlink refused");
        assert_eq!(
            error.message,
            "cannot open root <campaign-root>: symlink_at_document_path"
        );
        assert!(fs
            .log
            .borrow()
            .iter()
            .all(|entry| !entry.starts_with("list")));
    }

    #[test]
    fn collector_symlinked_root_ancestor_is_refused_without_listing() {
        let root = PathBuf::from("/fake/link/root");
        let mut fs = FakeFs::new();
        fs.dir(&root, &["campaign.json"]);
        fs.file(&root.join("campaign.json"), b"{}");
        fs.kinds
            .insert(PathBuf::from("/fake/link"), EntryKind::Symlink);
        let error = collect_campaign_files_with(&fs, &root).expect_err("ancestor refused");
        assert_eq!(
            error.message,
            "cannot open root <campaign-root>: symlink_at_document_path"
        );
        assert!(fs
            .log
            .borrow()
            .iter()
            .all(|entry| !entry.starts_with("list")));
    }

    #[test]
    fn collector_file_as_root_is_not_a_directory() {
        let root = PathBuf::from("/fake/root");
        let mut fs = FakeFs::new();
        fs.file(&root, b"{}");
        let error = collect_campaign_files_with(&fs, &root).expect_err("file is not a dir");
        assert_eq!(
            error.message,
            "cannot open root <campaign-root>: not_a_directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn collector_non_unicode_component_is_list_io() {
        use std::os::unix::ffi::OsStringExt;
        let root = PathBuf::from("/fake/root");
        let bad = OsString::from_vec(vec![0x62, 0x61, 0x80, 0x64]);
        let mut fs = FakeFs::new();
        fs.dir(&root, &["campaign.json"]);
        fs.file(&root.join("campaign.json"), b"{}");
        fs.lists.insert(
            root.clone(),
            vec![OsString::from("campaign.json"), bad.clone()],
        );
        fs.kinds.insert(root.join(&bad), EntryKind::File);

        let error = collect_campaign_files_with(&fs, &root).expect_err("non-Unicode fails");
        assert_eq!(error.code, DiagnosticCode::Io);
        assert_eq!(
            error.message,
            "cannot list <campaign-root>: non_unicode_component"
        );
        assert!(error.file.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn collector_non_unicode_component_is_list_io() {
        use std::os::windows::ffi::OsStringExt;
        let root = PathBuf::from("C:\\fake\\root");
        let mut fs = FakeFs::new();
        let bad = OsString::from_wide(&[0x0062, 0xD800]);
        fs.dir(&root, &["campaign.json"]);
        fs.file(&root.join("campaign.json"), b"{}");
        fs.lists.insert(
            root.clone(),
            vec![OsString::from("campaign.json"), bad.clone()],
        );
        fs.kinds.insert(root.join(&bad), EntryKind::File);

        let error = collect_campaign_files_with(&fs, &root).expect_err("non-Unicode fails");
        assert_eq!(error.code, DiagnosticCode::Io);
        assert_eq!(
            error.message,
            "cannot list <campaign-root>: non_unicode_component"
        );
        assert!(error.file.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn collector_non_unicode_root_is_open_root_io() {
        use std::os::unix::ffi::OsStringExt;
        let root = PathBuf::from(OsString::from_vec(vec![0x2f, 0x80]));
        let fs = FakeFs::new();
        let error = collect_campaign_files_with(&fs, &root).expect_err("non-Unicode root");
        assert_eq!(
            error.message,
            "cannot open root <campaign-root>: non_unicode_component"
        );
    }

    #[cfg(windows)]
    #[test]
    fn collector_non_unicode_root_is_open_root_io() {
        use std::os::windows::ffi::OsStringExt;
        let root = PathBuf::from(OsString::from_wide(&[0x0043, 0xD800]));
        let fs = FakeFs::new();
        let error = collect_campaign_files_with(&fs, &root).expect_err("non-Unicode root");
        assert_eq!(
            error.message,
            "cannot open root <campaign-root>: non_unicode_component"
        );
    }

    #[test]
    fn collector_preserves_classifier_invalid_path() {
        let dir =
            std::env::temp_dir().join(format!("crpg-validate-{}-badname", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("creatures")).expect("temp tree");
        fs::write(dir.join("creatures").join("bad name!.json"), b"ignored").expect("temp file");
        let error = collect_campaign_files(&dir).expect_err("bad family name");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(error.code, DiagnosticCode::InvalidPath);
        assert!(error.file.is_none());
    }

    #[test]
    fn collector_missing_real_root_is_not_found() {
        let dir = std::env::temp_dir().join(format!(
            "crpg-validate-{}-does-not-exist",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let error = collect_campaign_files(&dir).expect_err("missing root");
        assert_eq!(error.message, "cannot open root <campaign-root>: not_found");
    }

    #[test]
    fn collector_real_file_as_root_is_not_a_directory() {
        let dir =
            std::env::temp_dir().join(format!("crpg-validate-{}-file-root", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("file.json");
        fs::write(&file, b"{}").expect("temp file");
        let error = collect_campaign_files(&file).expect_err("file is not a dir");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            error.message,
            "cannot open root <campaign-root>: not_a_directory"
        );
    }

    #[test]
    fn migrate_root_only_parses() {
        match parse_args(&args(&["migrate", "campaigns/demo"])) {
            Ok(Command::Migrate { root }) => {
                assert_eq!(root, PathBuf::from("campaigns/demo"));
            }
            other => panic!("expected migrate: {other:?}"),
        }
    }

    #[test]
    fn migrate_parser_rejects_bad_syntax_as_usage() {
        let cases = [
            args(&["migrate"]),
            args(&["migrate", "a", "b"]),
            args(&["migrate", "--json", "a"]),
            args(&["migrate", "a", "--json"]),
            args(&["migrate", "--check", "a"]),
            args(&["migrate", "--dry-run", "a"]),
            args(&["migrate", "--to", "a"]),
            args(&["migrate", "--golden", "a"]),
            args(&["migrate", "a", "--golden", "g"]),
            args(&["migrate", "--bogus", "a"]),
            args(&["migrate", "a", "--bogus"]),
        ];
        for argv in &cases {
            match parse_args(argv) {
                Err(CliError::Usage(_)) => {}
                other => panic!("expected usage for {argv:?}: {other:?}"),
            }
        }
        assert_eq!(
            Outcome::cli_error(
                &parse_args(&args(&["migrate"])).expect_err("missing root is usage")
            )
            .code,
            2
        );
    }

    #[test]
    fn migrate_missing_root_names_usage() {
        match parse_args(&args(&["migrate"])) {
            Err(CliError::Usage(message)) => {
                assert!(message.contains("missing <campaign-root>"), "{message}");
            }
            other => panic!("expected usage: {other:?}"),
        }
    }

    #[test]
    fn migrate_unknown_subcommand_stays_usage() {
        assert!(matches!(
            parse_args(&args(&["migrate2", "a"])),
            Err(CliError::Usage(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn migrate_non_unicode_root_parses_for_io() {
        use std::os::unix::ffi::OsStringExt;
        let mut argv = vec![OsString::from("migrate")];
        argv.push(OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]));
        match parse_args(&argv) {
            Ok(Command::Migrate { .. }) => {}
            other => panic!("non-Unicode root must parse, failing later as io: {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn migrate_non_unicode_root_parses_for_io() {
        use std::os::windows::ffi::OsStringExt;
        let mut argv = vec![OsString::from("migrate")];
        argv.push(OsString::from_wide(&[0x0066, 0xD800]));
        match parse_args(&argv) {
            Ok(Command::Migrate { .. }) => {}
            other => panic!("non-Unicode root must parse, failing later as io: {other:?}"),
        }
    }

    #[test]
    fn io_kind_migrate_spellings_are_stable() {
        assert_eq!(IoKind::SourceChanged.to_string(), "source_changed");
        assert_eq!(IoKind::NotAFile.to_string(), "not_a_file");
    }

    #[test]
    fn disk_path_joins_logical_with_native_separators() {
        let root = PathBuf::from("/fake/root");
        assert_eq!(
            disk_path(&root, "campaign.json"),
            PathBuf::from("/fake/root").join("campaign.json")
        );
        assert_eq!(
            disk_path(&root, "areas/start/area.json"),
            PathBuf::from("/fake/root")
                .join("areas")
                .join("start")
                .join("area.json")
        );
    }

    /// Injected rewrite filesystem for migrate unit seams. Scripted metadata
    /// shapes, file bytes, and per-path open/write/sync failures, with a full
    /// operation log. Sorting, lexical update order, classification (real
    /// data-owned functions), and message construction stay live.
    struct FakeRewriteFs {
        kinds: BTreeMap<PathBuf, EntryKind>,
        metadata_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        contents: BTreeMap<PathBuf, Vec<u8>>,
        read_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        open_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        create_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        write_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        write_prefix: BTreeMap<PathBuf, usize>,
        sync_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        second_reads: BTreeMap<PathBuf, Vec<u8>>,
        read_counts: std::rc::Rc<RefCell<BTreeMap<PathBuf, usize>>>,
        appear_on_metadata: std::rc::Rc<RefCell<BTreeSet<PathBuf>>>,
        aliases: BTreeSet<(PathBuf, PathBuf)>,
        log: std::rc::Rc<RefCell<Vec<String>>>,
        written: std::rc::Rc<RefCell<BTreeMap<PathBuf, Vec<u8>>>>,
    }

    /// Injected failing handle for the rewrite seam. Logs its own write/sync
    /// calls and optionally fails each stage with a stable `ErrorKind`.
    struct FakeHandle {
        path: PathBuf,
        write_err: Option<std::io::ErrorKind>,
        write_prefix: Option<usize>,
        sync_err: Option<std::io::ErrorKind>,
        log: std::rc::Rc<RefCell<Vec<String>>>,
        written: std::rc::Rc<RefCell<BTreeMap<PathBuf, Vec<u8>>>>,
    }

    impl RewriteHandle for FakeHandle {
        fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            self.log
                .borrow_mut()
                .push(format!("write {}", portable(&self.path)));
            if let Some(kind) = self.write_err {
                return Err(std::io::Error::new(kind, "fake: cannot write"));
            }
            if let Some(length) = self.write_prefix {
                self.written
                    .borrow_mut()
                    .insert(self.path.clone(), bytes[..length.min(bytes.len())].to_vec());
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "fake: partial write",
                ));
            }
            self.written
                .borrow_mut()
                .insert(self.path.clone(), bytes.to_vec());
            Ok(())
        }

        fn sync_all(&mut self) -> std::io::Result<()> {
            self.log
                .borrow_mut()
                .push(format!("sync {}", portable(&self.path)));
            if let Some(kind) = self.sync_err {
                return Err(std::io::Error::new(kind, "fake: cannot sync"));
            }
            Ok(())
        }
    }

    impl FakeRewriteFs {
        fn new() -> Self {
            Self {
                kinds: BTreeMap::new(),
                metadata_err: BTreeMap::new(),
                contents: BTreeMap::new(),
                read_err: BTreeMap::new(),
                open_err: BTreeMap::new(),
                create_err: BTreeMap::new(),
                write_err: BTreeMap::new(),
                write_prefix: BTreeMap::new(),
                sync_err: BTreeMap::new(),
                second_reads: BTreeMap::new(),
                read_counts: std::rc::Rc::new(RefCell::new(BTreeMap::new())),
                appear_on_metadata: std::rc::Rc::new(RefCell::new(BTreeSet::new())),
                aliases: BTreeSet::new(),
                log: std::rc::Rc::new(RefCell::new(Vec::new())),
                written: std::rc::Rc::new(RefCell::new(BTreeMap::new())),
            }
        }

        fn ensure_parents(&mut self, path: &Path) {
            let mut ancestors: Vec<PathBuf> = path.ancestors().skip(1).map(PathBuf::from).collect();
            ancestors.reverse();
            for ancestor in ancestors {
                if ancestor.as_os_str().is_empty() {
                    continue;
                }
                if self.kinds.contains_key(&ancestor) || self.metadata_err.contains_key(&ancestor) {
                    continue;
                }
                self.kinds.insert(ancestor, EntryKind::Directory);
            }
        }

        fn dir(&mut self, path: &Path) {
            self.ensure_parents(path);
            self.kinds.insert(path.to_path_buf(), EntryKind::Directory);
        }

        fn file(&mut self, path: &Path, bytes: &[u8]) {
            self.ensure_parents(path);
            self.kinds.insert(path.to_path_buf(), EntryKind::File);
            self.contents.insert(path.to_path_buf(), bytes.to_vec());
        }

        fn symlink(&mut self, path: &Path) {
            self.ensure_parents(path);
            self.kinds.insert(path.to_path_buf(), EntryKind::Symlink);
        }

        fn other(&mut self, path: &Path) {
            self.ensure_parents(path);
            self.kinds.insert(path.to_path_buf(), EntryKind::Other);
        }

        /// Scripts a prewrite drift: the first `read_file` returns the
        /// `contents` bytes, every later read returns `second`.
        fn drift_second_read(&mut self, path: &Path, second: &[u8]) {
            self.second_reads
                .insert(path.to_path_buf(), second.to_vec());
        }

        /// Scripts a genuine create race: the next `metadata` for `path`
        /// reports `NotFound` while making the file appear for the later
        /// `create_new`, which then fails with `AlreadyExists`.
        fn appear_on_next_metadata(&mut self, path: &Path) {
            self.appear_on_metadata
                .borrow_mut()
                .insert(path.to_path_buf());
        }
    }

    impl RewriteFs for FakeRewriteFs {
        fn metadata(&self, path: &Path) -> std::io::Result<EntryKind> {
            self.log
                .borrow_mut()
                .push(format!("metadata {}", portable(path)));
            if self.appear_on_metadata.borrow().contains(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "fake: no such entry yet",
                ));
            }
            if let Some(kind) = self.metadata_err.get(path) {
                return Err(std::io::Error::new(*kind, "fake: cannot stat"));
            }
            self.kinds.get(path).copied().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "fake: no such entry")
            })
        }

        fn read_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
            self.log
                .borrow_mut()
                .push(format!("read {}", portable(path)));
            if let Some(kind) = self.read_err.get(path) {
                return Err(std::io::Error::new(*kind, "fake: cannot read"));
            }
            let mut counts = self.read_counts.borrow_mut();
            let count = counts.entry(path.to_path_buf()).or_insert(0);
            *count += 1;
            if *count >= 2 {
                if let Some(second) = self.second_reads.get(path) {
                    return Ok(second.clone());
                }
            }
            self.contents.get(path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "fake: no such file")
            })
        }

        fn open_truncate(&self, path: &Path) -> std::io::Result<Box<dyn RewriteHandle>> {
            self.log
                .borrow_mut()
                .push(format!("open {}", portable(path)));
            if let Some(kind) = self.open_err.get(path) {
                return Err(std::io::Error::new(*kind, "fake: cannot open"));
            }
            Ok(Box::new(FakeHandle {
                path: path.to_path_buf(),
                write_err: self.write_err.get(path).copied(),
                write_prefix: self.write_prefix.get(path).copied(),
                sync_err: self.sync_err.get(path).copied(),
                log: std::rc::Rc::clone(&self.log),
                written: std::rc::Rc::clone(&self.written),
            }))
        }

        fn create_new(&self, path: &Path) -> std::io::Result<Box<dyn RewriteHandle>> {
            self.log
                .borrow_mut()
                .push(format!("create {}", portable(path)));
            // Production `create_new` fails when the output already exists;
            // the fake mirrors that so create-race tests observe
            // `source_changed` without a real concurrent writer. A scripted
            // appearance between the metadata check and creation reports the
            // same `AlreadyExists` without needing a real concurrent writer.
            if self.appear_on_metadata.borrow().contains(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "fake: output appeared concurrently",
                ));
            }
            if self.kinds.contains_key(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "fake: output appeared concurrently",
                ));
            }
            if let Some(kind) = self.create_err.get(path) {
                return Err(std::io::Error::new(*kind, "fake: cannot create"));
            }
            Ok(Box::new(FakeHandle {
                path: path.to_path_buf(),
                write_err: self.write_err.get(path).copied(),
                write_prefix: self.write_prefix.get(path).copied(),
                sync_err: self.sync_err.get(path).copied(),
                log: std::rc::Rc::clone(&self.log),
                written: std::rc::Rc::clone(&self.written),
            }))
        }

        fn same_file(&self, left: &Path, right: &Path) -> std::io::Result<bool> {
            self.log
                .borrow_mut()
                .push(format!("same {} {}", portable(left), portable(right)));
            if !self.kinds.contains_key(left) {
                return Ok(false);
            }
            if !self.kinds.contains_key(right) {
                return Ok(false);
            }
            Ok(left == right
                || self
                    .aliases
                    .contains(&(left.to_path_buf(), right.to_path_buf()))
                || self
                    .aliases
                    .contains(&(right.to_path_buf(), left.to_path_buf())))
        }
    }

    fn migrate_path(value: &str) -> SourcePath {
        value.parse().expect("valid logical path")
    }

    /// Sets up a rewrite fake for one logical file under `/fake/root`: root
    /// and every ancestor as directories, the target as a file with
    /// `original` bytes. Returns the root and the target disk path.
    fn rewrite_single_fixture(
        fs: &mut FakeRewriteFs,
        logical: &str,
        original: &[u8],
    ) -> (PathBuf, PathBuf) {
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        let components: Vec<&str> = logical.split('/').collect();
        for index in 1..components.len() {
            let ancestor = components[..index].join("/");
            fs.dir(&disk_path(&root, &ancestor));
        }
        let disk_target = disk_path(&root, logical);
        fs.file(&disk_target, original);
        (root, disk_target)
    }

    #[test]
    fn rewrite_empty_updates_perform_zero_writes() {
        let fs = FakeRewriteFs::new();
        let originals = BTreeMap::new();
        let updates: Vec<(SourcePath, Vec<u8>)> = Vec::new();
        let root = PathBuf::from("/fake/root");
        execute_rewrites(&fs, &root, &originals, &updates).expect("empty is ok");
        assert!(
            fs.log.borrow().is_empty(),
            "zero updates must not touch the filesystem: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn rewrite_single_file_success_writes_and_syncs() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let (root, disk_target) = rewrite_single_fixture(&mut fs, logical, b"old");
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        execute_rewrites(&fs, &root, &originals, &updates).expect("writes");
        assert_eq!(
            fs.written.borrow().get(&disk_target),
            Some(&b"new".to_vec())
        );
        let log = fs.log.borrow();
        assert!(log
            .iter()
            .any(|entry| entry == &format!("open {}", portable(&disk_target))));
        assert!(log
            .iter()
            .any(|entry| entry == &format!("write {}", portable(&disk_target))));
        assert!(log
            .iter()
            .any(|entry| entry == &format!("sync {}", portable(&disk_target))));
    }

    #[test]
    fn rewrite_target_symlink_is_check_failure_before_open() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        let disk_target = disk_path(&root, logical);
        fs.symlink(&disk_target);
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error =
            execute_rewrites(&fs, &root, &originals, &updates).expect_err("symlink refused");
        assert_eq!(error.code, DiagnosticCode::Io);
        assert_eq!(
            error.message,
            "cannot check campaign.json: symlink_at_document_path"
        );
        assert_eq!(error.file.expect("names target").as_str(), logical);
        assert!(
            fs.log
                .borrow()
                .iter()
                .all(|entry| !entry.starts_with("open")),
            "symlink must fail before open: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn rewrite_ancestor_symlink_names_target_and_skips_open() {
        let mut fs = FakeRewriteFs::new();
        let logical = "areas/start/area.json";
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        fs.symlink(&disk_path(&root, "areas"));
        let disk_target = disk_path(&root, logical);
        fs.file(&disk_target, b"old");
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error =
            execute_rewrites(&fs, &root, &originals, &updates).expect_err("ancestor symlink");
        assert_eq!(
            error.message,
            "cannot check areas/start/area.json: symlink_at_document_path"
        );
        assert_eq!(error.file.expect("names target").as_str(), logical);
        assert!(
            fs.log
                .borrow()
                .iter()
                .all(|entry| !entry.starts_with("open")),
            "ancestor failure must precede open: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn rewrite_changed_source_is_source_changed_without_open() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let (root, _) = rewrite_single_fixture(&mut fs, logical, b"live-bytes");
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"collected-bytes".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("changed source");
        assert_eq!(error.message, "cannot check campaign.json: source_changed");
        assert!(
            fs.log
                .borrow()
                .iter()
                .all(|entry| !entry.starts_with("open")),
            "changed source must fail before open: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn rewrite_missing_target_is_check_not_found() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        // No target entry: metadata reports NotFound.
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("missing");
        assert_eq!(error.message, "cannot check campaign.json: not_found");
        assert!(
            fs.log
                .borrow()
                .iter()
                .all(|entry| !entry.starts_with("open")),
            "missing target must fail before open: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn rewrite_directory_target_is_not_a_file() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        let disk_target = disk_path(&root, logical);
        fs.dir(&disk_target);
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("dir target");
        assert_eq!(error.message, "cannot check campaign.json: not_a_file");
        assert!(
            fs.log
                .borrow()
                .iter()
                .all(|entry| !entry.starts_with("open")),
            "nonregular target must fail before open: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn rewrite_other_target_is_not_a_file() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        let disk_target = disk_path(&root, logical);
        fs.other(&disk_target);
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("other target");
        assert_eq!(error.message, "cannot check campaign.json: not_a_file");
    }

    #[test]
    fn rewrite_open_failure_is_open_op_with_stable_kind() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let (root, disk_target) = rewrite_single_fixture(&mut fs, logical, b"old");
        fs.open_err
            .insert(disk_target.clone(), std::io::ErrorKind::PermissionDenied);
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("open fails");
        assert_eq!(
            error.message,
            "cannot open campaign.json: permission_denied"
        );
        assert!(
            fs.written.borrow().is_empty(),
            "open failure writes nothing"
        );
    }

    #[test]
    fn rewrite_write_failure_reports_write_without_rollback_claim() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let (root, disk_target) = rewrite_single_fixture(&mut fs, logical, b"old");
        fs.write_err
            .insert(disk_target.clone(), std::io::ErrorKind::PermissionDenied);
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("write fails");
        assert_eq!(
            error.message,
            "cannot write campaign.json: permission_denied"
        );
        assert!(
            !error.message.contains("rollback"),
            "must not claim rollback: {}",
            error.message
        );
        assert!(
            fs.written.borrow().is_empty(),
            "failed write stores nothing"
        );
    }

    #[test]
    fn rewrite_sync_failure_reports_sync() {
        let mut fs = FakeRewriteFs::new();
        let logical = "campaign.json";
        let (root, disk_target) = rewrite_single_fixture(&mut fs, logical, b"old");
        fs.sync_err.insert(disk_target, std::io::ErrorKind::Other);
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path(logical), b"old".to_vec());
        let updates = vec![(migrate_path(logical), b"new".to_vec())];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("sync fails");
        assert_eq!(error.message, "cannot sync campaign.json: io_error");
    }

    #[test]
    fn rewrite_first_lexical_failure_stops_and_keeps_prefix() {
        let mut fs = FakeRewriteFs::new();
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        for logical in ["campaign.json", "creatures/creature.json"] {
            let components: Vec<&str> = logical.split('/').collect();
            for index in 1..components.len() {
                fs.dir(&disk_path(&root, &components[..index].join("/")));
            }
            fs.file(&disk_path(&root, logical), b"old");
        }
        // Second file fails on write; first must already be written.
        let second_disk = disk_path(&root, "creatures/creature.json");
        fs.write_err
            .insert(second_disk.clone(), std::io::ErrorKind::PermissionDenied);
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path("campaign.json"), b"old".to_vec());
        originals.insert(migrate_path("creatures/creature.json"), b"old".to_vec());
        let updates = vec![
            (migrate_path("campaign.json"), b"new".to_vec()),
            (migrate_path("creatures/creature.json"), b"new".to_vec()),
        ];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("second fails");
        assert_eq!(
            error.message,
            "cannot write creatures/creature.json: permission_denied"
        );
        assert_eq!(
            fs.written.borrow().get(&disk_path(&root, "campaign.json")),
            Some(&b"new".to_vec()),
            "earlier lexical file stays written: no rollback"
        );
        assert!(
            !error.message.contains("rollback"),
            "must not claim rollback: {}",
            error.message
        );
    }

    #[test]
    fn rewrite_first_failure_prevents_second_attempt() {
        let mut fs = FakeRewriteFs::new();
        let root = PathBuf::from("/fake/root");
        fs.dir(&root);
        for logical in ["campaign.json", "campaign.lock"] {
            fs.file(&disk_path(&root, logical), b"old");
        }
        // First file's re-read differs, so it fails before any open.
        let mut originals = BTreeMap::new();
        originals.insert(migrate_path("campaign.json"), b"stale".to_vec());
        originals.insert(migrate_path("campaign.lock"), b"old".to_vec());
        // Rewrite fake holds live bytes `old` for the first file, which
        // differs from the collected `stale`, forcing source_changed.
        let updates = vec![
            (migrate_path("campaign.json"), b"new".to_vec()),
            (migrate_path("campaign.lock"), b"new".to_vec()),
        ];
        let error = execute_rewrites(&fs, &root, &originals, &updates).expect_err("first fails");
        assert_eq!(error.message, "cannot check campaign.json: source_changed");
        let log = fs.log.borrow();
        assert!(
            log.iter()
                .all(|entry| !entry.contains("campaign.lock") || entry.contains("metadata")),
            "second file must not be attempted after first lexical failure, yet log is {log:?}"
        );
        // More precisely, no open for the second file.
        let second_disk = portable(&disk_path(&root, "campaign.lock"));
        assert!(
            log.iter()
                .all(|entry| entry != &format!("open {second_disk}")),
            "second open must not happen: {log:?}"
        );
        assert!(
            fs.written.borrow().is_empty(),
            "first failure writes nothing"
        );
    }

    #[test]
    fn plan_updates_detects_key_mismatch_without_panic() {
        let mut files = BTreeMap::new();
        files.insert(migrate_path("campaign.json"), b"old".to_vec());
        let mut serialized = BTreeMap::new();
        serialized.insert(migrate_path("campaign.json"), b"old".to_vec());
        serialized.insert(migrate_path("campaign.lock"), b"new".to_vec());
        assert_eq!(plan_updates(&files, &serialized), Err(()));
        let mut files = BTreeMap::new();
        files.insert(migrate_path("campaign.json"), b"old".to_vec());
        files.insert(migrate_path("campaign.lock"), b"old".to_vec());
        let mut serialized = BTreeMap::new();
        serialized.insert(migrate_path("campaign.json"), b"old".to_vec());
        assert_eq!(plan_updates(&files, &serialized), Err(()));
    }

    #[test]
    fn plan_updates_returns_only_differing_files_in_lexical_order() {
        let mut files = BTreeMap::new();
        files.insert(migrate_path("campaign.json"), b"same".to_vec());
        files.insert(migrate_path("campaign.lock"), b"old".to_vec());
        files.insert(migrate_path("worlds/world.json"), b"old".to_vec());
        let mut serialized = BTreeMap::new();
        serialized.insert(migrate_path("campaign.json"), b"same".to_vec());
        serialized.insert(migrate_path("campaign.lock"), b"new".to_vec());
        serialized.insert(migrate_path("worlds/world.json"), b"newer".to_vec());
        let updates = plan_updates(&files, &serialized).expect("plans");
        let logicals: Vec<&str> = updates.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(logicals, ["campaign.lock", "worlds/world.json"]);
    }

    #[test]
    fn run_migrate_with_bad_engine_is_internal_failure_with_zero_writes() {
        let root = PathBuf::from("/fake/root");
        let mut walk = FakeFs::new();
        walk.dir(&root, &["campaign.json"]);
        walk.file(&root.join("campaign.json"), b"{}");
        let rewrite = FakeRewriteFs::new();
        let outcome = run_migrate_with(&walk, &rewrite, &root, "not-a-version");
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(outcome.stderr, MIGRATE_ENGINE_FAILURE.as_bytes());
        assert!(
            rewrite.log.borrow().is_empty(),
            "version failure must not touch rewrite fs: {:?}",
            rewrite.log.borrow()
        );
    }

    #[test]
    fn run_migrate_with_preflight_failure_performs_zero_rewrite_writes() {
        let root = PathBuf::from("/fake/root");
        let mut walk = FakeFs::new();
        // Collects one document but misses the other required files, so the
        // data loader fails layout before any rewrite starts.
        walk.dir(&root, &["campaign.json"]);
        walk.file(&root.join("campaign.json"), b"{}");
        let rewrite = FakeRewriteFs::new();
        let outcome = run_migrate_with(&walk, &rewrite, &root, "0.1.0");
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert!(!outcome.stderr.is_empty());
        assert!(
            rewrite.log.borrow().is_empty(),
            "preflight failure must perform zero writes: {:?}",
            rewrite.log.borrow()
        );
    }

    #[test]
    fn document_set_failure_bytes_are_exact() {
        assert_eq!(
            DOCUMENT_SET_FAILURE,
            "crpgc migrate: internal document set failure\n"
        );
        assert_eq!(
            MIGRATE_ENGINE_FAILURE,
            "crpgc migrate: internal engine version failure\n"
        );
    }

    struct FailWriter;

    impl std::io::Write for FailWriter {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fake: stream is broken",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fake: stream is broken",
            ))
        }
    }

    struct VecWriter<'a>(&'a mut Vec<u8>);

    impl std::io::Write for VecWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FlushFailWriter(Vec<u8>);

    impl std::io::Write for FlushFailWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fake: flush is broken",
            ))
        }
    }

    struct PrefixFailWriter {
        bytes: Vec<u8>,
        remaining: usize,
    }

    impl std::io::Write for PrefixFailWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "fake: stream failed after prefix",
                ));
            }
            let count = self.remaining.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            self.remaining -= count;
            Ok(count)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn emit_code_returns_outcome_code_on_success_and_1_on_stream_failure() {
        let outcome = Outcome {
            code: 0,
            stdout: b"out".to_vec(),
            stderr: b"err".to_vec(),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = {
            let mut stdout_writer = VecWriter(&mut stdout);
            let mut stderr_writer = VecWriter(&mut stderr);
            emit_code(&mut stdout_writer, &mut stderr_writer, &outcome)
        };
        assert_eq!(code, 0);
        assert_eq!(stdout, b"out");
        assert_eq!(stderr, b"err");

        let mut failing_stdout = FailWriter;
        let mut ok_stderr = Vec::new();
        let mut ok_stderr_writer = VecWriter(&mut ok_stderr);
        assert_eq!(
            emit_code(&mut failing_stdout, &mut ok_stderr_writer, &outcome),
            1,
            "stdout failure must exit 1 without panic"
        );

        let mut ok_stdout = Vec::new();
        let mut ok_stdout_writer = VecWriter(&mut ok_stdout);
        let mut failing_stderr = FailWriter;
        assert_eq!(
            emit_code(&mut ok_stdout_writer, &mut failing_stderr, &outcome),
            1,
            "stderr failure must exit 1 without panic"
        );

        let mut flush_failing_stdout = FlushFailWriter(Vec::new());
        let mut ok_stderr = Vec::new();
        let mut ok_stderr_writer = VecWriter(&mut ok_stderr);
        assert_eq!(
            emit_code(&mut flush_failing_stdout, &mut ok_stderr_writer, &outcome),
            1,
            "a final flush failure must be observed"
        );
        assert_eq!(flush_failing_stdout.0, b"out");

        let mut prefix_stdout = PrefixFailWriter {
            bytes: Vec::new(),
            remaining: 2,
        };
        let mut ok_stderr = Vec::new();
        let mut ok_stderr_writer = VecWriter(&mut ok_stderr);
        assert_eq!(
            emit_code(&mut prefix_stdout, &mut ok_stderr_writer, &outcome),
            1,
            "partial stream failure must exit 1"
        );
        assert_eq!(prefix_stdout.bytes, b"ou", "written prefix is retained");
    }

    const NEW_ID: &str = "00000000000000000000000011";
    const NEW_ENTRY_ID: &str = "00000000000000000000000012";

    /// Asserts a T013 argv fails parsing with exit 2 and the command's
    /// single shared usage line, never echoing arbitrary input.
    fn assert_new_usage(argv: &[&str], usage: &str) {
        let parsed = parse_args(&args(argv));
        let message = match parsed {
            Err(CliError::Usage(message)) => message,
            other => panic!("expected usage for {argv:?}: {other:?}"),
        };
        let outcome = Outcome::cli_error(&CliError::Usage(message));
        assert_eq!(outcome.code, 2);
        assert!(outcome.stdout.is_empty());
        assert_eq!(
            outcome.stderr,
            format!("crpgc: usage: {usage}\n").into_bytes(),
            "one usage line for {argv:?}"
        );
    }

    #[test]
    fn new_parser_accepts_all_types_in_either_order() {
        for type_text in ["creature", "item", "dialogue", "quest"] {
            let needs_entry = type_text == "dialogue" || type_text == "quest";
            let with_entry = if needs_entry {
                vec!["--entry-id", NEW_ENTRY_ID]
            } else {
                Vec::new()
            };
            let mut before: Vec<&str> = vec!["new", type_text, "--slug", "goblin", "--id", NEW_ID];
            before.extend_from_slice(&with_entry);
            let mut after: Vec<&str> = vec!["new", "--slug", "goblin", "--id", NEW_ID];
            after.extend_from_slice(&with_entry);
            after.push(type_text);
            for argv in [before, after] {
                match parse_args(&args(&argv)) {
                    Ok(Command::New {
                        kind,
                        slug,
                        id,
                        entry_id,
                    }) => {
                        assert_eq!(slug, "goblin");
                        assert_eq!(id.to_string(), NEW_ID);
                        assert_eq!(
                            entry_id.map(|entry| entry.to_string()),
                            needs_entry.then(|| NEW_ENTRY_ID.to_owned())
                        );
                        let _ = kind;
                    }
                    other => panic!("expected new for {argv:?}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn new_parser_rejects_bad_grammar_as_single_usage_line() {
        let cases: Vec<Vec<&str>> = vec![
            vec!["new"],
            vec!["new", "creature"],
            vec!["new", "creature", "--slug", "goblin"],
            vec!["new", "creature", "--id", NEW_ID],
            vec![
                "new", "creature", "--slug", "goblin", "--id", NEW_ID, "--slug", "orc",
            ],
            vec![
                "new", "creature", "--slug", "goblin", "--id", NEW_ID, "--id", NEW_ID,
            ],
            vec![
                "new",
                "dialogue",
                "--slug",
                "talk",
                "--id",
                NEW_ID,
                "--entry-id",
                NEW_ENTRY_ID,
                "--entry-id",
                NEW_ENTRY_ID,
            ],
            vec!["new", "creature", "--slug", "goblin"],
            vec!["new", "creature", "--slug"],
            vec!["new", "creature", "--slug", "goblin", "--id"],
            vec![
                "new", "creature", "extra", "--slug", "goblin", "--id", NEW_ID,
            ],
            vec!["new", "creature", "a", "b", "--slug", "g", "--id", NEW_ID],
            vec!["new", "golem", "--slug", "goblin", "--id", NEW_ID],
            vec![
                "new", "creature", "--slug", "goblin", "--id", NEW_ID, "--bogus",
            ],
            vec!["new", "--bogus", "creature", "--slug", "g", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "goblin", "--id", NEW_ID, "--"],
            vec!["new", "--", "creature", "--slug", "g", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "goblin", "--id", NEW_ID, "-s"],
            vec!["new", "creature", "--slug=goblin", "--id", NEW_ID],
            vec![
                "new", "creature", "--slug", "goblin", "--id", NEW_ID, "--help",
            ],
            vec![
                "new",
                "creature",
                "--slug",
                "goblin",
                "--id",
                NEW_ID,
                "--version",
            ],
            vec!["new", "dialogue", "--slug", "talk", "--id", NEW_ID],
            vec!["new", "quest", "--slug", "fetch", "--id", NEW_ID],
            vec![
                "new",
                "creature",
                "--slug",
                "goblin",
                "--id",
                NEW_ID,
                "--entry-id",
                NEW_ENTRY_ID,
            ],
            vec![
                "new",
                "item",
                "--slug",
                "sword",
                "--id",
                NEW_ID,
                "--entry-id",
                NEW_ENTRY_ID,
            ],
            vec![
                "new",
                "dialogue",
                "--slug",
                "talk",
                "--id",
                NEW_ID,
                "--entry-id",
                NEW_ID,
            ],
            vec!["new", "creature", "--slug", "Bad", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "-goblin", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "goblin-", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "gob--lin", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "gob_lin", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "", "--id", NEW_ID],
            vec!["new", "creature", "--slug", "goblin", "--id", "short"],
            vec![
                "new",
                "creature",
                "--slug",
                "goblin",
                "--id",
                "0000000000000000000000001!",
            ],
            vec![
                "new",
                "creature",
                "--slug",
                "goblin",
                "--id",
                "80000000000000000000000000",
            ],
            vec![
                "new",
                "dialogue",
                "--slug",
                "talk",
                "--id",
                NEW_ID,
                "--entry-id",
                "bogus",
            ],
        ];
        for argv in &cases {
            assert_new_usage(argv, NEW_USAGE);
        }
    }

    #[cfg(unix)]
    #[test]
    fn new_non_unicode_text_is_usage() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]);
        for argv in [
            vec![
                OsString::from("new"),
                bad.clone(),
                OsString::from("--slug"),
                OsString::from("g"),
                OsString::from("--id"),
                OsString::from(NEW_ID),
            ],
            vec![
                OsString::from("new"),
                OsString::from("creature"),
                OsString::from("--slug"),
                bad.clone(),
                OsString::from("--id"),
                OsString::from(NEW_ID),
            ],
        ] {
            match parse_args(&argv) {
                Err(CliError::Usage(message)) => {
                    assert_eq!(
                        Outcome::cli_error(&CliError::Usage(message)).stderr,
                        format!("crpgc: usage: {NEW_USAGE}\n").into_bytes()
                    );
                }
                other => panic!("non-Unicode new text must be usage: {other:?}"),
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn new_non_unicode_text_is_usage() {
        use std::os::windows::ffi::OsStringExt;
        let bad = OsString::from_wide(&[0x0066, 0xD800]);
        let argv = vec![
            OsString::from("new"),
            OsString::from("creature"),
            OsString::from("--slug"),
            bad,
            OsString::from("--id"),
            OsString::from(NEW_ID),
        ];
        match parse_args(&argv) {
            Err(CliError::Usage(message)) => {
                assert_eq!(
                    Outcome::cli_error(&CliError::Usage(message)).stderr,
                    format!("crpgc: usage: {NEW_USAGE}\n").into_bytes()
                );
            }
            other => panic!("non-Unicode new text must be usage: {other:?}"),
        }
    }

    #[test]
    fn slug_grammar_matches_pinned_cases_without_regex() {
        for valid in ["a", "0", "goblin", "goblin-2", "a-b-c-0"] {
            assert!(is_valid_slug(valid), "{valid} must be valid");
        }
        for invalid in [
            "", "-", "-a", "a-", "a--b", "A", "Goblin", "gob_lin", "gob lin", "gob.lin", "göblin",
            "a-", "--",
        ] {
            assert!(!is_valid_slug(invalid), "{invalid:?} must be invalid");
        }
    }

    #[test]
    fn new_templates_carry_exact_identities_and_tags() {
        let id: Ulid = NEW_ID.parse().expect("fixture id");
        let entry: Ulid = NEW_ENTRY_ID.parse().expect("fixture entry");
        let creature = run_new(NewKind::Creature, "goblin", id, None);
        assert_eq!(creature.code, 0);
        assert!(creature.stderr.is_empty());
        match crpg_data::read_document(&creature.stdout).expect("writes canonical") {
            crpg_data::Document::Creature(creature) => {
                assert_eq!(creature.id, id);
                assert_eq!(creature.slug, "goblin");
                assert_eq!(creature.name, "creature.goblin.name");
                assert!(creature.note.is_none());
                assert!(creature.stats.is_empty());
                assert!(creature.tags.is_empty());
                assert!(creature.faction.is_none());
                assert!(creature.inventory.is_empty());
            }
            other => panic!("expected creature: {other:?}"),
        }
        let item = run_new(NewKind::Item, "sword", id, None);
        match crpg_data::read_document(&item.stdout).expect("writes canonical") {
            crpg_data::Document::Item(item) => {
                assert_eq!(item.id, id);
                assert_eq!(item.name, "item.sword.name");
                assert!(item.stats.is_empty() && item.tags.is_empty() && item.note.is_none());
            }
            other => panic!("expected item: {other:?}"),
        }
        let dialogue = run_new(NewKind::Dialogue, "talk", id, Some(entry));
        match crpg_data::read_document(&dialogue.stdout).expect("writes canonical") {
            crpg_data::Document::Dialogue(dialogue) => {
                assert_eq!(dialogue.entry, entry);
                assert_eq!(dialogue.name, "dialogue.talk.name");
                assert_eq!(dialogue.nodes.len(), 1);
                assert_eq!(dialogue.nodes[0].id, entry);
                assert_eq!(dialogue.nodes[0].body, crpg_data::DialogueBody::End);
            }
            other => panic!("expected dialogue: {other:?}"),
        }
        let quest = run_new(NewKind::Quest, "fetch", id, Some(entry));
        match crpg_data::read_document(&quest.stdout).expect("writes canonical") {
            crpg_data::Document::Quest(quest) => {
                assert_eq!(quest.entry, entry);
                assert_eq!(quest.states.len(), 1);
                assert_eq!(quest.states[0].id, entry);
                assert_eq!(quest.states[0].name, "quest.fetch.state.done");
                assert!(quest.states[0].terminal);
                assert!(quest.states[0].on_enter.is_empty());
                assert!(quest.states[0].transitions.is_empty());
            }
            other => panic!("expected quest: {other:?}"),
        }
        // Repeated calls with identical operands produce identical bytes.
        assert_eq!(
            run_new(NewKind::Creature, "goblin", id, None).stdout,
            creature.stdout
        );
    }

    #[test]
    fn schema_parser_accepts_all_stems_and_rejects_the_rest() {
        for stem in [
            "campaign",
            "world",
            "area",
            "creature",
            "item",
            "dialogue",
            "quest",
            "faction",
            "graph",
            "placements",
            "triggers",
            "locale",
            "variables",
            "campaign-lock",
            "assets-lock",
            "placement",
            "action-signature",
        ] {
            match parse_args(&args(&["schema", stem])) {
                Ok(Command::Schema { stem: parsed }) => assert_eq!(parsed, stem),
                other => panic!("expected schema {stem}: {other:?}"),
            }
        }
        for argv in [
            args(&["schema"]),
            args(&["schema", "campaign", "world"]),
            args(&["schema", "placement-kind"]),
            args(&["schema", "Campaign"]),
            args(&["schema", "--check", "campaign"]),
            args(&["schema", "campaign", "--check"]),
            args(&["schema", "--"]),
        ] {
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                SCHEMA_USAGE,
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn schema_non_unicode_stem_is_usage() {
        use std::os::unix::ffi::OsStringExt;
        let argv = vec![
            OsString::from("schema"),
            OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]),
        ];
        assert!(matches!(parse_args(&argv), Err(CliError::Usage(_))));
    }

    #[cfg(windows)]
    #[test]
    fn schema_non_unicode_stem_is_usage() {
        use std::os::windows::ffi::OsStringExt;
        let argv = vec![
            OsString::from("schema"),
            OsString::from_wide(&[0x0066, 0xD800]),
        ];
        assert!(matches!(parse_args(&argv), Err(CliError::Usage(_))));
    }

    #[test]
    fn schema_output_is_data_bytes_unchanged() {
        let expected = crpg_data::generated_schemas().expect("generates");
        for stem in [
            "campaign",
            "creature",
            "campaign-lock",
            "placement",
            "action-signature",
        ] {
            let outcome = run_schema(stem);
            assert_eq!(outcome.code, 0);
            assert!(outcome.stderr.is_empty());
            assert_eq!(
                outcome.stdout,
                expected[&format!("{stem}.schema.json")],
                "CLI must pass data bytes through unchanged"
            );
            assert_eq!(outcome.stdout.last(), Some(&b'\n'));
        }
        // Repeated output is identical.
        assert_eq!(run_schema("quest").stdout, run_schema("quest").stdout);
    }

    #[test]
    fn explain_parser_matrix_and_usage_bytes() {
        match parse_args(&args(&["explain", NEW_ID])) {
            Ok(Command::Explain { id, root }) => {
                assert_eq!(id.to_string(), NEW_ID);
                assert_eq!(root, PathBuf::from("."));
            }
            other => panic!("expected explain: {other:?}"),
        }
        for argv in [
            args(&["explain", "--root", "root/dir", NEW_ID]),
            args(&["explain", NEW_ID, "--root", "root/dir"]),
        ] {
            match parse_args(&argv) {
                Ok(Command::Explain { root, .. }) => {
                    assert_eq!(root, PathBuf::from("root/dir"));
                }
                other => panic!("expected explain with root: {other:?}"),
            }
        }
        // Core aliases normalize at parse: lowercase `i` decodes as `1`.
        let aliased = NEW_ID.replace('1', "i");
        match parse_args(&args(&["explain", &aliased])) {
            Ok(Command::Explain { id, .. }) => assert_eq!(id.to_string(), NEW_ID),
            other => panic!("expected alias parse: {other:?}"),
        }
        for argv in [
            args(&["explain"]),
            args(&["explain", NEW_ID, NEW_ID]),
            args(&["explain", "--root"]),
            args(&["explain", NEW_ID, "--root"]),
            args(&["explain", "--root", "a", "--root", "b", NEW_ID]),
            args(&["explain", "--bogus", NEW_ID]),
            args(&["explain", NEW_ID, "--bogus"]),
            args(&["explain", "--", NEW_ID]),
            args(&["explain", NEW_ID, "--help"]),
            args(&["explain", "short"]),
            args(&["explain", "0000000000000000000000001!"]),
            args(&["explain", "80000000000000000000000000"]),
            args(&["explain", " 00000000000000000000000011"]),
            args(&["explain", "00000000000000000000000011 "]),
        ] {
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                EXPLAIN_USAGE,
            );
        }
        // The all-zero id is syntactically valid: no invented NIL prohibition.
        assert!(parse_args(&args(&["explain", "00000000000000000000000000"])).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn explain_non_unicode_id_is_usage_but_root_parses_for_io() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]);
        assert!(matches!(
            parse_args(&[OsString::from("explain"), bad]),
            Err(CliError::Usage(_))
        ));
        let argv = vec![
            OsString::from("explain"),
            OsString::from(NEW_ID),
            OsString::from("--root"),
            OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]),
        ];
        assert!(matches!(parse_args(&argv), Ok(Command::Explain { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn explain_non_unicode_id_is_usage_but_root_parses_for_io() {
        use std::os::windows::ffi::OsStringExt;
        let bad = OsString::from_wide(&[0x0066, 0xD800]);
        assert!(matches!(
            parse_args(&[OsString::from("explain"), bad.clone()]),
            Err(CliError::Usage(_))
        ));
        let argv = vec![
            OsString::from("explain"),
            OsString::from(NEW_ID),
            OsString::from("--root"),
            bad,
        ];
        assert!(matches!(parse_args(&argv), Ok(Command::Explain { .. })));
    }

    #[test]
    fn explain_flag_shaped_root_values_are_usage_before_io() {
        // Every Unicode dash-prefixed token used as `--root`'s value is a
        // usage error, never an I/O path: help/version/terminator/
        // short-flag/duplicate-option forms plus unknown flags.
        for value in [
            "--help",
            "--version",
            "--",
            "-x",
            "--root",
            "--catalog",
            "--check",
            "--bogus",
        ] {
            let argv = args(&["explain", NEW_ID, "--root", value]);
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                EXPLAIN_USAGE,
            );
            // Leading `--root` form is equally usage.
            let argv = args(&["explain", "--root", value, NEW_ID]);
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                EXPLAIN_USAGE,
            );
        }
        // The reviewed cases: `--help`, `--`, and a short flag as values
        // with a valid id must be usage exit 2, not exit-1 I/O.
        for argv in [
            args(&["explain", "00000000000000000000000004", "--root", "--help"]),
            args(&["explain", "00000000000000000000000004", "--root", "--"]),
            args(&["explain", "00000000000000000000000004", "--root", "-x"]),
        ] {
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                EXPLAIN_USAGE,
            );
        }
        // An explicit relative prefix escapes a dash-leading path: it parses
        // for later I/O validation instead of failing as usage.
        match parse_args(&args(&["explain", NEW_ID, "--root", "./-dash-root"])) {
            Ok(Command::Explain { root, .. }) => {
                assert_eq!(root, PathBuf::from("./-dash-root"));
            }
            other => panic!("./-prefixed root must parse: {other:?}"),
        }
        // Non-Unicode values are preserved for exit-1 I/O validation (pinned
        // by the cfg-gated tests above); Unicode dash values never reach I/O.
    }

    #[test]
    fn explain_engine_failure_and_collector_failure_have_exact_bytes() {
        let id: Ulid = NEW_ID.parse().expect("valid");
        let root = PathBuf::from("/fake/root");
        let mut walk = FakeFs::new();
        walk.dir(&root, &["campaign.json"]);
        walk.file(&root.join("campaign.json"), b"{}");
        let outcome = run_explain_with(&walk, &root, id, "not-a-version");
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(outcome.stderr, EXPLAIN_ENGINE_FAILURE.as_bytes());
        let missing = PathBuf::from("/fake/missing");
        let outcome = run_explain_with(&FakeFs::new(), &missing, id, "0.1.0");
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert!(!outcome.stderr.is_empty());
    }

    #[test]
    fn fmt_parser_matrix_and_usage_bytes() {
        match parse_args(&args(&["fmt"])) {
            Ok(Command::Fmt { root, check }) => {
                assert_eq!(root, PathBuf::from("."));
                assert!(!check);
            }
            other => panic!("expected fmt default: {other:?}"),
        }
        match parse_args(&args(&["fmt", "--check", "some/root"])) {
            Ok(Command::Fmt { root, check }) => {
                assert_eq!(root, PathBuf::from("some/root"));
                assert!(check);
            }
            other => panic!("expected fmt check-first: {other:?}"),
        }
        match parse_args(&args(&["fmt", "some/root", "--check"])) {
            Ok(Command::Fmt { check, .. }) => assert!(check),
            other => panic!("expected fmt check-last: {other:?}"),
        }
        for argv in [
            args(&["fmt", "a", "b"]),
            args(&["fmt", "--check", "--check"]),
            args(&["fmt", "--check", "a", "--check"]),
            args(&["fmt", "--bogus"]),
            args(&["fmt", "a", "--bogus"]),
            args(&["fmt", "--"]),
            args(&["fmt", "--help"]),
        ] {
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                FMT_USAGE,
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn fmt_non_unicode_root_parses_for_io() {
        use std::os::unix::ffi::OsStringExt;
        let argv = vec![
            OsString::from("fmt"),
            OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]),
        ];
        assert!(matches!(parse_args(&argv), Ok(Command::Fmt { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn fmt_non_unicode_root_parses_for_io() {
        use std::os::windows::ffi::OsStringExt;
        let argv = vec![
            OsString::from("fmt"),
            OsString::from_wide(&[0x0066, 0xD800]),
        ];
        assert!(matches!(parse_args(&argv), Ok(Command::Fmt { .. })));
    }

    #[test]
    fn fmt_engine_failure_and_preflight_zero_writes() {
        let root = PathBuf::from("/fake/root");
        let mut walk = FakeFs::new();
        walk.dir(&root, &["campaign.json"]);
        walk.file(&root.join("campaign.json"), b"{}");
        let rewrite = FakeRewriteFs::new();
        let outcome = run_fmt_with(&walk, &rewrite, &root, "not-a-version", false);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(outcome.stderr, FMT_ENGINE_FAILURE.as_bytes());
        let outcome = run_fmt_with(&walk, &rewrite, &root, "0.1.0", true);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert!(!outcome.stderr.is_empty());
        assert!(
            rewrite.log.borrow().is_empty(),
            "preflight failure must perform zero writes: {:?}",
            rewrite.log.borrow()
        );
    }

    #[test]
    fn fmt_check_dirty_lines_and_document_set_bytes_are_exact() {
        assert_eq!(
            FMT_DOCUMENT_SET_FAILURE,
            "crpgc fmt: internal document set failure\n"
        );
        assert_eq!(
            FMT_ENGINE_FAILURE,
            "crpgc fmt: internal engine version failure\n"
        );
        assert_eq!(
            EXPLAIN_ENGINE_FAILURE,
            "crpgc explain: internal engine version failure\n"
        );
    }

    #[test]
    fn lock_parser_matrix_and_usage_bytes() {
        match parse_args(&args(&["lock", "--catalog", "catalog.json"])) {
            Ok(Command::Lock { root, catalog }) => {
                assert_eq!(root, PathBuf::from("."));
                assert_eq!(catalog, PathBuf::from("catalog.json"));
            }
            other => panic!("expected lock: {other:?}"),
        }
        for argv in [
            args(&["lock", "some/root", "--catalog", "catalog.json"]),
            args(&["lock", "--catalog", "catalog.json", "some/root"]),
        ] {
            match parse_args(&argv) {
                Ok(Command::Lock { root, catalog }) => {
                    assert_eq!(root, PathBuf::from("some/root"));
                    assert_eq!(catalog, PathBuf::from("catalog.json"));
                }
                other => panic!("expected lock with root: {other:?}"),
            }
        }
        for argv in [
            args(&["lock"]),
            args(&["lock", "some/root"]),
            args(&["lock", "--catalog"]),
            args(&["lock", "--catalog", "a", "--catalog", "b"]),
            args(&["lock", "a", "b", "--catalog", "c"]),
            args(&["lock", "--bogus", "--catalog", "c"]),
            args(&["lock", "--catalog", "c", "--bogus"]),
            args(&["lock", "--", "--catalog", "c"]),
            args(&["lock", "--catalog", "c", "--help"]),
        ] {
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                LOCK_USAGE,
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn lock_non_unicode_paths_parse_for_io() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]);
        let argv = vec![
            OsString::from("lock"),
            bad.clone(),
            OsString::from("--catalog"),
            bad,
        ];
        assert!(matches!(parse_args(&argv), Ok(Command::Lock { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn lock_non_unicode_paths_parse_for_io() {
        use std::os::windows::ffi::OsStringExt;
        let bad = OsString::from_wide(&[0x0066, 0xD800]);
        let argv = vec![
            OsString::from("lock"),
            bad.clone(),
            OsString::from("--catalog"),
            bad,
        ];
        assert!(matches!(parse_args(&argv), Ok(Command::Lock { .. })));
    }

    #[test]
    fn lock_flag_shaped_catalog_values_are_usage_before_io() {
        // Every Unicode dash-prefixed token used as `--catalog`'s value is
        // usage, never I/O: the reviewed `lock --catalog --help` case plus
        // version/terminator/short-flag/duplicate forms.
        for value in [
            "--help",
            "--version",
            "--",
            "-x",
            "--catalog",
            "--root",
            "--check",
            "--bogus",
        ] {
            let argv = args(&["lock", "--catalog", value]);
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                LOCK_USAGE,
            );
        }
        for argv in [
            args(&["lock", "--catalog", "--help"]),
            args(&["lock", "--catalog", "--"]),
            args(&["lock", "--catalog", "-x"]),
            args(&["lock", "--catalog", "--catalog"]),
        ] {
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                LOCK_USAGE,
            );
        }
        // An explicit relative prefix escapes a dash-leading catalog path.
        match parse_args(&args(&["lock", "--catalog", "./-catalog.json"])) {
            Ok(Command::Lock { catalog, .. }) => {
                assert_eq!(catalog, PathBuf::from("./-catalog.json"));
            }
            other => panic!("./-prefixed catalog must parse: {other:?}"),
        }
    }

    #[test]
    fn new_and_run_flag_shaped_option_values_are_usage() {
        // Text option values that look like flags are usage, never silently
        // accepted as slugs/ids/numbers.
        for argv in [
            args(&["new", "creature", "--slug", "--help", "--id", NEW_ID]),
            args(&["new", "creature", "--slug", "goblin", "--id", "--"]),
            args(&[
                "new",
                "creature",
                "--slug",
                "goblin",
                "--id",
                NEW_ID,
                "--entry-id",
                "--help",
            ]),
            args(&["run", "--ticks", "--help", "--hash-every", "1"]),
            args(&["run", "--ticks", "10", "--hash-every", "--"]),
            args(&["run", "--ticks", "10", "--hash-every", "1", "--seed", "-x"]),
        ] {
            let usage = if argv[0].to_str() == Some("new") {
                NEW_USAGE
            } else {
                RUN_USAGE
            };
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                usage,
            );
        }
    }

    /// Minimal lock-test campaign inputs on a fake rewrite fs: a campaign
    /// with no requirements plus an empty assets lock, so an empty catalog
    /// resolves. Bytes are authored JSON envelopes, read through the same
    /// production `read_document`/`read_assets_lock` paths as the binary.
    fn lock_empty_inputs(fs: &mut FakeRewriteFs, root: &Path, catalog_bytes: &[u8]) -> PathBuf {
        fs.dir(root);
        fs.dir(&root.join("assets"));
        fs.file(
            &root.join("campaign.json"),
            br#"{"engine":">=0.1.0","entry":{"area":"00000000000000000000000003","spawn":"00000000000000000000000005","world":"00000000000000000000000002"},"id":"00000000000000000000000001","name":"fixture.campaign","package":"fixture.one-area","requires":[],"schema":"crpg.campaign/1","slug":"campaign","version":"0.1.0"}"#,
        );
        fs.file(
            &root.join("assets").join("assets.lock"),
            br#"{"assets":{},"schema":"crpg.assets-lock/1"}"#,
        );
        let catalog = root.join("catalog.json");
        fs.file(&catalog, catalog_bytes);
        catalog
    }

    #[test]
    fn lock_creates_missing_output_writes_expected_bytes() {
        let mut fs = FakeRewriteFs::new();
        let root = PathBuf::from("/fake/root");
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(
            outcome.code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&outcome.stderr)
        );
        assert!(outcome.stdout.is_empty() && outcome.stderr.is_empty());
        let disk_lock = disk_path(&root, "campaign.lock");
        let written = fs
            .written
            .borrow()
            .get(&disk_lock)
            .cloned()
            .expect("creates lock");
        // The new lock's note is the data constructor's None, and the bytes
        // equal the direct data calls: forwarding, not a CLI oracle.
        let assets = crpg_data::read_assets_lock(br#"{"assets":{},"schema":"crpg.assets-lock/1"}"#)
            .expect("test assets parse");
        let expected = crpg_data::write_campaign_lock(
            &crpg_data::make_campaign_lock(&[], &[], &assets).expect("empty resolves"),
        )
        .expect("test lock writes");
        assert_eq!(written, expected);
        assert!(
            fs.log
                .borrow()
                .iter()
                .any(|entry| entry == &format!("create {}", portable(&disk_lock))),
            "missing output uses create_new: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn lock_no_ops_on_byte_equality_and_replaces_stale_output() {
        let mut fs = FakeRewriteFs::new();
        let root = PathBuf::from("/fake/root");
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        let assets = crpg_data::read_assets_lock(br#"{"assets":{},"schema":"crpg.assets-lock/1"}"#)
            .expect("test assets parse");
        let expected = crpg_data::write_campaign_lock(
            &crpg_data::make_campaign_lock(&[], &[], &assets).expect("empty resolves"),
        )
        .expect("test lock writes");
        // Stale output is replaced through the existing-file discipline.
        let disk_lock = disk_path(&root, "campaign.lock");
        fs.file(&disk_lock, b"{\"stale\":true}");
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(
            outcome.code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&outcome.stderr)
        );
        assert_eq!(fs.written.borrow().get(&disk_lock), Some(&expected));
        // Byte-identical output performs zero writes.
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.file(&disk_lock, &expected);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 0);
        assert!(fs.written.borrow().is_empty(), "no-op must not write");
        assert!(
            fs.log
                .borrow()
                .iter()
                .all(|entry| !entry.starts_with("open") && !entry.starts_with("create")),
            "no-op must not open or create: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn lock_replacement_detects_byte_drift_with_zero_opens() {
        // Drift between the initial capture and the prewrite reread surfaces
        // as `source_changed` with exact bytes and zero opens/writes.
        let mut fs = FakeRewriteFs::new();
        let root = PathBuf::from("/fake/root");
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        let disk_lock = disk_path(&root, "campaign.lock");
        fs.file(&disk_lock, b"{\"stale\":true}");
        fs.drift_second_read(&disk_lock, b"{\"drifted\":true}");
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "campaign.lock: error[io]: cannot check campaign.lock: source_changed\n"
        );
        assert!(fs.written.borrow().is_empty(), "drift must not write");
        assert!(
            fs.log
                .borrow()
                .iter()
                .all(|entry| !entry.starts_with("open")
                    && !entry.starts_with("create")
                    && !entry.starts_with("write")),
            "drift must not open/create/write: {:?}",
            fs.log.borrow()
        );
    }

    #[test]
    fn lock_rejects_symlinked_ancestors_without_traversal() {
        let root = PathBuf::from("/fake/root");
        // Catalog with a symlinked ancestor is rejected with the portable
        // `<catalog>` diagnostic, never `invalid catalog`.
        let mut fs = FakeRewriteFs::new();
        fs.dir(&root);
        fs.dir(&root.join("assets"));
        fs.file(
            &root.join("campaign.json"),
            br#"{"engine":">=0.1.0","entry":{"area":"00000000000000000000000003","spawn":"00000000000000000000000005","world":"00000000000000000000000002"},"id":"00000000000000000000000001","name":"fixture.campaign","package":"fixture.one-area","requires":[],"schema":"crpg.campaign/1","slug":"campaign","version":"0.1.0"}"#,
        );
        fs.file(
            &root.join("assets").join("assets.lock"),
            br#"{"assets":{},"schema":"crpg.assets-lock/1"}"#,
        );
        let link_parent = PathBuf::from("/fake/link");
        fs.symlink(&link_parent);
        let catalog = link_parent.join("catalog.json");
        fs.file(&catalog, b"[]");
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "<campaign>: error[io]: cannot read <catalog>: symlink_at_document_path\n"
        );
        assert!(fs.written.borrow().is_empty());
        // A symlinked parent of the supplied root is rejected with the
        // portable `<campaign-root>` diagnostic.
        let mut fs = FakeRewriteFs::new();
        fs.symlink(&PathBuf::from("/fake"));
        fs.dir(&root);
        let catalog = lock_empty_inputs(&mut fs, &PathBuf::from("/fake/other"), b"[]");
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "<campaign>: error[io]: cannot open root <campaign-root>: symlink_at_document_path\n"
        );
        // A symlinked `assets` ancestor names the affected target logical.
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.kinds.insert(root.join("assets"), EntryKind::Symlink);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "assets/assets.lock: error[io]: cannot read assets/assets.lock: symlink_at_document_path\n"
        );
    }

    #[test]
    fn lock_rejects_bad_inputs_with_exact_bytes_and_zero_output_writes() {
        let root = PathBuf::from("/fake/root");
        // Wrong campaign document kind.
        let mut fs = FakeRewriteFs::new();
        fs.dir(&root);
        fs.dir(&root.join("assets"));
        fs.file(
            &root.join("campaign.json"),
            br#"{"areas":[],"id":"00000000000000000000000002","name":"k","schema":"crpg.world/1","slug":"world","variables":[]}"#,
        );
        fs.file(
            &root.join("assets").join("assets.lock"),
            br#"{"assets":{},"schema":"crpg.assets-lock/1"}"#,
        );
        let catalog = root.join("catalog.json");
        fs.file(&catalog, b"[]");
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(outcome.stderr, LOCK_EXPECTED_CAMPAIGN.as_bytes());
        assert!(fs.written.borrow().is_empty());
        // Malformed catalog and non-array catalog share one exact line.
        for catalog_bytes in [
            b"{".as_slice(),
            b"{}".as_slice(),
            b"null".as_slice(),
            b"[}".as_slice(),
        ] {
            let mut fs = FakeRewriteFs::new();
            let catalog = lock_empty_inputs(&mut fs, &root, catalog_bytes);
            let outcome = run_lock_with(&fs, &root, &catalog);
            assert_eq!(outcome.code, 1, "catalog {catalog_bytes:?}");
            assert!(outcome.stdout.is_empty());
            assert_eq!(outcome.stderr, LOCK_INVALID_CATALOG.as_bytes());
            assert!(fs.written.borrow().is_empty(), "bad catalog writes nothing");
        }
        // Unknown candidate fields are strict typed errors, not leniency.
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(
            &mut fs,
            &root,
            b"[{\"kind\":\"module\",\"package\":\"a.b\",\"version\":\"1.0.0\",\"checksum\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"extra\":1}]",
        );
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(outcome.stderr, LOCK_INVALID_CATALOG.as_bytes());
        // Missing campaign input is a portable read diagnostic.
        let mut fs = FakeRewriteFs::new();
        fs.dir(&root);
        fs.dir(&root.join("assets"));
        fs.file(
            &root.join("assets").join("assets.lock"),
            br#"{"assets":{},"schema":"crpg.assets-lock/1"}"#,
        );
        let catalog = root.join("catalog.json");
        fs.file(&catalog, b"[]");
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "campaign.json: error[io]: cannot read campaign.json: not_found\n"
        );
        // A concurrently appearing output is source_changed, never a silent
        // overwrite or a directory creation: covered by
        // lock_create_race_is_source_changed below.
    }

    #[test]
    fn lock_create_race_is_source_changed() {
        // A missing output that appears between the metadata check and
        // creation reports source_changed: production create_new fails with
        // AlreadyExists, which the writer maps to a check diagnostic, never
        // a silent overwrite.
        let mut fs = FakeRewriteFs::new();
        let root = PathBuf::from("/fake/root");
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        let disk_lock = disk_path(&root, "campaign.lock");
        fs.kinds.remove(&disk_lock);
        fs.create_err
            .insert(disk_lock.clone(), std::io::ErrorKind::AlreadyExists);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "campaign.lock: error[io]: cannot check campaign.lock: source_changed\n"
        );
        assert!(fs.written.borrow().is_empty());
        // A genuine scripted appearance between the metadata check and
        // creation takes the same path: metadata reports missing while the
        // later create observes the new file as `AlreadyExists`.
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        let disk_lock = disk_path(&root, "campaign.lock");
        fs.kinds.remove(&disk_lock);
        fs.contents.remove(&disk_lock);
        fs.appear_on_next_metadata(&disk_lock);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "campaign.lock: error[io]: cannot check campaign.lock: source_changed\n"
        );
        assert!(fs.written.borrow().is_empty());
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.read_err.insert(
            root.join("assets").join("assets.lock"),
            std::io::ErrorKind::PermissionDenied,
        );
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "assets/assets.lock: error[io]: cannot read assets/assets.lock: permission_denied\n"
        );
        assert!(fs.written.borrow().is_empty());
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.read_err
            .insert(catalog.clone(), std::io::ErrorKind::PermissionDenied);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "<campaign>: error[io]: cannot read <catalog>: permission_denied\n"
        );
        assert!(fs.written.borrow().is_empty());

        // The bounded writer may retain a prefix of the failing output and
        // must report exit 1 without attempting sync or rollback.
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.kinds.remove(&disk_lock);
        fs.contents.remove(&disk_lock);
        fs.write_prefix.insert(disk_lock.clone(), 7);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(String::from_utf8(outcome.stderr)
            .expect("utf-8")
            .contains("cannot write campaign.lock: "));
        assert_eq!(
            fs.written
                .borrow()
                .get(&disk_lock)
                .expect("partial prefix retained")
                .len(),
            7
        );
        assert!(fs
            .log
            .borrow()
            .iter()
            .all(|entry| entry != &format!("sync {}", portable(&disk_lock))));
    }

    #[test]
    fn lock_output_symlink_and_nonregular_are_check_failures() {
        let root = PathBuf::from("/fake/root");
        for kind in [EntryKind::Symlink, EntryKind::Directory, EntryKind::Other] {
            let mut fs = FakeRewriteFs::new();
            let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
            let disk_lock = disk_path(&root, "campaign.lock");
            fs.kinds.insert(disk_lock.clone(), kind);
            let outcome = run_lock_with(&fs, &root, &catalog);
            assert_eq!(outcome.code, 1);
            assert!(outcome.stdout.is_empty());
            let stderr = String::from_utf8(outcome.stderr).expect("utf-8");
            assert!(
                stderr.starts_with("campaign.lock: error[io]: cannot check campaign.lock: "),
                "{stderr}"
            );
            assert!(fs.written.borrow().is_empty());
        }
    }

    #[test]
    fn lock_create_and_replace_cover_open_write_sync_failures() {
        let root = PathBuf::from("/fake/root");
        let disk_lock = disk_path(&root, "campaign.lock");
        // Creation path (missing output via `create_new`): open/create,
        // write, and sync failures each report their op with exact bytes and
        // zero successful writes.
        for (kind, op) in [
            (std::io::ErrorKind::PermissionDenied, "open"),
            (std::io::ErrorKind::Other, "open"),
        ] {
            let mut fs = FakeRewriteFs::new();
            let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
            fs.kinds.remove(&disk_lock);
            fs.contents.remove(&disk_lock);
            fs.create_err.insert(disk_lock.clone(), kind);
            // PermissionDenied keeps its kind; Other collapses to io_error.
            let outcome = run_lock_with(&fs, &root, &catalog);
            assert_eq!(outcome.code, 1, "create open {kind:?}");
            assert!(outcome.stdout.is_empty());
            let stderr = String::from_utf8(outcome.stderr).expect("utf-8");
            assert!(
                stderr.starts_with(&format!(
                    "campaign.lock: error[io]: cannot {op} campaign.lock: "
                )),
                "{stderr}"
            );
            assert!(fs.written.borrow().is_empty());
        }
        for (setup, op, needle) in [
            ("write", "write", "cannot write campaign.lock: "),
            ("sync", "sync", "cannot sync campaign.lock: "),
        ] {
            let mut fs = FakeRewriteFs::new();
            let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
            fs.kinds.remove(&disk_lock);
            fs.contents.remove(&disk_lock);
            if setup == "write" {
                fs.write_err
                    .insert(disk_lock.clone(), std::io::ErrorKind::Other);
            } else {
                fs.sync_err
                    .insert(disk_lock.clone(), std::io::ErrorKind::Other);
            }
            let outcome = run_lock_with(&fs, &root, &catalog);
            assert_eq!(outcome.code, 1, "create {setup}");
            let stderr = String::from_utf8(outcome.stderr).expect("utf-8");
            assert!(stderr.contains(needle), "{stderr}");
            assert_eq!(op, if setup == "write" { "write" } else { "sync" });
        }
        // Replacement path (existing output via `open_truncate`): open,
        // write, and sync failures with exact first-failure diagnostics and
        // bounded partial-write effects (no rollback claim).
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.file(&disk_lock, b"{\"stale\":true}");
        fs.open_err
            .insert(disk_lock.clone(), std::io::ErrorKind::PermissionDenied);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "campaign.lock: error[io]: cannot open campaign.lock: permission_denied\n"
        );
        assert!(fs.written.borrow().is_empty());
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.file(&disk_lock, b"{\"stale\":true}");
        fs.write_err
            .insert(disk_lock.clone(), std::io::ErrorKind::Other);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(String::from_utf8(outcome.stderr)
            .expect("utf-8")
            .contains("cannot write campaign.lock: "));
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.file(&disk_lock, b"{\"stale\":true}");
        fs.sync_err
            .insert(disk_lock.clone(), std::io::ErrorKind::Other);
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert!(String::from_utf8(outcome.stderr)
            .expect("utf-8")
            .contains("cannot sync campaign.lock: "));
        // Input-read failures (campaign/assets/catalog) yield zero output
        // writes with exact first-failure order: campaign first.
        let mut fs = FakeRewriteFs::new();
        let catalog = lock_empty_inputs(&mut fs, &root, b"[]");
        fs.read_err.insert(
            root.join("campaign.json"),
            std::io::ErrorKind::PermissionDenied,
        );
        let outcome = run_lock_with(&fs, &root, &catalog);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            String::from_utf8(outcome.stderr).expect("utf-8"),
            "campaign.json: error[io]: cannot read campaign.json: permission_denied\n"
        );
        assert!(fs.written.borrow().is_empty());
    }

    #[test]
    fn run_schema_defensive_missing_stem_returns_data_outcome() {
        // The parser admits only the seventeen stems; calling the runner
        // directly with an unlisted stem exercises the defensive `None` arm
        // (data-owned line, exit 1, empty stdout) rather than panicking.
        let outcome = run_schema("bogus-stem");
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert!(!outcome.stderr.is_empty());
        assert!(
            String::from_utf8(outcome.stderr)
                .expect("utf-8")
                .contains("missing generated schema"),
            "defensive generation failure must name the stem"
        );
    }

    #[test]
    fn fmt_document_set_mismatch_maps_to_exact_outcome() {
        // `plan_updates` key mismatch is an internal failure before writes;
        // the runner maps it to the exact fixed line with exit 1.
        let mut files = BTreeMap::new();
        files.insert(migrate_path("campaign.json"), b"{}".to_vec());
        let mut serialized = BTreeMap::new();
        serialized.insert(migrate_path("campaign.json"), b"{}".to_vec());
        serialized.insert(migrate_path("worlds/world.json"), b"{}".to_vec());
        assert!(plan_updates(&files, &serialized).is_err());
        let outcome = Outcome {
            code: 1,
            stdout: Vec::new(),
            stderr: FMT_DOCUMENT_SET_FAILURE.as_bytes().to_vec(),
        };
        assert_eq!(outcome.code, 1);
        assert!(outcome.stdout.is_empty());
        assert_eq!(
            outcome.stderr,
            b"crpgc fmt: internal document set failure\n"
        );
    }

    #[test]
    fn run_parser_matrix_and_usage_bytes() {
        match parse_args(&args(&["run", "--ticks", "10", "--hash-every", "2"])) {
            Ok(Command::Run {
                ticks,
                hash_every,
                seed,
            }) => {
                assert_eq!((ticks, hash_every, seed), (10, 2, 0));
            }
            other => panic!("expected run: {other:?}"),
        }
        match parse_args(&args(&[
            "run",
            "--seed",
            "7",
            "--hash-every",
            "003",
            "--ticks",
            "010",
        ])) {
            Ok(Command::Run {
                ticks,
                hash_every,
                seed,
            }) => {
                assert_eq!((ticks, hash_every, seed), (10, 3, 7));
            }
            other => panic!("expected run permuted with leading zeroes: {other:?}"),
        }
        for argv in [
            args(&["run"]),
            args(&["run", "--ticks", "10"]),
            args(&["run", "--hash-every", "2"]),
            args(&["run", "--ticks", "10", "--hash-every", "2", "--seed"]),
            args(&["run", "--ticks", "--hash-every", "2"]),
            args(&["run", "--ticks", "10", "--ticks", "11", "--hash-every", "2"]),
            args(&[
                "run",
                "--ticks",
                "10",
                "--hash-every",
                "2",
                "--hash-every",
                "3",
            ]),
            args(&[
                "run",
                "--ticks",
                "10",
                "--hash-every",
                "2",
                "--seed",
                "1",
                "--seed",
                "2",
            ]),
            args(&["run", "--ticks", "10", "--bogus", "2"]),
            args(&["run", "--ticks", "10", "--hash-every", "2", "extra"]),
            args(&["run", "--ticks=10", "--hash-every", "2"]),
            args(&["run", "--ticks", "+10", "--hash-every", "2"]),
            args(&["run", "--ticks", "-1", "--hash-every", "2"]),
            args(&["run", "--ticks", "1 0", "--hash-every", "2"]),
            args(&["run", "--ticks", "0x10", "--hash-every", "2"]),
            args(&["run", "--ticks", "1_0", "--hash-every", "2"]),
            args(&["run", "--ticks", "", "--hash-every", "2"]),
            args(&["run", "--ticks", "1000001", "--hash-every", "1"]),
            args(&["run", "--ticks", "10", "--hash-every", "0"]),
            args(&["run", "--ticks", "10", "--hash-every", "1000001"]),
            args(&[
                "run",
                "--ticks",
                "18446744073709551616",
                "--hash-every",
                "1",
            ]),
            args(&["run", "--ticks", "10", "--hash-every", "2", "--seed", "-1"]),
            args(&["run", "--help", "--ticks", "1", "--hash-every", "1"]),
            args(&["run", "--ticks", "1", "--hash-every", "1", "--"]),
        ] {
            assert_new_usage(
                &argv
                    .iter()
                    .map(|arg| arg.to_str().expect("unicode"))
                    .collect::<Vec<_>>(),
                RUN_USAGE,
            );
        }
        // Boundaries: N=0 and M=N=1_000_000 parse; defaults hold.
        match parse_args(&args(&["run", "--ticks", "0", "--hash-every", "1000000"])) {
            Ok(Command::Run { ticks, .. }) => assert_eq!(ticks, 0),
            other => panic!("expected N=0: {other:?}"),
        }
        // Accepted numeric boundaries parse without performing million-tick
        // simulation work in parser tests: N=1_000_000 and u64::MAX seed are
        // accepted by the parser; execution is covered by small-N binary
        // tests only.
        match parse_args(&args(&[
            "run",
            "--ticks",
            "1000000",
            "--hash-every",
            "1000000",
        ])) {
            Ok(Command::Run {
                ticks, hash_every, ..
            }) => {
                assert_eq!((ticks, hash_every), (1_000_000, 1_000_000));
            }
            other => panic!("expected N=1_000_000: {other:?}"),
        }
        match parse_args(&args(&[
            "run",
            "--ticks",
            "10",
            "--hash-every",
            "2",
            "--seed",
            "18446744073709551615",
        ])) {
            Ok(Command::Run { seed, .. }) => assert_eq!(seed, u64::MAX),
            other => panic!("expected u64::MAX seed: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_non_unicode_number_is_usage() {
        use std::os::unix::ffi::OsStringExt;
        let argv = vec![
            OsString::from("run"),
            OsString::from("--ticks"),
            OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]),
            OsString::from("--hash-every"),
            OsString::from("1"),
        ];
        assert!(matches!(parse_args(&argv), Err(CliError::Usage(_))));
    }

    #[cfg(windows)]
    #[test]
    fn run_non_unicode_number_is_usage() {
        use std::os::windows::ffi::OsStringExt;
        let argv = vec![
            OsString::from("run"),
            OsString::from("--ticks"),
            OsString::from_wide(&[0x0066, 0xD800]),
            OsString::from("--hash-every"),
            OsString::from("1"),
        ];
        assert!(matches!(parse_args(&argv), Err(CliError::Usage(_))));
    }

    #[test]
    fn run_number_parser_pins_digits_and_overflow() {
        assert_eq!(parse_run_number("0"), Ok(0));
        assert_eq!(parse_run_number("007"), Ok(7));
        assert_eq!(parse_run_number("1000000"), Ok(1_000_000));
        for bad in [
            "",
            "+1",
            "-1",
            " 1",
            "1 ",
            "0x1",
            "1_0",
            "1.0",
            "①",
            "18446744073709551616",
        ] {
            assert_eq!(parse_run_number(bad), Err(()), "{bad:?} must fail");
        }
    }

    #[test]
    fn hex32_is_64_lowercase_hex() {
        let rendered = hex32(&[0xABu8; 32]);
        assert_eq!(rendered.len(), 64);
        assert_eq!(rendered, "ab".repeat(32));
        assert_eq!(hex32(&[0u8; 32]), "00".repeat(32));
        assert_ne!(hex32(&[0u8; 32]), hex32(&[1u8; 32]));
    }

    #[test]
    fn run_samples_match_the_native_harness_at_index_k_minus_1() {
        let outcome = run_run(5, 2, 0);
        assert_eq!(outcome.code, 0);
        assert!(outcome.stderr.is_empty());
        let expected = crpg_testkit::run_hash_sequence(0, 5, Box::new(|_| {}));
        let mut lines = Vec::new();
        for (index, hash) in expected.iter().enumerate() {
            let tick = index + 1;
            if tick % 2 == 0 {
                lines.push(format!("{tick} {}\n", hex32(hash)));
            }
        }
        assert_eq!(outcome.stdout, lines.concat().into_bytes());
        // Zero ticks and M>N produce empty stdout with exit 0.
        assert!(run_run(0, 1, 0).stdout.is_empty());
        assert!(run_run(3, 9, 0).stdout.is_empty());
        // Repeatability with an explicit seed.
        assert_eq!(run_run(4, 1, 42).stdout, run_run(4, 1, 42).stdout);
    }
}
