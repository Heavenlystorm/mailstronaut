//! Real-executable configuration tests, with synthetic sources only.

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::{
        ffi::OsStr,
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
        path::{Path, PathBuf},
        process::{Command, Output, Stdio},
        time::Instant,
    };

    use mailstronaut_config::{MAX_SOURCE_BYTES, process};
    use tokio::time::sleep;
    use tokio_util::sync::CancellationToken;
    use wait_timeout::ChildExt as _;

    /// The application built by Cargo, never a shell or an executable from PATH.
    const EXECUTABLE: &str = env!("CARGO_BIN_EXE_mailstronaut");
    /// A tail-recursive pure expression with no result.
    const NONTERMINATING: &str = "let f = x: f x; in f 0";

    /// Give CLI tests an outer watchdog independent of the configuration timeout.
    #[expect(
        clippy::implicit_return,
        clippy::unwrap_used,
        clippy::panic,
        reason = "Fixture setup and watchdog failures must fail the test immediately."
    )]
    fn command(arguments: &[&OsStr]) -> Output {
        let mut child = Command::new(EXECUTABLE)
            .args(arguments)
            .env("PATH", "")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("XDG_RUNTIME_DIR")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if child
            .wait_timeout(Duration::from_secs(8))
            .unwrap()
            .is_none()
        {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("configuration CLI exceeded its independent watchdog");
        }
        child.wait_with_output().unwrap()
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "Fixture setup failures must fail the test immediately."
    )]
    fn native_cli_initialization_and_managed_symlinks_are_non_destructive() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile/config with spaces.nix");
        let arguments = [
            "config".as_ref(),
            "init".as_ref(),
            "--path".as_ref(),
            path.as_os_str(),
        ];
        assert!(command(&arguments).status.success());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let original = fs::read(&path).unwrap();
        assert!(!command(&arguments).status.success());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(
            command(&[
                "config".as_ref(),
                "check".as_ref(),
                "--path".as_ref(),
                path.as_os_str()
            ])
            .status
            .success()
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        let managed = directory.path().join("managed.nix");
        fs::write(
            &managed,
            "{ schemaVersion = 1; managed = true; accounts = []; }",
        )
        .unwrap();
        let linked = directory.path().join("linked.nix");
        symlink(&managed, &linked).unwrap();
        assert!(
            !command(&[
                "config".as_ref(),
                "init".as_ref(),
                "--path".as_ref(),
                linked.as_os_str()
            ])
            .status
            .success()
        );
        assert!(
            command(&[
                "config".as_ref(),
                "check".as_ref(),
                "--path".as_ref(),
                linked.as_os_str()
            ])
            .status
            .success()
        );
        assert!(linked.is_symlink());
        assert_eq!(
            fs::read_to_string(managed).unwrap(),
            "{ schemaVersion = 1; managed = true; accounts = []; }"
        );
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "Fixture setup failures must fail the test immediately."
    )]
    fn cli_errors_are_redacted_and_never_initialize_graphics() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.nix");
        fs::write(
            &path,
            "{ schemaVersion = 1; password = \"synthetic-secret\"; }",
        )
        .unwrap();
        let output = command(&[
            "config".as_ref(),
            "check".as_ref(),
            "--path".as_ref(),
            path.as_os_str(),
        ]);
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-secret"));
        let no_output: [u8; 0] = [];
        assert_eq!(output.stdout, no_output);
        assert!(command(&["--version".as_ref()]).status.success());
        assert!(
            !command(&["config".as_ref(), "unexpected".as_ref()])
                .status
                .success()
        );
        assert!(
            !command(&["--version".as_ref(), "extra".as_ref()])
                .status
                .success()
        );
    }

    #[expect(
        clippy::unwrap_used,
        clippy::single_call_fn,
        reason = "Fixture failures stop the test; rejected inputs may produce different error variants."
    )]
    async fn real_helper_evaluates_native_accounts_and_bounds_expansion() {
        let executable = Path::new(EXECUTABLE);
        let cancellation = CancellationToken::new();
        let documented = include_str!("../../../docs/CONFIGURATION.md");
        let source = documented
            .split("```nix\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        let config = process::evaluate(executable, source, &cancellation)
            .await
            .unwrap();
        assert_eq!(
            config.schema_version, 1,
            "documented schema version must be accepted"
        );
        assert!(
            !config.accounts.is_empty(),
            "documented accounts must be decoded"
        );
        let oversized = " ".repeat(MAX_SOURCE_BYTES + 1);
        assert!(
            matches!(
                process::evaluate(executable, &oversized, &cancellation).await,
                Err(process::EvaluationError::SourceTooLarge)
            ),
            "oversized source must be rejected"
        );
        for rejected_source in [
            "{ schemaVersion = 2; }",
            "{ schemaVersion = \"one\"; }",
            "{ schemaVersion = 1; accounts = true; }",
            "{ schemaVersion = builtins.seq (builtins.readFile /etc/passwd) 1; }",
            "{ schemaVersion = builtins.seq (builtins.getEnv \"HOME\") 1; }",
            "import /etc/passwd",
            "builtins.fetchurl { url = \"https://example.invalid\"; }",
        ] {
            assert!(
                process::evaluate(executable, rejected_source, &cancellation)
                    .await
                    .is_err(),
                "unsupported or invalid input must be rejected"
            );
        }
        let expanded = r#"{
        schemaVersion = 1;
        accounts = builtins.genList (i: {
            id = "a${builtins.toString i}";
            provider = "icloud";
            address = "person@example.test";
            displayName = builtins.concatStringsSep "" (builtins.genList (_: "x") 1000);
            userName = "person";
            auth = { kind = "password"; secretRef = "test"; };
        }) 1500;
    }"#;
        assert!(
            matches!(
                process::evaluate(executable, expanded, &cancellation).await,
                Err(process::EvaluationError::Rejected)
            ),
            "oversized expansion must be rejected"
        );
    }

    /// Inspect only this test process's children, without changing process state.
    #[expect(
        clippy::implicit_return,
        clippy::question_mark_used,
        reason = "Concise optional traversal handles transient proc entries."
    )]
    fn helper_child() -> Option<PathBuf> {
        for thread in fs::read_dir("/proc/self/task").ok()?.flatten() {
            let Ok(children) = fs::read_to_string(thread.path().join("children")) else {
                continue;
            };
            for pid in children.split_whitespace() {
                let path = Path::new("/proc").join(pid);
                if fs::read(path.join("cmdline")).is_ok_and(|command_line| {
                    command_line
                        .split(|&byte| byte == 0)
                        .any(|argument| argument == b"--internal-evaluate-config")
                }) {
                    return Some(path);
                }
            }
        }
        None
    }

    #[tokio::test]
    #[expect(
        clippy::implicit_return,
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "The spawned task and observed live child are required test fixtures."
    )]
    async fn recursive_helper_is_timed_out_and_cancellation_reaps_a_live_child() {
        real_helper_evaluates_native_accounts_and_bounds_expansion().await;
        let cancellation = CancellationToken::new();
        let started = Instant::now();
        let result = process::evaluate(Path::new(EXECUTABLE), NONTERMINATING, &cancellation).await;
        assert!(matches!(result, Err(process::EvaluationError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(helper_child().is_none());

        let worker_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            process::evaluate(Path::new(EXECUTABLE), NONTERMINATING, &worker_cancellation).await
        });
        let mut live_child = None;
        let maximum_attempts: u32 = 100;
        for _attempt in 0..maximum_attempts {
            if let Some(path) = helper_child()
                && let Ok(limits) = fs::read_to_string(path.join("limits"))
                && helper_limits_applied(&limits)
            {
                live_child = Some(path);
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(
            live_child.is_some(),
            "a live helper must apply all limits within one second"
        );
        cancellation.cancel();
        let cancelled_result = task.await.unwrap();
        assert!(matches!(
            cancelled_result,
            Err(process::EvaluationError::Cancelled)
        ));
        assert!(
            !live_child
                .expect("a real helper must have been observed")
                .exists()
        );
    }

    /// The executable is visible in proc before its Rust startup code runs.
    /// Observe readiness through all kernel limits, without adding a fixed delay.
    #[expect(
        clippy::implicit_return,
        clippy::single_call_fn,
        reason = "Keep kernel-limit readiness parsing separate from the async cancellation scenario."
    )]
    fn helper_limits_applied(limits: &str) -> bool {
        [
            ("Max address space", 3, ["1073741824", "1073741824"]),
            ("Max cpu time", 3, ["2", "3"]),
            ("Max core file size", 4, ["0", "0"]),
        ]
        .into_iter()
        .all(|(resource, fields, expected)| {
            limits
                .lines()
                .find(|line| line.starts_with(resource))
                .is_some_and(|line| line.split_whitespace().skip(fields).take(2).eq(expected))
        })
    }
}
