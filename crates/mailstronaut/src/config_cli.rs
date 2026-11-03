//! Display-independent entry points for native Nix configuration.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::ExitCode,
};

use directories::BaseDirs;
use mailstronaut_config::{helper, native, persistence, process, source};
use tokio::runtime;
use tokio_util::sync::CancellationToken;

/// Handle the finite CLI surface before toolkit or session initialization.
pub fn dispatch(arguments: &[OsString]) -> Option<ExitCode> {
    match arguments {
        [] => None,
        [flag] if flag == OsStr::new("--background") => None,
        [flag, _path] if flag == OsStr::new("--config") => None,
        [first, second, _path]
            if first == OsStr::new("--background") && second == OsStr::new("--config") =>
        {
            None
        }
        [flag] if flag == OsStr::new("--internal-evaluate-config") => Some(helper::run()),
        [flag] if flag == OsStr::new("--internal-evaluate-raw") => Some(helper::run_raw()),
        [flag] if flag == OsStr::new("--version") => {
            println!("mailstronaut {}", env!("CARGO_PKG_VERSION"));
            Some(ExitCode::SUCCESS)
        }
        [flag] if flag == OsStr::new("--help") => {
            println!(
                "Usage: mailstronaut [--demo | [--background] [--config FILE] | --version | config <init|check|export> [--path FILE] | config reconcile --declared FILE [--path FILE]]"
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
        persistence::initialize(&path).map_err(|error| error.to_string())?;
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
        runtime
            .block_on(persistence::reconcile(
                &executable,
                &declared,
                &path,
                &cancellation,
            ))
            .map_err(|error| error.to_string())?;
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

/// Select the same native source for GUI startup and explicit CLI checks.
pub fn selected_path(arguments: &[OsString]) -> Option<PathBuf> {
    if let [flag, path] = arguments
        && flag == OsStr::new("--config")
    {
        return Some(PathBuf::from(path));
    }
    if let [first, second, path] = arguments
        && first == OsStr::new("--background")
        && second == OsStr::new("--config")
    {
        return Some(PathBuf::from(path));
    }
    BaseDirs::new().map(|base| base.config_dir().join("mailstronaut/config.nix"))
}
