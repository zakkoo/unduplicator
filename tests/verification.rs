use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use unduplicator::{
    catalog::Catalog,
    config::Config,
    disk::Gates,
    inventory::inventory,
    pipeline::{self, Counters, Observer},
    verification,
};

#[test]
fn pass_results_order_shared_dependencies_and_fresh_restart() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for (name, data) in [("a", b"same".as_slice()), ("b", b"same"), ("c", b"other")] {
        std::fs::write(source.join(name), data).unwrap();
    }
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
    let counters = Arc::new(Counters::default());
    pipeline::process(
        &mut cat,
        &cfg,
        run,
        Arc::new(AtomicBool::new(false)),
        counters.clone(),
        Gates::discover(&cfg.source, &cfg.destination, &cfg.staging),
        pipeline::noop(),
    )
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let observer: Observer = {
        let stop = stop.clone();
        Arc::new(move |name| {
            if name == "verification_result" {
                stop.store(true, Ordering::Relaxed);
            }
            Ok(())
        })
    };
    assert!(verification::verify(&mut cat, &cfg, run, &stop, &counters, &observer).is_err());
    assert_eq!(cat.phase(run).unwrap(), "verifying");
    let first: String = cat
        .db
        .query_row(
            "SELECT digest FROM verification_results WHERE pass=1 ORDER BY rowid",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(first, cat.contents_for_run(run).unwrap()[0].digest);
    verification::verify(
        &mut cat,
        &cfg,
        run,
        &AtomicBool::new(false),
        &counters,
        &pipeline::noop(),
    )
    .unwrap();
    assert_eq!(cat.phase(run).unwrap(), "verified");
    assert_eq!(counters.verification.load(Ordering::Relaxed), 9);
    assert_eq!(
        cat.db
            .query_row(
                "SELECT COUNT(*) FROM verification_results WHERE pass=2 AND success=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(cat.done(run).unwrap(), 3);
    let content = cat.contents_for_run(run).unwrap().remove(0);
    let final_path = cfg.destination.join(&content.intended);
    std::fs::write(&final_path, b"mismatch").unwrap();
    assert!(
        verification::verify(
            &mut cat,
            &cfg,
            run,
            &AtomicBool::new(false),
            &counters,
            &pipeline::noop()
        )
        .is_err()
    );
    assert_eq!(cat.phase(run).unwrap(), "verification-failed");
    std::fs::remove_file(&final_path).unwrap();
    assert!(
        verification::verify(
            &mut cat,
            &cfg,
            run,
            &AtomicBool::new(false),
            &counters,
            &pipeline::noop()
        )
        .is_err()
    );
    assert_eq!(
        cat.db
            .query_row(
                "SELECT COUNT(*) FROM verification_results WHERE pass=4 AND success=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&final_path, b"same").unwrap();
        std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            verification::verify(
                &mut cat,
                &cfg,
                run,
                &AtomicBool::new(false),
                &counters,
                &pipeline::noop()
            )
            .is_err()
        );
        assert_eq!(
            cat.db
                .query_row(
                    "SELECT COUNT(*) FROM verification_results WHERE pass=5 AND success=0",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        std::fs::set_permissions(final_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        use std::os::unix::fs::symlink;
        let other = cfg.destination.join("Other");
        let saved = cfg.destination.join("saved-other");
        std::fs::rename(&other, &saved).unwrap();
        symlink(&saved, &other).unwrap();
        assert!(
            verification::verify(
                &mut cat,
                &cfg,
                run,
                &AtomicBool::new(false),
                &counters,
                &pipeline::noop()
            )
            .is_err()
        );
        assert_eq!(cat.phase(run).unwrap(), "verification-failed");
        std::fs::remove_file(other).unwrap();
        std::fs::rename(saved, cfg.destination.join("Other")).unwrap();
    }
}
