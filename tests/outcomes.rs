use std::process::Command;

fn import(temp: &tempfile::TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_unduplicator"));
    command
        .arg("import")
        .arg("--source")
        .arg(temp.path().join("source"))
        .arg("--destination")
        .arg(temp.path().join("dest"))
        .arg("--catalog")
        .arg(temp.path().join("db"))
        .arg("--staging")
        .arg(temp.path().join("stage"))
        .args(["--staging-budget", "10"]);
    command
}
#[test]
fn successful_import_verification_mismatch_and_invalid_invocation() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("source")).unwrap();
    std::fs::write(temp.path().join("source/a"), b"original").unwrap();
    let output = import(&temp).output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Run ID: 1"));
    assert!(stdout.contains("(unverified)"));
    assert!(stdout.contains("0/1 done"));
    assert!(stdout.contains("Estimating"));
    let verify = || {
        Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("verify")
            .arg("--catalog")
            .arg(temp.path().join("db"))
            .args(["--run", "1"])
            .output()
            .unwrap()
    };
    let output = verify();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("(verified)"));
    std::fs::write(temp.path().join("dest/Other/a"), b"changed").unwrap();
    let output = verify();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("verification-failed run 1"));
    assert!(stderr.contains("mismatch"));
    let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .arg("resume")
        .arg("--catalog")
        .arg(temp.path().join("db"))
        .args(["--run", "1", "--verify"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let output = import(&temp)
        .arg("--staging-budget")
        .arg("0")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir(other.path().join("source")).unwrap();
    std::fs::write(other.path().join("source/a"), b"ok").unwrap();
    let output = import(&other).arg("--verify").output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("(verified)"));
}

#[cfg(unix)]
#[test]
fn source_read_failure_and_concurrent_command_are_unsuccessful() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let file = source.join("a");
    std::fs::write(&file, b"original").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    let output = import(&temp).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unfinished run 1"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("content read"));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"original");
    let cat = unduplicator::catalog::Catalog::open(&temp.path().join("db"), false).unwrap();
    assert_eq!(cat.done(1).unwrap(), 0);
    let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .arg("resume")
        .arg("--catalog")
        .arg(temp.path().join("db"))
        .args(["--run", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("locked"));
    drop(cat);
    let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .arg("resume")
        .arg("--catalog")
        .arg(temp.path().join("db"))
        .args(["--run", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
}

#[cfg(feature = "test-failpoints")]
#[test]
fn failed_copy_and_interruption_identify_resumable_work() {
    for (variable, code) in [
        ("UNDUPLICATOR_ERRORPOINT", 1),
        ("UNDUPLICATOR_INTERRUPTPOINT", 130),
    ] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("source")).unwrap();
        std::fs::write(temp.path().join("source/a"), b"original").unwrap();
        let output = import(&temp)
            .env(variable, "partial_transfer")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("unfinished run 1"));
        assert!(stderr.contains("Resume:"));
        let review = String::from_utf8_lossy(&output.stdout);
        assert!(review.contains("UNDUPLICATOR — RUN QUICK CHECK"));
        assert!(review.contains(if code == 130 {
            "Last invocation: interrupted"
        } else {
            "Last invocation: failed"
        }));
        let report = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("report")
            .arg("--catalog")
            .arg(temp.path().join("db"))
            .args(["--run", "1"])
            .output()
            .unwrap();
        assert!(report.status.success());
        assert!(review.ends_with(String::from_utf8_lossy(&report.stdout).as_ref()));
        let resumed = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("resume")
            .arg("--catalog")
            .arg(temp.path().join("db"))
            .args(["--run", "1", "--verify"])
            .output()
            .unwrap();
        assert_eq!(
            resumed.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert!(String::from_utf8_lossy(&resumed.stdout).contains("(verified)"));
    }
}
