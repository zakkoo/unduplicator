use std::{
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use unduplicator::{
    catalog::Catalog,
    config::Config,
    disk::Gates,
    inventory::inventory,
    naming,
    pipeline::{self, Counters},
    recovery, staging,
};

fn config(root: &Path, label: &str) -> Config {
    let source = root.join(format!("source{label}"));
    std::fs::create_dir_all(&source).unwrap();
    Config::validate(
        &source,
        &root.join(format!("dest{label}")),
        &root.join(format!("db{label}")),
        &root.join("stage"),
        4096,
    )
    .unwrap()
}
fn import(cfg: &Config) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_unduplicator"));
    command
        .arg("import")
        .arg("--source")
        .arg(&cfg.source)
        .arg("--destination")
        .arg(&cfg.destination)
        .arg("--catalog")
        .arg(&cfg.catalog)
        .arg("--staging")
        .arg(&cfg.staging)
        .arg("--staging-budget")
        .arg(cfg.budget.to_string());
    command
}
fn run_command(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("command hung: {}", String::from_utf8_lossy(&output.stderr));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn resume(cfg: &Config, run: i64) -> Output {
    run_command(
        Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("resume")
            .arg("--catalog")
            .arg(&cfg.catalog)
            .arg("--run")
            .arg(run.to_string())
            .arg("--verify"),
    )
}
fn successful(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn shared_staging_is_locked_namespaced_and_retained_bytes_are_budgeted() {
    let temp = tempfile::tempdir().unwrap();
    let mut a = config(temp.path(), "a");
    a.budget = 3;
    let mut b = config(temp.path(), "b");
    b.budget = 3;
    std::fs::write(a.source.join("file"), b"AAA").unwrap();
    std::fs::write(b.source.join("file"), b"BBB").unwrap();
    let mut ca = Catalog::open(&a.catalog, true).unwrap();
    let ra = ca.new_run(&a).unwrap();
    ca.prepare_staging(&a).unwrap();
    inventory(&mut ca, &a, ra, &AtomicBool::new(false)).unwrap();
    let stage = ca.stage_path(&a, ra, 0);
    let (digest, _) = staging::hash(
        &a.source.join("file"),
        Some(&stage),
        &AtomicBool::new(false),
        &Default::default(),
    )
    .unwrap();
    let row = ca.source_rows(ra, false).unwrap().remove(0);
    let content = naming::decide(&ca, &a, &row, &digest, Some(&stage)).unwrap();
    ca.claim(&row, &content).unwrap();
    let mut cb = Catalog::open(&b.catalog, true).unwrap();
    let rb = cb.new_run(&b).unwrap();
    assert_ne!(stage, cb.stage_path(&b, rb, 0));
    assert!(
        cb.prepare_staging(&b)
            .unwrap_err()
            .to_string()
            .contains("locked")
    );
    drop(ca);
    inventory(&mut cb, &b, rb, &AtomicBool::new(false)).unwrap();
    let counters = Arc::new(Counters::default());
    let gates = Gates::discover(&b.source, &b.destination, &b.staging);
    recovery::reconcile(
        &mut cb,
        &b,
        rb,
        &AtomicBool::new(false),
        &counters,
        &gates,
        &pipeline::noop(),
    )
    .unwrap();
    assert_eq!(std::fs::read(&stage).unwrap(), b"AAA");
    pipeline::process(
        &mut cb,
        &b,
        rb,
        Arc::new(AtomicBool::new(false)),
        counters.clone(),
        gates,
        pipeline::noop(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(b.destination.join("Other/file")).unwrap(),
        b"BBB"
    );
    assert_eq!(std::fs::read(&stage).unwrap(), b"AAA");
    assert_eq!(counters.stage_peak.load(Ordering::Relaxed), 3);
    drop(cb);
    successful(&resume(&a, ra));
    assert_eq!(
        std::fs::read(a.destination.join("Other/file")).unwrap(),
        b"AAA"
    );
}

#[test]
fn historical_staging_cannot_become_a_source_or_vice_versa() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
    let run = cat.new_run(&cfg).unwrap();
    cat.prepare_staging(&cfg).unwrap();
    let stage = cat.stage_path(&cfg, run, 0);
    std::fs::write(&stage, b"preserve").unwrap();
    let mut unsafe_cfg = cfg.clone();
    unsafe_cfg.source = cfg.staging.clone();
    unsafe_cfg.staging = temp.path().join("new-stage");
    assert!(cat.new_run(&unsafe_cfg).is_err());
    assert_eq!(std::fs::read(&stage).unwrap(), b"preserve");
    unsafe_cfg.source = temp.path().join("new-source");
    unsafe_cfg.staging = cfg.source.clone();
    assert!(cat.new_run(&unsafe_cfg).is_err());
}

#[cfg(unix)]
#[test]
fn verification_rejects_leaf_symlinks_and_fifos_without_blocking() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    let source = cfg.source.join("file");
    std::fs::write(&source, b"original").unwrap();
    successful(&run_command(&mut import(&cfg)));
    let final_path = cfg.destination.join("Other/file");
    for fifo in [false, true] {
        std::fs::remove_file(&final_path).unwrap();
        if fifo {
            let path = std::ffi::CString::new(final_path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        } else {
            symlink(&source, &final_path).unwrap();
        }
        let output = run_command(
            Command::new(env!("CARGO_BIN_EXE_unduplicator"))
                .arg("verify")
                .arg("--catalog")
                .arg(&cfg.catalog)
                .args(["--run", "1"]),
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains("Run 1: FAIL"));
    }
}

#[cfg(feature = "test-failpoints")]
#[test]
fn offline_older_run_does_not_block_new_content_or_a_pending_duplicate() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    std::fs::write(cfg.source.join("file"), b"AAA").unwrap();
    let output = run_command(import(&cfg).env("UNDUPLICATOR_FAILPOINT", "pending_commit"));
    assert_eq!(output.status.code(), Some(99));
    std::fs::remove_dir_all(&cfg.source).unwrap();
    std::fs::remove_dir_all(&cfg.staging).unwrap();
    let mut next = cfg.clone();
    next.source = temp.path().join("next-source");
    std::fs::create_dir(&next.source).unwrap();
    std::fs::write(next.source.join("new"), b"BBB").unwrap();
    successful(&run_command(import(&next).arg("--verify")));
    std::fs::write(next.source.join("duplicate"), b"AAA").unwrap();
    let interrupted =
        run_command(import(&next).env("UNDUPLICATOR_FAILPOINT", "duplicate_dependency"));
    assert_eq!(interrupted.status.code(), Some(99));
    // The newly hashed duplicate stage must remain usable after a second crash.
    std::fs::remove_file(next.source.join("duplicate")).unwrap();
    successful(&resume(&next, 3));
    assert_eq!(
        std::fs::read(cfg.destination.join("Other/file")).unwrap(),
        b"AAA"
    );
    assert_eq!(
        std::fs::read(cfg.destination.join("Other/new")).unwrap(),
        b"BBB"
    );
}

#[cfg(unix)]
#[test]
fn long_collisions_preserve_native_names_and_digest_suffixes() {
    use std::os::unix::ffi::OsStringExt;
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    let mut native = vec![b'x'; 251];
    native[0] = 0xff;
    native.extend_from_slice(b".txt");
    let names = [
        std::ffi::OsString::from(format!("{}.txt", "a".repeat(191))),
        std::ffi::OsString::from(format!("{}.txt", "é".repeat(125))),
        std::ffi::OsString::from_vec(native),
    ];
    for (i, name) in names.iter().enumerate() {
        for (folder, content) in [
            ("a", format!("original{i}")),
            ("b", format!("collision{i}")),
        ] {
            let directory = cfg.source.join(folder);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(name), content).unwrap();
        }
    }
    successful(&run_command(import(&cfg).arg("--verify")));
    successful(&resume(&cfg, 1));
    let cat = Catalog::open(&cfg.catalog, false).unwrap();
    assert_eq!(cat.done(1).unwrap(), 6);
    for content in cat.contents_for_run(1).unwrap() {
        assert!(
            unduplicator::catalog::path_bytes(Path::new(content.intended.file_name().unwrap()))
                .len()
                <= 255
        );
        assert_eq!(
            std::fs::read(&content.source).unwrap(),
            std::fs::read(cfg.destination.join(&content.intended)).unwrap()
        );
    }
}

