//! Display-independent entry points for native Nix configuration.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::Write as _,
    os::unix::fs::DirBuilderExt as _,
    path::{Path, PathBuf},
    process::ExitCode,
};

use directories::BaseDirs;
use mailstronaut_config::{helper, native, process, source};
use tokio::runtime;
use tokio_util::sync::CancellationToken;

/// An inert native template: initialization must never add real account logins.
const INITIAL_CONFIG: &str =
    "# Mailstronaut editable configuration\n{\n  schemaVersion = 1;\n  accounts = [ ];\n}\n";

/// Handle the finite CLI surface before toolkit or session initialization.
pub fn dispatch(arguments: &[OsString]) -> Option<ExitCode> {
    match arguments {
        [] => None,
        [flag, _path] if flag == OsStr::new("--config") => None,
        [flag] if flag == OsStr::new("--internal-evaluate-config") => Some(helper::run()),
        [flag] if flag == OsStr::new("--internal-evaluate-raw") => Some(helper::run_raw()),
        [flag] if flag == OsStr::new("--version") => {
            println!("mailstronaut {}", env!("CARGO_PKG_VERSION"));
            Some(ExitCode::SUCCESS)
        }
        [flag] if flag == OsStr::new("--help") => {
            println!(
                "Usage: mailstronaut [--demo | --config FILE | --version | config <init|check|export> [--path FILE] | config reconcile --declared FILE [--path FILE]]"
            );
            Some(ExitCode::SUCCESS)
        }
        [command, rest @ ..] if command == OsStr::new("config") => {
            match configuration_command(rest) {
                Ok(message) => {
                    print!("{message}");
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
fn configuration_command(arguments: &[OsString]) -> Result<String, String> {
    const USAGE: &str = "Usage: mailstronaut config <init|check|export> [--path FILE] | config reconcile --declared FILE [--path FILE]";
    let (command, declared, explicit_path) = match arguments {
        [command] => (command, None, None),
        [command, flag, path] if flag == OsStr::new("--path") => {
            (command, None, Some(PathBuf::from(path)))
        }
        [command, flag, declared] if flag == OsStr::new("--declared") => {
            (command, Some(PathBuf::from(declared)), None)
        }
        [command, first, declared, second, path]
            if first == OsStr::new("--declared") && second == OsStr::new("--path") =>
        {
            (
                command,
                Some(PathBuf::from(declared)),
                Some(PathBuf::from(path)),
            )
        }
        _ => return Err(USAGE.into()),
    };
    let selected_path = explicit_path
        .or_else(|| BaseDirs::new().map(|base| base.config_dir().join("mailstronaut/config.nix")));
    let path = selected_path.ok_or("unable to locate the configuration directory")?;
    if command == OsStr::new("init") && declared.is_none() {
        initialize(&path)?;
        return Ok("Created native Nix configuration.\n".into());
    }
    let executable = env::current_exe().map_err(|_error| "unable to locate this executable")?;
    let runtime = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_error| "unable to start configuration runtime")?;
    let cancellation = CancellationToken::new();
    if command == OsStr::new("reconcile") {
        let declared = declared.ok_or(USAGE)?;
        runtime.block_on(reconcile(&executable, &declared, &path, &cancellation))?;
        return Ok("Applied declared configuration.\n".into());
    }
    if declared.is_some() {
        return Err(USAGE.into());
    }
    let is_check = command == OsStr::new("check");
    if !is_check && command != OsStr::new("export") {
        return Err("Unknown configuration command. Use init, check, export or reconcile.".into());
    }
    let source = source::read(&path)
        .map_err(|error| error.to_string())?
        .ok_or("configuration does not exist")?;
    let validated = runtime
        .block_on(process::evaluate(&executable, &source, &cancellation))
        .map_err(|error| error.to_string())?;
    if is_check {
        return Ok("Configuration is valid.\n".into());
    }
    native::export(&validated).map_err(|_error| "unable to export configuration".into())
}

/// Apply Home Manager's declaration to the application's writable native file. The
/// merged result is validated in full before it replaces anything, so a bad declaration
/// leaves the last valid file in place.
async fn reconcile(
    executable: &Path,
    declared: &Path,
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let declared_source = source::read(declared)
        .map_err(|error| error.to_string())?
        .ok_or("declared configuration does not exist")?;
    let declared = process::evaluate_raw(executable, &declared_source, cancellation)
        .await
        .map_err(|error| error.to_string())?;
    if !declared.is_object() {
        return Err("declared configuration must be an attribute set".into());
    }
    let edit = source::Edit::begin(path).map_err(|error| error.to_string())?;
    check_ownership(path, edit.current())?;
    let mut merged = match edit.current() {
        Some(source) => process::evaluate_raw(executable, source, cancellation)
            .await
            .map_err(|error| format!("current configuration: {error}"))?,
        None => serde_json::json!({ "schemaVersion": 1 }),
    };
    if !merged.is_object() {
        return Err("current configuration must be an attribute set".into());
    }
    native::merge(&mut merged, declared);
    if !native::set_managed(&mut merged, false) {
        return Err("current configuration must be an attribute set".into());
    }
    let text = native::to_nix(&merged).map_err(|error| error.to_string())?;
    process::evaluate(executable, &text, cancellation)
        .await
        .map_err(|error| format!("merged configuration: {error}"))?;
    install(path, &text, &edit)
}

fn check_ownership(path: &Path, current: Option<&str>) -> Result<(), String> {
    let Some(current) = current else {
        return Ok(());
    };
    let metadata = fs::symlink_metadata(path).map_err(|_| "unable to inspect configuration")?;
    if metadata.file_type().is_symlink() {
        let target = fs::canonicalize(path).map_err(|_| "unable to inspect configuration")?;
        let module_link = target.parent() == Some(Path::new("/nix/store"))
            && target
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("-mailstronaut-config.nix"))
            && current.starts_with(native::MANAGED_HEADER);
        if !module_link {
            return Err("configuration is a link to another file; refusing to replace it".into());
        }
    } else if metadata.permissions().readonly() {
        return Err("configuration is read-only".into());
    } else if !current.starts_with(native::MUTABLE_HEADER) {
        return Err("configuration is user-authored; export and back it up before adopting an app-owned file".into());
    }
    Ok(())
}

/// Replace atomically with a private regular file; a concurrent writer cannot be
/// observed half-written, and a store link is replaced rather than written through.
fn install(path: &Path, text: &str, edit: &source::Edit) -> Result<(), String> {
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
        .write_all(native::MUTABLE_HEADER.as_bytes())
        .map_err(|_error| "unable to write configuration")?;
    temporary
        .write_all(text.as_bytes())
        .map_err(|_error| "unable to write configuration")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_error| "unable to sync configuration")?;
    edit.verify().map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map_err(|_error| "unable to install configuration")?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_error| "unable to sync configuration directory")?;
    Ok(())
}

/// Select the same native source for GUI startup and explicit CLI checks.
pub fn selected_path(arguments: &[OsString]) -> Option<PathBuf> {
    if let [flag, path] = arguments
        && flag == OsStr::new("--config")
    {
        return Some(PathBuf::from(path));
    }
    BaseDirs::new().map(|base| base.config_dir().join("mailstronaut/config.nix"))
}

/// Install a private native template atomically, without overwriting any path.
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
