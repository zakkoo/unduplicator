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
fn full(catalog: &Path, run: i64) -> String {
    text(
        &Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .args(["report", "--catalog"])
            .arg(catalog)
            .args(["--run", &run.to_string(), "--full"])
            .output()
            .unwrap(),
    )
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
    assert!(first.ends_with(&unduplicator::report::concise(&first_report)));
    assert!(first.contains("Run 1: PASS"));
    assert!(first.contains("Found: 2 files, 8.00 B"));
    assert!(first.contains("Destination: 1/1 unique files ready (1 without earlier-run matches, 0 seen in earlier runs)"));
    assert!(first.contains("Duplicates: 1 extra source copies, 4.00 B"));
    assert!(first.contains("Copied this invocation: 1 file, 4.00 B written"));
    assert!(first.contains("Failed: 0 source, 0 content, 0 verification"));
    assert!(!first.contains("SOURCE EXTENSIONS"));
    assert!(first_report.contains("VERDICT: PASS"));
    assert!(first_report.contains("Source files: 2/2 complete; 2/2 hashed"));
    assert!(first_report.contains("Destination groups: 1/1 complete; 0 pending"));
    assert!(first_report.contains("Source bytes: 8 hashed = 4 distinct + 4 repeated"));
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
    assert!(second.ends_with(&unduplicator::report::concise(&second_report)));
    assert!(second.contains("Destination: 2/2 unique files ready (1 without earlier-run matches, 1 seen in earlier runs)"));
    assert!(second.contains("Copied this invocation: 1 file, 5.00 B written"));
    let old_format = second_report.replace("  Destination files copied: 1\n", "");
    assert!(
        unduplicator::report::concise(&old_format)
            .contains("Written this invocation: 5.00 B (file count not recorded)")
    );
    assert!(second_report.contains("VERDICT: PASS"));
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
    assert_eq!(
        text(&cli(&dbpath, "report", Some(1))),
        unduplicator::report::concise(&first_report)
    );
    assert_eq!(
        text(&cli(&dbpath, "report", Some(2))),
        unduplicator::report::concise(&second_report)
    );
    assert_eq!(full(&dbpath, 1), first_report);
    assert_eq!(full(&dbpath, 2), second_report);
    assert_eq!(cli(&dbpath, "report", Some(999)).status.code(), Some(1));
    drop(locked);
    std::fs::write(root.join("dest/Other/a.txt"), b"corrupt").unwrap();
    let failure = cli(&dbpath, "verify", Some(2));
    assert_eq!(failure.status.code(), Some(1));
    let failed_report = stored(&db, 2);
    assert_ne!(failed_report, second_report);
    assert!(
        String::from_utf8_lossy(&failure.stdout)
            .ends_with(&unduplicator::report::concise(&failed_report))
    );
    assert!(failed_report.contains("Last invocation: failed"));
    assert!(failed_report.contains("VERDICT: FAIL"));
    assert!(failed_report.contains("Recorded phase: verification-failed"));
    assert!(failed_report.contains("Failed: 1"));
    assert!(
        unduplicator::report::concise(&failed_report)
            .contains("Copied this invocation: 0 files, 0.00 B written")
    );
    assert!(failed_report.contains("mismatch"));
    assert!(!failed_report.contains("Latest verification passed"));
    assert_eq!(
        text(&cli(&dbpath, "report", Some(2))),
        unduplicator::report::concise(&failed_report)
    );
    assert_eq!(full(&dbpath, 2), failed_report);
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
    assert!(legacy.contains("Inventory: unfinished; counts are partial"));
    assert!(legacy.contains("Run 1: REVIEW"));
    assert!(legacy.contains("Found: 1 file"));
    assert!(legacy.contains("Inventory, hashing, or destination work remains unfinished."));
    assert!(full(&config.catalog, run).contains("Source files without a digest: 1"));
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
    assert_eq!(unduplicator::report::concise(&stored(&cat.db, run)), report);
    assert_eq!(text(&cli(&config.catalog, "report", Some(run))), report);
}

#[test]
fn verdict_distinguishes_unverified_skips_and_broken_inventory_totals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join("source")).unwrap();
    std::fs::write(root.join("source/file.txt"), b"content").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .args(["import", "--source"])
        .arg(root.join("source"))
        .arg("--destination")
        .arg(root.join("dest"))
        .arg("--catalog")
        .arg(root.join("db"))
        .arg("--staging")
        .arg(root.join("stage"))
        .args(["--staging-budget", "100"])
        .output()
        .unwrap();
    let initial = text(&output);
    assert!(initial.contains("Run 1: REVIEW"));
    assert!(initial.contains("Destination content has no complete passing verification pass."));
    let verified = text(&cli(&root.join("db"), "verify", Some(1)));
    assert!(verified.contains("Run 1: PASS"));
    let resumed_without_verification = text(&cli(&root.join("db"), "resume", Some(1)));
    assert!(resumed_without_verification.contains("Run 1: REVIEW"));
    assert!(resumed_without_verification.contains("Run is not currently recorded as verified"));

    std::fs::create_dir(root.join("source_with_skip")).unwrap();
    std::fs::write(root.join("source_with_skip/file.txt"), b"content").unwrap();
    std::os::unix::fs::symlink("file.txt", root.join("source_with_skip/link.txt")).unwrap();
    let skipped = text(&import(root, "source_with_skip"));
    assert!(skipped.contains("Run 2: REVIEW"));
    assert!(skipped.contains("Skipped entries need review: 1 symlinks, 0 special files."));

    let db = rusqlite::Connection::open(root.join("db")).unwrap();
    db.execute("UPDATE runs SET total_bytes=total_bytes+1 WHERE id=1", [])
        .unwrap();
    let inconsistent = unduplicator::report::generate(&db, 1).unwrap();
    assert!(inconsistent.contains("VERDICT: FAIL"));
    assert!(inconsistent.contains("Inventory file or byte totals do not reconcile."));
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
