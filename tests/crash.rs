#![cfg(feature = "test-failpoints")]
use std::process::Command;
use unduplicator::{catalog::Catalog, staging};

#[test]
fn every_crash_window_recovers_without_overwrite_or_duplicate_copy() {
    for point in [
        "pending_commit",
        "new_directory_sync",
        "partial_transfer",
        "file_sync",
        "publication",
        "directory_sync",
        "before_catalog_commit",
        "catalog_commit",
        "before_stage_cleanup",
        "stage_cleanup",
        "before_report_commit",
        "report_commit",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("dest");
        let catalog = temp.path().join("catalog/nested/db");
        let stage = temp.path().join("stage");
        std::fs::create_dir(&source).unwrap();
        let data = vec![42u8; unduplicator::publication::BUFFER_SIZE + 33];
        for name in ["a.bin", "b.bin"] {
            std::fs::write(source.join(name), &data).unwrap();
        }
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(source.join("nested/a.bin"), b"different").unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("keep"), b"existing").unwrap();
        let crash = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("import")
            .arg("--source")
            .arg(&source)
            .arg("--destination")
            .arg(&destination)
            .arg("--catalog")
            .arg(&catalog)
            .arg("--staging")
            .arg(&stage)
            .args(["--staging-budget", "4194304"])
            .env("UNDUPLICATOR_FAILPOINT", point)
            .output()
            .unwrap();
        assert_eq!(
            crash.status.code(),
            Some(99),
            "{point}: {}",
            String::from_utf8_lossy(&crash.stderr)
        );
        assert!(String::from_utf8_lossy(&crash.stdout).contains("Run ID: 1"));
        let before = Catalog::open(&catalog, false).unwrap();
        let saved = before.contents_for_run(1).unwrap();
        assert_catalog_report(&before, &catalog, 1);
        if point == "partial_transfer" {
            assert!(
                std::fs::metadata(destination.join(&saved[0].temporary))
                    .unwrap()
                    .len()
                    < data.len() as u64
            );
        }
        drop(before);
        let resumed = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("resume")
            .arg("--catalog")
            .arg(&catalog)
            .args(["--run", "1"])
            .output()
            .unwrap();
        assert!(
            resumed.status.success(),
            "{point}: {}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        let cat = Catalog::open(&catalog, false).unwrap();
        assert_eq!(cat.done(1).unwrap(), 3);
        assert_catalog_report(&cat, &catalog, 1);
        let report = unduplicator::report::generate(&cat.db, 1).unwrap();
        assert!(report.contains(&format!("({} bytes)", data.len() + 9)));
        let contents = cat.contents_for_run(1).unwrap();
        assert_eq!(contents.len(), 2);
        for content in &contents {
            let (digest, size) = staging::hash(
                &destination.join(&content.intended),
                None,
                &std::sync::atomic::AtomicBool::new(false),
                &std::sync::atomic::AtomicU64::new(0),
            )
            .unwrap();
            assert_eq!(digest, content.digest);
            assert_eq!(size, content.size);
        }
        assert_eq!(
            contents
                .iter()
                .find(|c| c.digest == saved[0].digest)
                .unwrap()
                .intended,
            saved[0].intended
        );
        assert_eq!(
            std::fs::read(destination.join("keep")).unwrap(),
            b"existing"
        );
        assert_eq!(std::fs::read(source.join("a.bin")).unwrap(), data);
        assert_eq!(std::fs::read(source.join("b.bin")).unwrap(), data);
        assert_eq!(std::fs::read_dir(stage).unwrap().count(), 0);
        if [
            "publication",
            "directory_sync",
            "before_catalog_commit",
            "catalog_commit",
            "before_stage_cleanup",
            "stage_cleanup",
            "before_report_commit",
            "report_commit",
        ]
        .contains(&point)
        {
            let output = String::from_utf8_lossy(&resumed.stdout);
            let copied: u64 = output
                .lines()
                .find(|line| line.starts_with("complete:"))
                .unwrap()
                .rsplit("destination bytes ")
                .next()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            // The independent writer may also publish the nine-byte second
            // unique file before the coordinator reaches its crash point.
            assert!(copied <= 9, "adoption must not rewrite original: {output}");
        }
        drop(cat);
        // Repeating resume cannot double-count completed content.
        let again = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("resume")
            .arg("--catalog")
            .arg(&catalog)
            .args(["--run", "1"])
            .output()
            .unwrap();
        assert!(again.status.success());
        let cat = Catalog::open(&catalog, false).unwrap();
        assert_catalog_report(&cat, &catalog, 1);
        assert_eq!(unduplicator::report::generate(&cat.db, 1).unwrap(), report);
    }
}

