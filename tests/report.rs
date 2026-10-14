use std::{
    path::Path,
    process::{Command, Output},
};
use unduplicator::catalog::Catalog;

fn cli(catalog: &Path, command: &str, run: Option<i64>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_unduplicator"));
    cmd.arg(command).arg("--catalog").arg(catalog);
    if let Some(run) = run {
        cmd.args(["--run", &run.to_string()]);
    }
    cmd.output().unwrap()
}
fn import(root: &Path, source: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .arg("import")
        .arg("--source")
        .arg(root.join(source))
        .arg("--destination")
        .arg(root.join("dest"))
        .arg("--catalog")
        .arg(root.join("db"))
        .arg("--staging")
        .arg(root.join("stage"))
        .args(["--staging-budget", "100", "--verify"])
        .output()
        .unwrap()
}
fn text(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}
fn stored(db: &rusqlite::Connection, run: i64) -> String {
    db.query_row("SELECT report FROM run_reports WHERE run=?1", [run], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let dbpath = root.join("db");
    for dir in ["first", "second"] {
        std::fs::create_dir(root.join(dir)).unwrap();
    }
    for name in ["a.txt", "b.txt"] {
        std::fs::write(root.join("first").join(name), b"same").unwrap();
    }
    let first = text(&import(root, "first"));
    let db = rusqlite::Connection::open(&dbpath).unwrap();
    let first_report = stored(&db, 1);
    assert!(first.ends_with(&first_report));
    assert!(first_report.contains("Regular files recorded: 2"));
    assert!(first_report.contains("Extra copies within this source: 1"));
    assert!(first_report.contains("Redundant bytes within this source: 4.00 B (4 bytes)"));
    assert!(first_report.contains("Required destination content complete: 1/1"));
    assert!(first_report.contains("Latest verification passed"));
    for (name, data) in [
        ("shared.txt", b"same".as_slice()),
        ("copy.txt", b"same"),
        ("new.bin", b"other"),
    ] {
        std::fs::write(root.join("second").join(name), data).unwrap();
    }
    let second = text(&import(root, "second"));
    let second_report = stored(&db, 2);
    assert!(second.ends_with(&second_report));
    assert!(second_report.contains("Distinct content groups: 2"));
    assert!(
        second_report.contains("Distinct groups referenced by earlier runs: 1 / 4.00 B (4 bytes)")
    );
    assert!(
        second_report
            .contains("Distinct groups without earlier-run references: 1 / 5.00 B (5 bytes)")
    );
    assert!(second_report.contains("Source files mapped to completed content: 3/3"));
    assert!(second_report.contains("copy.txt"));
    assert!(second_report.contains("shared.txt"));
    assert_eq!(stored(&db, 1), first_report); // A saved review remains a snapshot.
    // Readers work while the importer lock is held and need no ExifTool/source.
    let locked = Catalog::open(&dbpath, false).unwrap();
    let listed = text(&cli(&dbpath, "runs", None));
    assert!(listed.contains("2\tverified\t3\t13\t"));
    assert!(listed.contains("1\tverified\t2\t8\t"));
    assert_eq!(text(&cli(&dbpath, "report", Some(1))), first_report);
    assert_eq!(text(&cli(&dbpath, "report", Some(2))), second_report);
    assert_eq!(cli(&dbpath, "report", Some(999)).status.code(), Some(1));
    drop(locked);
    std::fs::write(root.join("dest/Other/a.txt"), b"corrupt").unwrap();
    let failure = cli(&dbpath, "verify", Some(2));
    assert_eq!(failure.status.code(), Some(1));
    let failed_report = stored(&db, 2);
    assert_ne!(failed_report, second_report);
    assert!(String::from_utf8_lossy(&failure.stdout).ends_with(&failed_report));
    assert!(failed_report.contains("Last invocation: failed"));
    assert!(failed_report.contains("Recorded phase: verification-failed"));
    assert!(failed_report.contains("Failed: 1"));
    assert!(failed_report.contains("mismatch"));
    assert!(!failed_report.contains("Latest verification passed"));
    assert_eq!(text(&cli(&dbpath, "report", Some(2))), failed_report);
    std::fs::write(root.join("dest/Other/a.txt"), b"same").unwrap();
    assert!(text(&cli(&dbpath, "resume", Some(2))).contains("Last invocation: success"));
    assert!(stored(&db, 2).contains("Recorded phase: complete"));
}

#[test]
fn legacy_catalog_migration_and_unfinished_inventory_are_honest() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join("source")).unwrap();
    let config = unduplicator::config::Config::validate(
        &root.join("source"),
        &root.join("dest"),
        &root.join("db"),
        &root.join("stage"),
        100,
    )
    .unwrap();
    let cat = Catalog::open(&config.catalog, true).unwrap();
    let run = cat.new_run(&config).unwrap();
    // Simulate a partially committed inventory with no final totals.
    cat.db
        .execute(
            "INSERT INTO source_files(run,sequence,relative,size) VALUES(?1,0,?2,4)",
            rusqlite::params![run, b"partial\nfile".as_slice()],
        )
        .unwrap();
    cat.db
        .execute_batch("DROP TABLE run_reports; PRAGMA user_version=1;")
        .unwrap();
    drop(cat);
    let legacy = text(&cli(&config.catalog, "report", Some(run)));
    assert!(legacy.contains("current catalog snapshot (not persisted)"));
    assert!(legacy.contains("Inventory: unfinished; counts below are partial"));
    assert!(legacy.contains("Regular files recorded: 1"));
    assert!(legacy.contains("Source files without a digest: 1"));
    assert!(legacy.contains("Import unfinished"));
    assert!(!legacy.contains("Every inventoried source file"));
    assert!(text(&cli(&config.catalog, "runs", None)).contains("1\tinventory\t?\t?\tnone"));
    let db = rusqlite::Connection::open(&config.catalog).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    let cat = Catalog::open(&config.catalog, false).unwrap();
    assert_eq!(
        cat.db
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        cat.db
            .query_row("SELECT COUNT(*) FROM source_files", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let report =
        unduplicator::report::save(&cat, run, "failed", &Default::default(), Default::default())
            .unwrap();
    assert_eq!(stored(&cat.db, run), report);
    assert_eq!(text(&cli(&config.catalog, "report", Some(run))), report);
}

#[test]
fn missing_catalog_is_not_created_and_empty_catalog_lists_cleanly() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    assert_eq!(cli(&path, "runs", None).status.code(), Some(1));
    assert_eq!(cli(&path, "report", Some(1)).status.code(), Some(1));
    assert!(!path.exists());
    let _cat = Catalog::open(&path, true).unwrap();
    assert!(text(&cli(&path, "runs", None)).contains("No runs recorded."));
}