#[test]
fn catalog_failure_while_recording_an_error_preserves_the_original_exit_status() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    successful(&run_command(&mut import(&cfg)));
    let cat = Catalog::open(&cfg.catalog, false).unwrap();
    cat.db.execute_batch("CREATE TRIGGER fail_completion BEFORE UPDATE OF state ON contents BEGIN SELECT RAISE(FAIL,'completion_write_failed'); END;
        CREATE TRIGGER fail_error BEFORE UPDATE OF error ON runs WHEN NEW.error IS NOT NULL BEGIN SELECT RAISE(FAIL,'error_record_write_failed'); END;").unwrap();
    drop(cat);
    std::fs::write(cfg.source.join("file"), b"data").unwrap();
    let output = run_command(&mut import(&cfg));
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("completion_write_failed"), "{stderr}");
    assert!(stderr.contains("could not record run error"), "{stderr}");
}

#[test]
fn pending_case_variants_reserve_distinct_names_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    for name in ["Photo.txt", "photo.txt"] {
        std::fs::write(cfg.source.join(name), name).unwrap();
    }
    let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
    let run = cat.new_run(&cfg).unwrap();
    inventory(&mut cat, &cfg, run, &AtomicBool::new(false)).unwrap();
    let rows = cat.source_rows(run, false).unwrap();
    let first = naming::decide(&cat, &cfg, &rows[0], &"a".repeat(64), None).unwrap();
    cat.claim(&rows[0], &first).unwrap();
    let second = naming::decide(&cat, &cfg, &rows[1], &"b".repeat(64), None).unwrap();
    assert!(second.intended.to_string_lossy().contains(&"b".repeat(64)));
    assert_ne!(
        first.intended.to_string_lossy().to_ascii_lowercase(),
        second.intended.to_string_lossy().to_ascii_lowercase()
    );
}

