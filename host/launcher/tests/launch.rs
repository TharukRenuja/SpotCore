use std::process::Command;

fn launcher() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spotcore-engine"))
}

#[test]
fn invalid_invocations_and_recursion_are_rejected() {
    assert!(!launcher().output().unwrap().status.success());
    let result = launcher()
        .args(["--backend", env!("CARGO_BIN_EXE_spotcore-engine"), "--"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("own backend"));
}

#[cfg(unix)]
#[test]
fn forwards_literal_arguments_environment_and_exit_status() {
    let output = launcher()
        .args([
            "--backend",
            "/bin/sh",
            "--",
            "-c",
            "printf '%s\\n%s' \"$1\" \"$SPOTCORE_LAUNCHER_TEST\"; exit 23",
            "fixture",
            "spaces ; $literal",
        ])
        .env("SPOTCORE_LAUNCHER_TEST", "synthetic-secret")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"spaces ; $literal\nsynthetic-secret");
    assert!(output.stderr.is_empty());
}

#[cfg(windows)]
#[test]
fn forwards_windows_exit_status() {
    let backend =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
    let output = launcher()
        .arg("--backend")
        .arg(backend)
        .args(["--", "/C", "exit", "23"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
}

#[cfg(unix)]
#[test]
fn backend_parent_guard_receives_launcher_pid() {
    let output = launcher()
        .args([
            "--parent-pid",
            &std::process::id().to_string(),
            "--backend",
            "/bin/sh",
            "--",
            "-c",
            "printf '%s|%s' \"$2\" \"$PPID\"",
            "fixture",
            "--parent-pid",
            "1",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let (argument, parent) = text.split_once('|').unwrap();
    assert_eq!(argument, parent);
    assert_ne!(argument, "1");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_exit_stops_launcher_and_backend_group() {
    let file = std::env::temp_dir().join(format!("spotcore-owner-test-{}", std::process::id()));
    let mut owner = Command::new("/bin/sh")
        .args([
            "-c",
            "\"$1\" --parent-pid \"$$\" --backend /bin/sleep -- 60 & echo $! > \"$2\"; sleep 0.5",
            "fixture",
            env!("CARGO_BIN_EXE_spotcore-engine"),
        ])
        .arg(&file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    assert!(owner.wait().unwrap().success());
    let pid: i32 = std::fs::read_to_string(&file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut stopped = false;
    for _ in 0..50 {
        let state = std::fs::read_to_string(format!("/proc/{pid}/status"));
        if state.map_or(true, |s| {
            s.lines()
                .any(|l| l.starts_with("State:") && l.contains("Z (zombie)"))
        }) {
            stopped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !stopped {
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    let _ = std::fs::remove_file(file);
    assert!(stopped, "launcher survived owner exit");
}
