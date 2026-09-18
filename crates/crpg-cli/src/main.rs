#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! `crpgc` — the campaign toolchain CLI: validate, migrate, replay, and the
//! later pack, run and diff subcommands. No Godot, no rendering. T009b
//! shipped the `replay` subcommand; T011b adds the thin `validate` wrapper
//! over `crpg-data` validation; T012b adds the thin `migrate` explicit-save
//! wrapper over T012a migration-aware loading and the canonical writer.
//! T013 still owns the parser-framework decision, so all subcommands extend
//! the same hand-rolled `args_os` parser.

mod apply;

use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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

/// Exact stderr bytes when the compile-time engine version does not parse as
/// semver for `migrate`. A release-process bug, never user input.
const MIGRATE_ENGINE_FAILURE: &str = "crpgc migrate: internal engine version failure\n";

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
/// T011b owns `Validate`, T012b owns `Migrate`.
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

/// Production filesystem access: `symlink_metadata` (never follows links),
/// unordered directory names (the walker sorts), and plain file reads.
struct RealFs;

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
    if root.to_str().is_none() {
        return Err(io_diagnostic(
            "open root",
            None,
            "<campaign-root>",
            IoKind::NonUnicodeComponent,
        ));
    }
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
    collect_into(fs, root, "", &mut files)?;
    Ok(files)
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
            collect_into(fs, &disk_child, &logical, files)?;
        } else {
            collect_file_entry(fs, &disk_child, &logical, kind, files)?;
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
) -> Result<(), Diagnostic> {
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
    let files = match collect_campaign_files_with(walk_fs, root) {
        Ok(files) => files,
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
    if stdout_ok && stderr_ok {
        outcome.code
    } else {
        1
    }
}

/// Runs a parsed command. Replay stays the thin testkit consumer it was at
/// T009b; validate joins data-owned validation to OS-owned traversal;
/// migrate joins the same collector to T012a's migration-aware loader and
/// canonical writer plus its explicit save phase.
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
            Command::Validate { .. } | Command::Migrate { .. } => panic!("expected replay"),
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
            Command::Validate { .. } | Command::Migrate { .. } => panic!("expected replay"),
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

        fn dir(&mut self, path: &Path, children: &[&str]) {
            self.kinds.insert(path.to_path_buf(), EntryKind::Directory);
            self.lists.insert(
                path.to_path_buf(),
                children.iter().map(OsString::from).collect(),
            );
        }

        fn file(&mut self, path: &Path, bytes: &[u8]) {
            self.kinds.insert(path.to_path_buf(), EntryKind::File);
            self.contents.insert(path.to_path_buf(), bytes.to_vec());
            if let Some(parent) = path.parent() {
                self.kinds
                    .entry(parent.to_path_buf())
                    .or_insert(EntryKind::Directory);
            }
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
        write_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        sync_err: BTreeMap<PathBuf, std::io::ErrorKind>,
        log: std::rc::Rc<RefCell<Vec<String>>>,
        written: std::rc::Rc<RefCell<BTreeMap<PathBuf, Vec<u8>>>>,
    }

    /// Injected failing handle for the rewrite seam. Logs its own write/sync
    /// calls and optionally fails each stage with a stable `ErrorKind`.
    struct FakeHandle {
        path: PathBuf,
        write_err: Option<std::io::ErrorKind>,
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
                write_err: BTreeMap::new(),
                sync_err: BTreeMap::new(),
                log: std::rc::Rc::new(RefCell::new(Vec::new())),
                written: std::rc::Rc::new(RefCell::new(BTreeMap::new())),
            }
        }

        fn dir(&mut self, path: &Path) {
            self.kinds.insert(path.to_path_buf(), EntryKind::Directory);
        }

        fn file(&mut self, path: &Path, bytes: &[u8]) {
            self.kinds.insert(path.to_path_buf(), EntryKind::File);
            self.contents.insert(path.to_path_buf(), bytes.to_vec());
        }

        fn symlink(&mut self, path: &Path) {
            self.kinds.insert(path.to_path_buf(), EntryKind::Symlink);
        }

        fn other(&mut self, path: &Path) {
            self.kinds.insert(path.to_path_buf(), EntryKind::Other);
        }
    }

    impl RewriteFs for FakeRewriteFs {
        fn metadata(&self, path: &Path) -> std::io::Result<EntryKind> {
            self.log
                .borrow_mut()
                .push(format!("metadata {}", portable(path)));
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
                sync_err: self.sync_err.get(path).copied(),
                log: std::rc::Rc::clone(&self.log),
                written: std::rc::Rc::clone(&self.written),
            }))
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
    }
}