fn assert_catalog_report(cat: &Catalog, path: &std::path::Path, run: i64) {
    assert_eq!(
        cat.db
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let content = cat.contents_for_run(run).unwrap();
    let complete: Vec<_> = content.iter().filter(|c| c.complete).collect();
    let size: u64 = complete.iter().map(|c| c.size).sum();
    let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .arg("report")
        .arg("--catalog")
        .arg(path)
        .args(["--run", &run.to_string(), "--full"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains(&format!(
            "Required destination content complete: {}/{}",
            complete.len(),
            content.len()
        )),
        "{text}"
    );
    let bytes = text
        .lines()
        .find(|s| s.contains("Completed destination content size:"))
        .unwrap();
    assert!(bytes.ends_with(&format!("({size} bytes)")), "{text}");
}

#[test]
fn crashes_during_verification_or_report_replacement_never_show_a_stale_verdict() {
    for point in [
        "before_verification_result_commit",
        "verification_result",
        "verification_complete",
        "before_report_commit",
        "report_commit",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir(root.join("source")).unwrap();
        std::fs::write(root.join("source/a"), b"content").unwrap();
        let path = root.join("db");
        let initial = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("import")
            .arg("--source")
            .arg(root.join("source"))
            .arg("--destination")
            .arg(root.join("dest"))
            .arg("--catalog")
            .arg(&path)
            .arg("--staging")
            .arg(root.join("stage"))
            .args(["--staging-budget", "100", "--verify"])
            .output()
            .unwrap();
        assert!(initial.status.success());
        let db = rusqlite::Connection::open(&path).unwrap();
        let previous: String = db
            .query_row("SELECT report FROM run_reports WHERE run=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        let crash = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("verify")
            .arg("--catalog")
            .arg(&path)
            .args(["--run", "1"])
            .env("UNDUPLICATOR_FAILPOINT", point)
            .output()
            .unwrap();
        assert_eq!(
            crash.status.code(),
            Some(99),
            "{point}: {}",
            String::from_utf8_lossy(&crash.stderr)
        );
        let saved: String = db
            .query_row("SELECT report FROM run_reports WHERE run=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        if point != "report_commit" {
            assert_eq!(
                saved, previous,
                "{point}: previous report survives uncommitted replacement"
            );
        }
        let cat = Catalog::open(&path, false).unwrap();
        assert_catalog_report(&cat, &path, 1);
        let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("report")
            .arg("--catalog")
            .arg(&path)
            .args(["--run", "1"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let current = String::from_utf8(output.stdout).unwrap();
        let unfinished =
            ["before_verification_result_commit", "verification_result"].contains(&point);
        assert!(
            current.contains(if unfinished {
                "Run 1: REVIEW"
            } else {
                "Run 1: PASS"
            }),
            "{point}: {current}"
        );
        let (results, bytes): (i64, i64) = db.query_row("SELECT (SELECT COUNT(*) FROM verification_results WHERE pass=p.id),bytes FROM verification_passes p ORDER BY id DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(
            (results, bytes),
            if point == "before_verification_result_commit" {
                (0, 0)
            } else {
                (1, 7)
            }
        );
        drop(cat);
        let resumed = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("resume")
            .arg("--catalog")
            .arg(&path)
            .args(["--run", "1", "--verify"])
            .output()
            .unwrap();
        assert!(resumed.status.success());
        assert!(String::from_utf8_lossy(&resumed.stdout).contains("Run 1: PASS"));
        let cat = Catalog::open(&path, false).unwrap();
        assert_catalog_report(&cat, &path, 1);
    }
}
