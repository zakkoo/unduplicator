use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use unduplicator::{
    catalog::Catalog,
    config::Config,
    disk::Gates,
    inventory::inventory,
    naming::decide,
    pipeline::{self, Counters},
    publication, recovery, staging,
};

#[test]
fn validate_adopt_retry_conflict_and_completed_cleanup() {
    for scenario in [
        "final",
        "conflict",
        "temporary",
        "partial",
        "corrupt_stage",
        "missing_stage",
        "completed",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a"), b"content").unwrap();
        let cfg = Config::validate(
            &source,
            &temp.path().join("dest"),
            &temp.path().join("db"),
            &temp.path().join("stage"),
            7,
        )
        .unwrap();
        let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
        let run = cat.new_run(&cfg).unwrap();
        inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
        std::fs::create_dir(&cfg.staging).unwrap();
        let stage = cfg.staging.join(format!(".unduplicator-{run}-0.stage"));
        let (digest, _) = staging::hash(
            &source.join("a"),
            Some(&stage),
            &AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(0),
        )
        .unwrap();
        let row = cat.source_rows(run, false).unwrap().remove(0);
        let content = decide(&cat, &cfg, &row, &digest, Some(&stage)).unwrap();
        cat.claim(&row, &content).unwrap();
        let final_path = cfg.destination.join(&content.intended);
        let temporary = cfg.destination.join(&content.temporary);
        std::fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        match scenario {
            "final" => std::fs::write(&final_path, b"content").unwrap(),
            "conflict" => std::fs::write(&final_path, b"changed").unwrap(),
            "temporary" => std::fs::write(&temporary, b"content").unwrap(),
            "partial" => std::fs::write(&temporary, b"part").unwrap(),
            "corrupt_stage" => std::fs::write(&stage, b"corrupt").unwrap(),
            "missing_stage" => std::fs::remove_file(&stage).unwrap(),
            "completed" => {
                std::fs::write(&final_path, b"content").unwrap();
                cat.complete(&digest).unwrap();
            }
            _ => unreachable!(),
        }
        let unrelated = cfg.staging.join("unrelated");
        std::fs::write(&unrelated, b"keep").unwrap();
        let orphan = cfg.staging.join(format!(".unduplicator-{run}-999.stage"));
        std::fs::write(&orphan, b"").unwrap();
        let unknown = cfg.staging.join(".unduplicator-999-0.stage");
        std::fs::write(&unknown, b"keep").unwrap();
        let counters = Counters::default();
        let gates = Gates::discover(&cfg.source, &cfg.destination, &cfg.staging);
        let result = recovery::reconcile(
            &mut cat,
            &cfg,
            &AtomicBool::new(false),
            &counters,
            &gates,
            &pipeline::noop(),
        );
        if scenario == "conflict" {
            assert!(result.is_err());
            assert_eq!(cat.done(run).unwrap(), 0);
            assert_eq!(std::fs::read(&final_path).unwrap(), b"changed");
        } else {
            result.unwrap();
            assert_eq!(cat.done(run).unwrap(), 1);
            assert_eq!(std::fs::read(&final_path).unwrap(), b"content");
            assert!(!stage.exists());
            assert!(!orphan.exists());
            if ["final", "temporary", "completed"].contains(&scenario) {
                assert_eq!(counters.destination.load(Ordering::Relaxed), 0);
                assert_eq!(counters.copied_files.load(Ordering::Relaxed), 0);
            } else {
                assert_eq!(counters.destination.load(Ordering::Relaxed), 7);
                assert_eq!(counters.copied_files.load(Ordering::Relaxed), 1);
            }
            if ["corrupt_stage", "missing_stage"].contains(&scenario) {
                assert_eq!(counters.source.load(Ordering::Relaxed), 14);
            } else {
                assert_eq!(counters.source.load(Ordering::Relaxed), 0);
            }
        }
        assert!(unrelated.exists());
        assert!(unknown.exists());
        assert_eq!(std::fs::read(source.join("a")).unwrap(), b"content");
    }
}

#[cfg(unix)]
#[test]
fn pending_copy_rejects_destination_symlink_until_it_is_removed() {
    use sha2::Digest;
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"content").unwrap();
    let cfg = Config::validate(
        &source,
        &temp.path().join("dest"),
        &temp.path().join("db"),
        &temp.path().join("stage"),
        7,
    )
    .unwrap();
    let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
    let run = cat.new_run(&cfg).unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    let row = cat.source_rows(run, false).unwrap().remove(0);
    let digest = format!("{:x}", sha2::Sha256::digest(b"content"));
    let content = decide(&cat, &cfg, &row, &digest, None).unwrap();
    cat.claim(&row, &content).unwrap();
    std::fs::create_dir(&cfg.destination).unwrap();
    symlink(&source, cfg.destination.join("Other")).unwrap();
    let stop = AtomicBool::new(false);
    let counters = Counters::default();
    let gates = Gates::discover(&cfg.source, &cfg.destination, &cfg.staging);
    let result = recovery::reconcile(&mut cat, &cfg, &stop, &counters, &gates, &pipeline::noop());
    assert!(format!("{:#}", result.unwrap_err()).contains("symlink in destination directory"));
    assert_eq!(cat.done(run).unwrap(), 0);
    assert_eq!(std::fs::read_dir(&source).unwrap().count(), 1);

    std::fs::remove_file(cfg.destination.join("Other")).unwrap();
    recovery::reconcile(&mut cat, &cfg, &stop, &counters, &gates, &pipeline::noop()).unwrap();
    assert_eq!(cat.done(run).unwrap(), 1);
    assert_eq!(
        std::fs::read(cfg.destination.join("Other/a.txt")).unwrap(),
        b"content"
    );
    assert_eq!(std::fs::read_dir(&source).unwrap().count(), 1);
}

#[test]
fn recovery_drains_full_budget_before_new_staging() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for name in ["a", "b", "c"] {
        std::fs::write(source.join(name), name).unwrap();
    }
    let cfg = Config::validate(
        &source,
        &temp.path().join("dest"),
        &temp.path().join("db"),
        &temp.path().join("stage"),
        2,
    )
    .unwrap();
    let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
    let run = cat.new_run(&cfg).unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    std::fs::create_dir(&cfg.staging).unwrap();
    for row in cat.source_rows(run, false).unwrap().into_iter().take(2) {
        let stage = cfg
            .staging
            .join(format!(".unduplicator-{run}-{}.stage", row.sequence));
        let (digest, _) = staging::hash(
            &source.join(&row.relative),
            Some(&stage),
            &AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(0),
        )
        .unwrap();
        let content = decide(&cat, &cfg, &row, &digest, Some(&stage)).unwrap();
        cat.claim(&row, &content).unwrap();
    }
    let counters = Arc::new(Counters::default());
    let gates = Gates::discover(&cfg.source, &cfg.destination, &cfg.staging);
    recovery::reconcile(
        &mut cat,
        &cfg,
        &AtomicBool::new(false),
        &counters,
        &gates,
        &pipeline::noop(),
    )
    .unwrap();
    assert_eq!(cat.done(run).unwrap(), 2);
    assert_eq!(std::fs::read_dir(&cfg.staging).unwrap().count(), 0);
    pipeline::process(
        &mut cat,
        &cfg,
        run,
        Arc::new(AtomicBool::new(false)),
        counters.clone(),
        gates,
        pipeline::noop(),
    )
    .unwrap();
    assert!(counters.stage_peak.load(Ordering::Relaxed) <= 2);
    assert_eq!(cat.done(run).unwrap(), 3);
    publication::sync_directory(&cfg.destination).unwrap();
}
