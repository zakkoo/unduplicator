use unduplicator::catalog::Catalog;

#[test]
fn pending_dependencies_completion_and_run_selection() {
    use unduplicator::{catalog::Content, config::Config, inventory::inventory};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for name in ["a", "b"] {
        std::fs::write(source.join(name), b"same").unwrap();
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
    inventory(
        &mut cat,
        &cfg,
        run,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let rows = cat.source_rows(run, false).unwrap();
    let content = Content {
        digest: "digest".into(),
        size: 4,
        source: source.join("a"),
        intended: "Other/a".into(),
        stage: Some(cfg.staging.join("stage")),
        temporary: "Other/temp".into(),
        complete: false,
        date: None,
        provenance: "other".into(),
    };
    assert!(cat.claim(&rows[0], &content).unwrap());
    assert!(!cat.claim(&rows[1], &content).unwrap());
    assert_eq!(cat.done(run).unwrap(), 0);
    cat.db
        .execute(
            "UPDATE contents SET error='injected failure' WHERE digest='digest'",
            [],
        )
        .unwrap();
    assert_eq!(cat.done(run).unwrap(), 0);
    assert!(cat.path_reserved(&content.intended).unwrap());
    cat.complete("digest").unwrap();
    assert_eq!(cat.done(run).unwrap(), 2);
    let later = cat.new_run(&cfg).unwrap();
    inventory(
        &mut cat,
        &cfg,
        later,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        !cat.claim(&cat.source_rows(later, false).unwrap()[0], &content)
            .unwrap()
    );
    assert_eq!(cat.done(later).unwrap(), 1);
    assert_eq!(
        cat.content("digest").unwrap().unwrap().source,
        source.join("a")
    );
    assert_eq!(cat.config(run).unwrap().budget, 100);
    assert!(cat.config(999).is_err());
    // Foreign keys reject dependencies on unknown content.
    assert!(
        cat.db
            .execute(
                "UPDATE source_files SET digest='missing' WHERE run=?1",
                [run]
            )
            .is_err()
    );
    assert_eq!(cat.contents_for_run(run).unwrap().len(), 1);
}

#[test]
fn version_lock_pragmas_and_destination_association() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let catalog = Catalog::open(&path, true).unwrap();
    assert!(
        Catalog::open(&path, false)
            .unwrap_err_or_else()
            .contains("locked")
    );
    for (pragma, expected) in [("foreign_keys", 1), ("synchronous", 2), ("user_version", 2)] {
        assert_eq!(
            catalog
                .db
                .pragma_query_value(None, pragma, |r| r.get::<_, i64>(0))
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        catalog
            .db
            .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    let destination = temp.path().join("destination");
    catalog.associate(&destination).unwrap();
    catalog.associate(&destination).unwrap();
    assert!(catalog.associate(&temp.path().join("other")).is_err());
    drop(catalog);
    let catalog = Catalog::open(&path, false).unwrap();
    catalog.db.pragma_update(None, "user_version", 999).unwrap();
    drop(catalog);
    assert!(Catalog::open(&path, false).is_err());
}

trait ErrorText {
    fn unwrap_err_or_else(self) -> String;
}
impl ErrorText for anyhow::Result<Catalog> {
    fn unwrap_err_or_else(self) -> String {
        match self {
            Ok(_) => panic!("expected error"),
            Err(e) => format!("{e:#}"),
        }
    }
}
