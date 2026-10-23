use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, Ordering},
};
use unduplicator::{
    catalog::Catalog,
    config::Config,
    disk::Gates,
    inventory::inventory,
    pipeline::{self, Counters, Observer},
};

fn setup(temp: &tempfile::TempDir, budget: u64, files: &[(&str, &[u8])]) -> (Catalog, Config, i64) {
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for (name, data) in files {
        std::fs::write(source.join(name), data).unwrap();
    }
    let cfg = Config::validate(
        &source,
        &temp.path().join("dest"),
        &temp.path().join("db"),
        &temp.path().join("stage"),
        budget,
    )
    .unwrap();
    let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
    let run = cat.new_run(&cfg).unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    (cat, cfg, run)
}
fn gates() -> Gates {
    Gates::from_ids(
        Some("source".into()),
        Some("dest".into()),
        Some("ssd".into()),
    )
}

#[test]
fn bounded_single_pass_and_cross_import_empty_deduplication() {
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(
        &temp,
        4,
        &[
            ("a", b"abcd"),
            ("b", b"abcd"),
            ("c", b"xyz"),
            ("d", b""),
            ("e", b""),
        ],
    );
    let counters = Arc::new(Counters::default());
    pipeline::process(
        &mut cat,
        &cfg,
        run,
        Arc::new(AtomicBool::new(false)),
        counters.clone(),
        gates(),
        pipeline::noop(),
    )
    .unwrap();
    assert_eq!(counters.source.load(Ordering::Relaxed), 11);
    assert_eq!(counters.destination.load(Ordering::Relaxed), 7);
    assert!(counters.stage_peak.load(Ordering::Relaxed) <= 4);
    assert_eq!(cat.done(run).unwrap(), 5);
    assert_eq!(cat.contents_for_run(run).unwrap().len(), 3);
    assert_eq!(std::fs::read_dir(&cfg.staging).unwrap().count(), 0);
    for (name, data) in [
        ("a", b"abcd".as_slice()),
        ("b", b"abcd"),
        ("c", b"xyz"),
        ("d", b""),
        ("e", b""),
    ] {
        assert_eq!(std::fs::read(cfg.source.join(name)).unwrap(), data);
    }
    let later = cat.new_run(&cfg).unwrap();
    inventory(&mut cat, &cfg, later, &AtomicBool::new(false)).unwrap();
    let counters = Arc::new(Counters::default());
    pipeline::process(
        &mut cat,
        &cfg,
        later,
        Arc::new(AtomicBool::new(false)),
        counters.clone(),
        gates(),
        pipeline::noop(),
    )
    .unwrap();
    assert_eq!(counters.source.load(Ordering::Relaxed), 11);
    assert_eq!(counters.destination.load(Ordering::Relaxed), 0);
    assert_eq!(cat.done(later).unwrap(), 5);
}

#[test]
fn pending_duplicate_waits_for_shared_commit() {
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(&temp, 100, &[("a", b"same"), ("b", b"same")]);
    let barrier = Arc::new(Barrier::new(2));
    let seen = Arc::new(AtomicBool::new(false));
    let path = cfg.catalog.clone();
    let observer: Observer = {
        let barrier = barrier.clone();
        let seen = seen.clone();
        Arc::new(move |name| {
            if name == "destination_begin" {
                barrier.wait();
            }
            if name == "duplicate_dependency" {
                let db = rusqlite::Connection::open(&path)?;
                let done:i64=db.query_row("SELECT COUNT(*) FROM source_files s JOIN contents c ON s.digest=c.digest WHERE c.state='complete'",[],|r|r.get(0))?;
                assert_eq!(done, 0);
                let snapshot = unduplicator::progress::Snapshot::load(
                    &db,
                    1,
                    &Counters::default(),
                    100,
                    None,
                )?;
                assert_eq!(
                    (
                        snapshot.done,
                        snapshot.total,
                        snapshot.source_remaining,
                        snapshot.destination_remaining
                    ),
                    (0, 2, 0, 4)
                );
                assert!(
                    snapshot.line(None).starts_with(
                        "0/2 done; source bytes 0; destination bytes 0/4 known unique"
                    )
                );
                seen.store(true, Ordering::Relaxed);
                barrier.wait();
            }
            Ok(())
        })
    };
    let counters = Arc::new(Counters::default());
    pipeline::process(
        &mut cat,
        &cfg,
        run,
        Arc::new(AtomicBool::new(false)),
        counters.clone(),
        gates(),
        observer,
    )
    .unwrap();
    assert!(seen.load(Ordering::Relaxed));
    assert_eq!(cat.done(run).unwrap(), 2);
    assert_eq!(counters.destination.load(Ordering::Relaxed), 4);
}

