#![cfg(feature = "test-failpoints")]
use std::process::Command;
use unduplicator::{catalog::Catalog, staging};

#[test]
fn every_crash_window_recovers_without_overwrite_or_duplicate_copy() {
    for point in [
        "pending_commit",
        "partial_transfer",
        "file_sync",
        "publication",
        "directory_sync",
        "catalog_commit",
        "before_stage_cleanup",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("dest");
        let catalog = temp.path().join("db");
        let stage = temp.path().join("stage");
        std::fs::create_dir(&source).unwrap();
        let data = vec![42u8; unduplicator::publication::BUFFER_SIZE + 33];
        for name in ["a.bin", "b.bin"] {
            std::fs::write(source.join(name), &data).unwrap();
        }
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(source.join("nested/a.bin"), b"different").unwrap();
        std::fs::create_dir_all(destination.join("Other")).unwrap();
        std::fs::write(destination.join("Other/keep"), b"existing").unwrap();
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
            std::fs::read(destination.join("Other/keep")).unwrap(),
            b"existing"
        );
        assert_eq!(std::fs::read(source.join("a.bin")).unwrap(), data);
        assert_eq!(std::fs::read(source.join("b.bin")).unwrap(), data);
        assert_eq!(std::fs::read_dir(stage).unwrap().count(), 0);
        if [
            "publication",
            "directory_sync",
            "catalog_commit",
            "before_stage_cleanup",
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
    }
}