#[cfg(unix)]
#[test]
fn sigint_during_metadata_extraction_exits_130_and_can_resume() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/capture.jpg"),
        cfg.source.join("photo.jpg"),
    )
    .unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let fake = bin.join("exiftool");
    std::fs::write(&fake, "#!/bin/sh\nif [ \"$1\" = '-ver' ]; then echo 13; exit; fi\nwhile IFS= read -r line; do if [ \"$line\" = '-execute' ]; then : > \"$UNDUPLICATOR_TEST_READY\"; exec sleep 30; fi; done\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ready = temp.path().join("ready");
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let mut child = import(&cfg)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("UNDUPLICATOR_TEST_READY", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("metadata child did not start");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("SIGINT did not stop metadata read");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(130));
    successful(&resume(&cfg, 1));
}

#[cfg(target_os = "linux")]
#[test]
fn storage_errors_preserve_sources_pending_work_and_retryability() {
    let harness = tempfile::tempdir().unwrap();
    let library = harness.path().join("io_faults.so");
    successful(
        &Command::new("cc")
            .args(["-shared", "-fPIC", "-Wall", "-Werror"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/io_faults.c"))
            .args(["-ldl", "-o"])
            .arg(&library)
            .output()
            .unwrap(),
    );
    for (location, operation, errno) in [
        ("source", "read", "EIO"),
        ("stage", "write", "ENOSPC"),
        ("stage", "write", "EIO"),
        ("dest", "write", "ENOSPC"),
        ("dest", "write", "EIO"),
        ("dest", "sync", "EIO"),
        ("catalog", "write", "ENOSPC"),
        ("catalog", "write", "EIO"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut cfg = config(temp.path(), "");
        cfg.budget = 4 * 1024 * 1024;
        if location == "catalog" {
            successful(&run_command(&mut import(&cfg)));
        }
        let data = vec![42; 2 * 1024 * 1024 + 17];
        std::fs::write(cfg.source.join("file.bin"), &data).unwrap();
        let fault_path = match location {
            "source" => &cfg.source,
            "stage" => &cfg.staging,
            "dest" => &cfg.destination,
            _ => &cfg.catalog,
        };
        let mut command = import(&cfg);
        command
            .env("LD_PRELOAD", &library)
            .env("UNDUPLICATOR_TEST_FAULT_PATH", fault_path)
            .env("UNDUPLICATOR_TEST_FAULT_OP", operation)
            .env("UNDUPLICATOR_TEST_FAULT_ERRNO", errno);
        if location != "catalog" && operation != "sync" {
            command.env("UNDUPLICATOR_TEST_FAULT_AFTER", "1048576");
        }
        let output = run_command(&mut command);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{location}/{operation}/{errno}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(cfg.source.join("file.bin")).unwrap(), data);
        if location == "catalog" {
            successful(&run_command(import(&cfg).arg("--verify")));
        } else {
            let cat = Catalog::open(&cfg.catalog, false).unwrap();
            assert_eq!(cat.done(1).unwrap(), 0);
            drop(cat);
            successful(&resume(&cfg, 1));
        }
        assert_eq!(
            std::fs::read(cfg.destination.join("Other/file.bin")).unwrap(),
            data
        );
    }
}

#[test]
fn recovery_streams_past_a_large_unrelated_pending_run() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    let mut cat = Catalog::open(&cfg.catalog, true).unwrap();
    let older = cat.new_run(&cfg).unwrap();
    let tx = cat.db.transaction().unwrap();
    tx.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10000)
        INSERT INTO contents(digest,size,representative,intended,temporary,provenance)
        SELECT printf('%064x',x),1,CAST('/offline/file' AS BLOB),CAST('Other/'||x AS BLOB),CAST('Other/temp-'||x AS BLOB),'unknown' FROM n;").unwrap();
    tx.execute("INSERT INTO source_files(run,sequence,relative,size,digest) SELECT ?1,rowid,CAST(rowid AS BLOB),size,digest FROM contents", [older]).unwrap();
    tx.execute(
        "UPDATE runs SET phase='import',total_files=10000,total_bytes=10000 WHERE id=?1",
        [older],
    )
    .unwrap();
    tx.commit().unwrap();
    let current = cat.new_run(&cfg).unwrap();
    inventory(&mut cat, &cfg, current, &AtomicBool::new(false)).unwrap();
    recovery::reconcile(
        &mut cat,
        &cfg,
        current,
        &AtomicBool::new(false),
        &Counters::default(),
        &Gates::discover(&cfg.source, &cfg.destination, &cfg.staging),
        &pipeline::noop(),
    )
    .unwrap();
    assert!(cat.next_recovery_content(current, 0).unwrap().is_none());
    assert_eq!(
        cat.db
            .query_row(
                "SELECT COUNT(*) FROM contents WHERE state='pending' AND error IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        10000
    );
}

#[cfg(all(target_os = "linux", feature = "test-failpoints"))]
#[test]
fn restart_flushes_existing_ancestors_after_an_interrupted_mkdir() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path(), "");
    std::fs::write(cfg.source.join("file"), b"original").unwrap();
    std::fs::create_dir(&cfg.destination).unwrap();
    let crash = run_command(import(&cfg).env("UNDUPLICATOR_FAILPOINT", "directory_created"));
    assert_eq!(crash.status.code(), Some(99));
    assert!(cfg.destination.join("Other").is_dir());
    let library = temp.path().join("io_faults.so");
    successful(
        &Command::new("cc")
            .args(["-shared", "-fPIC", "-Wall", "-Werror"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/io_faults.c"))
            .args(["-ldl", "-o"])
            .arg(&library)
            .output()
            .unwrap(),
    );
    let output = run_command(
        Command::new(env!("CARGO_BIN_EXE_unduplicator"))
            .arg("resume")
            .arg("--catalog")
            .arg(&cfg.catalog)
            .args(["--run", "1"])
            .env("LD_PRELOAD", &library)
            .env("UNDUPLICATOR_TEST_FAULT_PATH", &cfg.destination)
            .env("UNDUPLICATOR_TEST_FAULT_OP", "sync")
            .env("UNDUPLICATOR_TEST_FAULT_ERRNO", "EIO")
            .env("UNDUPLICATOR_TEST_FAULT_EXACT", "1"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("directory flush"));
    let cat = Catalog::open(&cfg.catalog, false).unwrap();
    assert_eq!(cat.done(1).unwrap(), 0);
    drop(cat);
    successful(&resume(&cfg, 1));
}
