//! Display-independent entry points for native Nix configuration.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{Read as _, Write as _},
    os::unix::fs::DirBuilderExt as _,
    path::{Path, PathBuf},
    process::ExitCode,
};

use directories::BaseDirs;
use mailstronaut_config::{MAX_SOURCE_BYTES, helper, process};
use tokio::runtime;
use tokio_util::sync::CancellationToken;

/// An inert native template: initialization must never add real account logins.
const INITIAL_CONFIG: &str = "{\n  schemaVersion = 1;\n  accounts = [ ];\n}\n";

/// Handle the finite CLI surface before toolkit or session initialization.
#[expect(
    clippy::implicit_return,
    clippy::pattern_type_mismatch,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::single_call_fn,
    reason = "CLI dispatch uses slice patterns, tail expressions and terminal output at its boundary."
)]
pub fn dispatch(arguments: &[OsString]) -> Option<ExitCode> {
    match arguments {
        [] => None,
        [flag] if flag == OsStr::new("--internal-evaluate-config") => Some(helper::run()),
        [flag] if flag == OsStr::new("--version") => {
            println!("mailstronaut {}", env!("CARGO_PKG_VERSION"));
            Some(ExitCode::SUCCESS)
        }
        [flag] if flag == OsStr::new("--help") => {
            println!("Usage: mailstronaut [--version | config <init|check> [--path FILE]]");
            Some(ExitCode::SUCCESS)
        }
        [command, rest @ ..] if command == OsStr::new("config") => {
            match configuration_command(rest) {
                Ok(message) => {
                    println!("{message}");
                    Some(ExitCode::SUCCESS)
                }
                Err(message) => {
                    eprintln!("{message}");
                    Some(ExitCode::FAILURE)
                }
            }
        }
        _ => {
            eprintln!("Unknown arguments. Use --help for usage.");
            Some(ExitCode::from(2))
        }
    }
}

/// Select a command and optional explicit file without interpreting user text.
#[expect(
    clippy::implicit_return,
    clippy::pattern_type_mismatch,
    clippy::question_mark_used,
    clippy::single_call_fn,
    reason = "Slice patterns and concise propagation keep redacted CLI errors readable."
)]
fn configuration_command(arguments: &[OsString]) -> Result<&'static str, String> {
    let (command, explicit_path) = match arguments {
        [command] => (command, None),
        [command, flag, path] if flag == OsStr::new("--path") => {
            (command, Some(PathBuf::from(path)))
        }
        _ => return Err("Usage: mailstronaut config <init|check> [--path FILE]".into()),
    };
    if command != OsStr::new("init") && command != OsStr::new("check") {
        return Err("Unknown configuration command. Use init or check.".into());
    }
    let selected_path = explicit_path
        .or_else(|| BaseDirs::new().map(|base| base.config_dir().join("mailstronaut/config.nix")));
    let path = selected_path.ok_or("unable to locate the configuration directory")?;
    if command == OsStr::new("init") {
        initialize(&path)?;
        return Ok("Created native Nix configuration.");
    }
    let source = read_source(&path)?;
    let executable = env::current_exe().map_err(|_error| "unable to locate this executable")?;
    let runtime = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_error| "unable to start configuration runtime")?;
    let cancellation = CancellationToken::new();
    let _validated = runtime
        .block_on(process::evaluate(&executable, &source, &cancellation))
        .map_err(|error| error.to_string())?;
    Ok("Configuration is valid.")
}

/// Read at most the documented source bound, following managed-file symlinks.
#[expect(
    clippy::implicit_return,
    clippy::question_mark_used,
    clippy::single_call_fn,
    reason = "Concise propagation preserves redacted file errors at this boundary."
)]
fn read_source(path: &Path) -> Result<String, String> {
    if !fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return Err("configuration must be an existing regular file".into());
    }
    let file = fs::File::open(path).map_err(|_error| "unable to read configuration")?;
    let limit = u64::try_from(MAX_SOURCE_BYTES)
        .map_err(|_error| "invalid source limit")?
        .saturating_add(1);
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_error| "unable to read configuration")?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("configuration exceeds the source size limit".into());
    }
    String::from_utf8(bytes).map_err(|_error| "configuration must be UTF-8".into())
}

/// Install a private native template atomically, without overwriting any path.
#[expect(
    clippy::implicit_return,
    clippy::question_mark_used,
    clippy::single_call_fn,
    reason = "Concise propagation preserves redacted installation errors at this boundary."
)]
fn initialize(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|_error| "unable to create configuration directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_error| "unable to create configuration")?;
    temporary
        .write_all(INITIAL_CONFIG.as_bytes())
        .map_err(|_error| "unable to write configuration")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_error| "unable to sync configuration")?;
    let _persisted = temporary
        .persist_noclobber(path)
        .map_err(|_error| "configuration already exists or could not be installed")?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_error| "unable to sync configuration directory")?;
    Ok(())
}
