use std::{path::Path, sync::atomic::AtomicBool};
use unduplicator::{
    catalog::Catalog,
    config::Config,
    inventory::inventory,
    naming::{decide, suffixed},
};

#[test]
fn names_representatives_and_provenance_survive_completion_order_and_resume() {
    let mut expected = None;
    for seed in 1..=24u64 {
        let mut state = seed;
        let mut order = [0, 1, 2];
        for i in (1..3).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            order.swap(i, (state as usize) % (i + 1));
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        for directory in ["a", "b", "c"] {
            std::fs::create_dir_all(source.join(directory)).unwrap();
            std::fs::write(source.join(directory).join("same.txt"), directory).unwrap();
        }
        let cfg = Config::validate(
            &source,
            &temp.path().join("dest"),
            &temp.path().join("db"),
            &temp.path().join("stage"),
            10,
        )
        .unwrap();
        let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
        let run = cat.new_run(&cfg).unwrap();
        inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
        let rows = cat.source_rows(run, false).unwrap();
        let digests = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
        let mut paths = Vec::new();
        for (row, digest) in rows.iter().zip(&digests) {
            let content = decide(&cat, &cfg, row, digest, None).unwrap();
            paths.push(content.intended.clone());
            assert_eq!(content.provenance, "unknown");
            assert!(content.date.is_none());
            cat.claim(row, &content).unwrap();
        }
        for n in order {
            cat.complete(&digests[n]).unwrap();
        }
        assert_eq!(paths[0], Path::new("Other/same.txt"));
        assert_eq!(
            paths[1],
            suffixed(Path::new("Other/same.txt"), &digests[1]).unwrap()
        );
        if let Some(expected) = &expected {
            assert_eq!(&paths, expected);
        } else {
            expected = Some(paths.clone());
        }
        drop(cat);
        let cat = Catalog::open(&cfg.catalog, false).unwrap();
        for (i, digest) in digests.iter().enumerate() {
            assert_eq!(cat.content(digest).unwrap().unwrap().intended, paths[i]);
        }
        assert_eq!(
            decide(&cat, &cfg, &rows[2], &digests[0], None)
                .unwrap()
                .source,
            cfg.source.join("a/same.txt")
        );
    }
}

#[test]
fn existing_names_are_preserved_and_secondary_conflicts_reported() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("same.txt"), b"new").unwrap();
    let cfg = Config::validate(
        &source,
        &temp.path().join("dest"),
        &temp.path().join("db"),
        &temp.path().join("stage"),
        10,
    )
    .unwrap();
    std::fs::create_dir_all(cfg.destination.join("Other")).unwrap();
    std::fs::write(cfg.destination.join("Other/same.txt"), b"existing").unwrap();
    let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
    let run = cat.new_run(&cfg).unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    let row = cat.source_rows(run, false).unwrap().remove(0);
    let digest = "d".repeat(64);
    let content = decide(&cat, &cfg, &row, &digest, None).unwrap();
    assert!(content.intended.to_string_lossy().contains(&digest));
    std::fs::write(cfg.destination.join(&content.intended), b"conflict").unwrap();
    assert!(decide(&cat, &cfg, &row, &digest, None).is_err());
    assert_eq!(
        std::fs::read(cfg.destination.join("Other/same.txt")).unwrap(),
        b"existing"
    );
}