#[test]
fn oversized_passes_and_gate_access_counters() {
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(
        &temp,
        2,
        &[("a", b"oversized"), ("b", b"oversized"), ("c", b"x")],
    );
    let active = Arc::new(Mutex::new((0, 0)));
    let observer: Observer = {
        let active = active.clone();
        Arc::new(move |name| {
            let mut counts = active.lock().unwrap();
            match name {
                "source_begin" | "fallback_source_begin" => {
                    counts.0 += 1;
                    assert_eq!(counts.0, 1);
                }
                "source_end" | "fallback_source_end" => counts.0 -= 1,
                "destination_begin" => {
                    counts.1 += 1;
                    assert_eq!(counts.1, 1);
                }
                "destination_end" => counts.1 -= 1,
                _ => {}
            }
            Ok(())
        })
    };
    let counters = Arc::new(Counters::default());
    pipeline::process(
        &mut cat,
        &cfg,
        run,
        Arc::new(AtomicBool::new(false)),
        counters.clone(),
        gates(),
        observer,
    )
    .unwrap();
    assert_eq!(counters.source.load(Ordering::Relaxed), 28);
    assert_eq!(counters.destination.load(Ordering::Relaxed), 10);
    assert!(counters.stage_peak.load(Ordering::Relaxed) <= 2);
    assert_eq!(*active.lock().unwrap(), (0, 0));
    let snapshot =
        unduplicator::progress::Snapshot::load(&cat.db, run, &counters, cfg.budget, None).unwrap();
    assert_eq!(
        (
            snapshot.source_read,
            snapshot.destination_written,
            snapshot.verification_read
        ),
        (28, 10, 0)
    );
    assert_eq!(snapshot.done, 3);
}

#[test]
fn full_queues_failure_and_interruption_shutdown() {
    for interrupt in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let (mut cat, cfg, run) = setup(
            &temp,
            100,
            &[
                ("a", b"1"),
                ("b", b"2"),
                ("c", b"3"),
                ("d", b"4"),
                ("e", b"5"),
                ("f", b"6"),
            ],
        );
        let stop = Arc::new(AtomicBool::new(false));
        let full = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(Barrier::new(2));
        let started = Arc::new(AtomicBool::new(false));
        let observer: Observer = {
            let stop = stop.clone();
            let full = full.clone();
            let barrier = barrier.clone();
            let started = started.clone();
            Arc::new(move |name| {
                if name == "destination_begin" && !started.swap(true, Ordering::SeqCst) {
                    barrier.wait();
                }
                if name == "copy_queue_full" && !full.swap(true, Ordering::SeqCst) {
                    barrier.wait();
                }
                if name == "partial_transfer" {
                    if interrupt {
                        stop.store(true, Ordering::Relaxed);
                    } else {
                        anyhow::bail!("injected destination failure");
                    }
                }
                Ok(())
            })
        };
        let counters = Arc::new(Counters::default());
        assert!(pipeline::process(&mut cat, &cfg, run, stop, counters, gates(), observer).is_err());
        assert!(cat.done(run).unwrap() < 6);
        assert!(full.load(Ordering::SeqCst));
        assert_ne!(cat.phase(run).unwrap(), "complete");
        assert_eq!(std::fs::read(cfg.source.join("a")).unwrap(), b"1");
    }
}

#[test]
fn source_and_destination_workers_overlap_on_independent_devices() {
    use std::sync::atomic::AtomicUsize;
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(&temp, 100, &[("a", b"a"), ("b", b"b")]);
    let barrier = Arc::new(Barrier::new(2));
    let reads = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let observer: Observer = Arc::new(move |name| {
        if name == "source_begin" && reads.fetch_add(1, Ordering::SeqCst) == 1 {
            barrier.wait();
        }
        if name == "destination_begin" && writes.fetch_add(1, Ordering::SeqCst) == 0 {
            barrier.wait();
        }
        Ok(())
    });
    pipeline::process(
        &mut cat,
        &cfg,
        run,
        Arc::new(AtomicBool::new(false)),
        Arc::new(Counters::default()),
        gates(),
        observer,
    )
    .unwrap();
    assert_eq!(cat.done(run).unwrap(), 2);
}

#[test]
fn retained_stage_after_read_failure_prevents_new_admission() {
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(&temp, 4, &[("a", b"abcd"), ("b", b"efgh"), ("c", b"ijkl")]);
    let observer: Observer = Arc::new(|name| {
        if name == "source_end" {
            anyhow::bail!("injected staging failure");
        }
        Ok(())
    });
    assert!(
        pipeline::process(
            &mut cat,
            &cfg,
            run,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Counters::default()),
            gates(),
            observer
        )
        .is_err()
    );
    let bytes: u64 = std::fs::read_dir(&cfg.staging)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(bytes <= 4);
    assert_eq!(cat.done(run).unwrap(), 0);
}

#[test]
fn temporary_names_cannot_alias_final_names_or_delete_them_on_recovery() {
    use sha2::{Digest, Sha256};
    for own in [true, false] {
        let payload = if own { b"original".as_slice() } else { b"new" };
        let name = format!(".unduplicator-{:x}.tmp", Sha256::digest(payload));
        let temp = tempfile::tempdir().unwrap();
        let (mut cat, cfg, run) = setup(&temp, 100, &[(&name, b"original"), ("z", b"new")]);
        let counters = Arc::new(Counters::default());
        pipeline::process(
            &mut cat,
            &cfg,
            run,
            Arc::new(AtomicBool::new(false)),
            counters.clone(),
            gates(),
            pipeline::noop(),
        )
        .unwrap();
        for content in cat.contents_for_run(run).unwrap() {
            assert_ne!(content.intended, content.temporary);
        }
        assert_eq!(
            std::fs::read(cfg.destination.join("Other").join(&name)).unwrap(),
            b"original"
        );
        unduplicator::recovery::reconcile(
            &mut cat,
            &cfg,
            run,
            &AtomicBool::new(false),
            &counters,
            &gates(),
            &pipeline::noop(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(cfg.destination.join("Other").join(&name)).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(cfg.destination.join("Other/z")).unwrap(),
            b"new"
        );
    }
}
