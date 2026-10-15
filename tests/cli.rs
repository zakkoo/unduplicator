use clap::Parser;
use std::process::Command;
use unduplicator::{cli::Cli, config::Config};

#[test]
fn accepts_all_commands_and_rejects_invalid_arguments() {
    assert!(
        Cli::try_parse_from([
            "u",
            "import",
            "--source",
            "s",
            "--destination",
            "d",
            "--catalog",
            "c",
            "--staging",
            "t",
            "--staging-budget",
            "1",
            "--verify"
        ])
        .is_ok()
    );
    for command in ["resume", "verify", "report"] {
        assert!(Cli::try_parse_from(["u", command, "--catalog", "c", "--run", "1"]).is_ok());
    }
    assert!(Cli::try_parse_from(["u", "runs", "--catalog", "c"]).is_ok());
    for args in [
        vec!["verify", "--catalog", "c", "--run", "0"],
        vec!["report", "--catalog", "c", "--run", "0"],
        vec!["report", "--catalog", "c"],
        vec!["resume", "--source", "s"],
        vec!["import"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    for budget in ["0", "-1", "abc", "18446744073709551616"] {
        assert!(
            Cli::try_parse_from([
                "u",
                "import",
                "--source",
                "s",
                "--destination",
                "d",
                "--catalog",
                "c",
                "--staging",
                "t",
                "--staging-budget",
                budget
            ])
            .is_err()
        );
    }
}

#[test]
fn version_matches_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("unduplicator {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn roots_and_missing_dependency_fail_before_execution() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let destination = temp.path().join("dest");
    let catalog = temp.path().join("catalog.sqlite");
    let staging = temp.path().join("stage");
    assert!(Config::validate(&source, &destination, &catalog, &staging, 10).is_ok());
    for overlap in [&source, &source.join("nested")] {
        assert!(Config::validate(&source, overlap, &catalog, &staging, 10).is_err());
    }
    let invoke = |dest: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("import")
            .arg("--source")
            .arg(&source)
            .arg("--destination")
            .arg(dest)
            .arg("--catalog")
            .arg(&catalog)
            .arg("--staging")
            .arg(&staging)
            .args(["--staging-budget", "10"])
            .env("PATH", temp.path())
            .output()
            .unwrap()
    };
    let missing = invoke(&destination);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("ExifTool is required"));
    assert_eq!(invoke(&source).status.code(), Some(2));
    assert!(!catalog.exists());
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn symlink_roots_cannot_hide_overlap() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    assert!(
        Config::validate(
            &source,
            &alias.join("dest"),
            &temp.path().join("db"),
            &temp.path().join("stage"),
            1
        )
        .is_err()
    );
}
