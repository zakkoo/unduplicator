use std::sync::atomic::AtomicBool;
use unduplicator::{
    catalog::Catalog, config::Config, inventory::inventory, naming::decide, publication,
};

#[test]
fn real_publication_preserves_conflicts_and_failure_keeps_pending() {
    for failure in [
        None,
        Some("partial_transfer"),
        Some("file_sync"),
        Some("directory_sync"),
        Some("before_catalog_commit"),
        Some("sqlite_commit"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("file"), b"original").unwrap();
        let cfg = Config::validate(
            &source,
            &temp.path().join("dest"),
            &temp.path().join("db"),
            &temp.path().join("stage"),
            100,
        )
        .unwrap();
        let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
        let run = cat.new_run(&cfg).unwrap();
        inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
        std::fs::create_dir(&cfg.staging).unwrap();
        let stage = cfg.staging.join("stage");
        std::fs::copy(source.join("file"), &stage).unwrap();
        let row = cat.source_rows(run, false).unwrap().remove(0);
        let content = decide(&cat, &cfg, &row, &"a".repeat(64), Some(&stage)).unwrap();
        cat.claim(&row, &content).unwrap();
        if failure == Some("sqlite_commit") {
            cat.db.execute_batch("CREATE TRIGGER injected_commit_failure BEFORE UPDATE OF state ON contents BEGIN SELECT RAISE(FAIL,'injected SQLite commit failure'); END;").unwrap();
        }
        let mut operations = vec!["pending_commit".to_string()];
        let mut transferred = 0;
        let mut hook = |operation: &str| {
            operations.push(operation.to_string());
            if failure == Some(operation) {
                anyhow::bail!("injected {operation}");
            }
            Ok(())
        };
        let result = publication::publish(
            &content,
            &cfg.destination,
            &stage,
            &AtomicBool::new(false),
            &mut hook,
            &mut |n| transferred += n,
        )
        .and_then(|()| publication::commit(&mut cat, &content, &mut hook));
        if failure.is_some() {
            assert!(result.is_err());
            assert_eq!(cat.done(run).unwrap(), 0);
            assert!(stage.exists());
        } else {
            result.unwrap();
            assert_eq!(cat.done(run).unwrap(), 1);
            assert!(!stage.exists());
            assert_eq!(transferred, 8);
            let position = |operation| operations.iter().position(|s| s == operation).unwrap();
            let expected = [
                "pending_commit",
                "partial_transfer",
                "file_sync",
                "publication",
                "directory_sync",
                "before_catalog_commit",
                "catalog_commit",
                "before_stage_cleanup",
                "stage_cleanup",
            ];
            assert!(expected.windows(2).all(|w| position(w[0]) < position(w[1])));
            assert_eq!(
                std::fs::read(cfg.destination.join(&content.intended)).unwrap(),
                b"original"
            );
            assert!(
                publication::publish(
                    &content,
                    &cfg.destination,
                    &stage,
                    &AtomicBool::new(false),
                    &mut |_| Ok(()),
                    &mut |_| {}
                )
                .is_err()
            );
            assert_eq!(
                std::fs::read(cfg.destination.join(&content.intended)).unwrap(),
                b"original"
            );
        }
        assert_eq!(std::fs::read(source.join("file")).unwrap(), b"original");
    }
}

#[test]
fn no_replace_and_unsupported_flush_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();
    assert!(publication::no_replace(&a, &b).is_err());
    assert_eq!(std::fs::read(&b).unwrap(), b"b");
    assert!(publication::sync_directory(std::path::Path::new("/proc")).is_err());
    assert!(publication::platform_supported().is_ok());
}
