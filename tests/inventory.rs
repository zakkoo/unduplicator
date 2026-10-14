use std::sync::atomic::AtomicBool;
use unduplicator::{
    catalog::{Catalog, path_bytes},
    config::Config,
    inventory::inventory,
};

fn setup(temp: &tempfile::TempDir) -> (Catalog, Config, i64) {
    let source = temp.path().join("source");
    std::fs::create_dir_all(&source).unwrap();
    let config = Config::validate(
        &source,
        &temp.path().join("dest"),
        &temp.path().join("db"),
        &temp.path().join("stage"),
        100,
    )
    .unwrap();
    let catalog = Catalog::open(&config.catalog, true).unwrap();
    let run = catalog.new_run(&config).unwrap();
    (catalog, config, run)
}

#[test]
fn nested_local_order_and_empty_inventory() {
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(&temp);
    std::fs::create_dir(cfg.source.join("a")).unwrap();
    std::fs::write(cfg.source.join("z"), b"abc").unwrap();
    std::fs::write(cfg.source.join("a/x"), b"hi").unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    assert_eq!(cat.totals(run).unwrap(), (2, 5));
    let rows = cat.source_rows(run, false).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r.relative.to_str().unwrap())
            .collect::<Vec<_>>(),
        ["z", "a/x"]
    );
    assert!(rows.iter().all(|r| r.modified.is_some()));
    // Finalized inventory is reused, never rescanned.
    std::fs::write(cfg.source.join("later"), b"ignored").unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    assert_eq!(cat.totals(run).unwrap(), (2, 5));
    let empty = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(&empty);
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    assert_eq!(cat.totals(run).unwrap(), (0, 0));
}

#[cfg(unix)]
#[test]
fn native_paths_skips_and_metadata_only_reads() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    };
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(&temp);
    let name = std::ffi::OsString::from_vec(b"bad-\xff.dat".to_vec());
    let file = cfg.source.join(&name);
    std::fs::write(&file, b"abc").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    symlink(&file, cfg.source.join("link")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(cfg.source.join("fifo"))
            .status()
            .unwrap()
            .success()
    );
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    assert_eq!(cat.totals(run).unwrap(), (1, 3));
    assert_eq!(
        path_bytes(&cat.source_rows(run, false).unwrap()[0].relative),
        b"bad-\xff.dat"
    );
    let skipped: (i64, i64) = cat
        .db
        .query_row(
            "SELECT skipped_symlinks,skipped_special FROM runs WHERE id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(skipped, (1, 1));
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(unix)]
#[test]
fn inaccessible_directory_and_interrupted_restart() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let (mut cat, cfg, run) = setup(&temp);
    let inaccessible = cfg.source.join("closed");
    std::fs::create_dir(&inaccessible).unwrap();
    std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert!(inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).is_err());
    assert_eq!(cat.phase(run).unwrap(), "inventory");
    std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o700)).unwrap();
    for n in 0..300 {
        std::fs::write(cfg.source.join(format!("{n:03}")), b"a").unwrap();
    }
    assert!(inventory(&mut cat, &cfg, run, &AtomicBool::new(true)).is_err());
    // Simulate a previously committed partial inventory batch.
    cat.db
        .execute(
            "INSERT INTO source_files(run,sequence,relative,size) VALUES(?1,999,?2,7)",
            rusqlite::params![run, b"old".to_vec()],
        )
        .unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    assert_eq!(cat.totals(run).unwrap(), (300, 300));
    assert_eq!(cat.source_rows(run, false).unwrap().len(), 300);
}
