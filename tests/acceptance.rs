use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, AtomicU64},
};
use unduplicator::{catalog::Catalog, staging};

fn importer(source: &Path, destination: &Path, catalog: &Path, staging: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_unduplicator"));
    command
        .arg("import")
        .arg("--source")
        .arg(source)
        .arg("--destination")
        .arg(destination)
        .arg("--catalog")
        .arg(catalog)
        .arg("--staging")
        .arg(staging)
        .args(["--staging-budget", "4096", "--verify"]);
    command
}
fn snapshot(root: &Path) -> BTreeMap<std::path::PathBuf, (Vec<u8>, std::time::SystemTime)> {
    fn visit(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<std::path::PathBuf, (Vec<u8>, std::time::SystemTime)>,
    ) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    (
                        std::fs::read(&path).unwrap(),
                        std::fs::metadata(&path).unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
fn layout(catalog: &Catalog, run: i64) -> BTreeMap<String, std::path::PathBuf> {
    catalog
        .contents_for_run(run)
        .unwrap()
        .into_iter()
        .map(|c| (c.digest, c.intended))
        .collect()
}

#[test]
fn implausible_dates_are_silent_and_unknown_media_uses_filename_pattern_folders() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("dest");
    let catalog = temp.path().join("db");
    let stage = temp.path().join("stage");
    std::fs::create_dir(&source).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/capture.jpg");
    let examples = [
        ("00011101_00005341.jpg", "Media/Unknown/#_#"),
        ("IMG_1234.jpg", "Media/Unknown/IMG_#"),
        ("img_56789.jpg", "Media/Unknown/IMG_#"),
        (
            "19280302-3BE4-47A4-AD40-3DEFB724E1DB.jpg",
            "Media/Unknown/UUID",
        ),
        ("aabbccdd20200229aabbccdd20200229.jpg", "Media/Unknown/Hash"),
        ("holiday.jpg", "Media/Unknown/Other"),
        ("photo_20200229.jpg", "Media/2020/2020-02/2020-02-29"),
    ];
    for (name, _) in examples {
        let file = source.join(name);
        std::fs::copy(&fixture, &file).unwrap();
        // Use real, distinct JPEGs with unusable capture dates and current file times.
        let output = Command::new("exiftool")
            .args([
                "-overwrite_original",
                "-all=",
                "-DateTimeOriginal=0001:11:01 00:00:00",
            ])
            .arg(format!("-Comment={name}"))
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let original = snapshot(&source);
    let output = importer(&source, &destination, &catalog, &stage)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("(verified)"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("metadata diagnostic"));
    for (name, directory) in examples {
        let imported = destination.join(directory).join(name);
        assert_eq!(
            std::fs::read(&imported).unwrap(),
            std::fs::read(source.join(name)).unwrap()
        );
    }
    let cat = Catalog::open(&catalog, false).unwrap();
    assert_eq!(cat.done(1).unwrap(), examples.len() as u64);
    for content in cat.all_contents().unwrap() {
        if content.intended.starts_with("Media/Unknown") {
            assert!(content.date.is_none());
            assert_eq!(content.provenance, "unknown");
        }
    }
    assert_eq!(snapshot(&source), original);
}

#[test]
fn multi_source_resume_deduplication_layout_budget_and_verification() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let later = temp.path().join("later");
    let destination = temp.path().join("dest");
    let catalog = temp.path().join("db");
    let stage = temp.path().join("stage");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&later).unwrap();
    let large = vec![7u8; 16384];
    for name in ["a.bin", "b.bin"] {
        std::fs::write(source.join(name), &large).unwrap();
    }
    for (directory, data) in [("one", b"first".as_slice()), ("two", b"second")] {
        std::fs::create_dir(source.join(directory)).unwrap();
        std::fs::write(source.join(directory).join("same.txt"), data).unwrap();
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(fixtures.join("capture.jpg"), source.join("20200229.jpg")).unwrap();
    std::fs::copy(fixtures.join("capture.mp4"), source.join("video.mp4")).unwrap();
    std::fs::copy(fixtures.join("capture.png"), source.join("image.png")).unwrap();
    std::fs::copy(fixtures.join("capture.mov"), source.join("movie.mov")).unwrap();
    std::fs::write(source.join("empty"), b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::fs::write(
            source.join(std::ffi::OsString::from_vec(b"native-\xff.txt".to_vec())),
            b"native",
        )
        .unwrap();
    }
    std::fs::copy(source.join("20200229.jpg"), later.join("renamed.jpg")).unwrap();
    std::fs::write(later.join("new.txt"), b"new").unwrap();
    let original = snapshot(&source);
    let original_later = snapshot(&later);
    let mut command = importer(&source, &destination, &catalog, &stage);
    #[cfg(feature = "test-failpoints")]
    command.env("UNDUPLICATOR_FAILPOINT", "publication");
    let first = command.output().unwrap();
    #[cfg(feature = "test-failpoints")]
    assert_eq!(first.status.code(), Some(99));
    #[cfg(not(feature = "test-failpoints"))]
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let resumed = Command::new(env!("CARGO_BIN_EXE_unduplicator"))
        .arg("resume")
        .arg("--catalog")
        .arg(&catalog)
        .args(["--run", "1", "--verify"])
        .output()
        .unwrap();
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let output = String::from_utf8_lossy(&resumed.stdout);
    assert!(output.contains("(verified)"));
    let peak: u64 = output
        .split("staging peak ")
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(peak <= 4096);
    let cat = Catalog::open(&catalog, false).unwrap();
    let first_layout = layout(&cat, 1);
    assert_eq!(cat.done(1).unwrap(), original.len() as u64);
    let paths = first_layout
        .values()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>();
    assert!(
        paths
            .iter()
            .any(|p| p == "Media/2001/2001-01/2001-01-02/20200229.jpg")
    );
    assert!(
        paths
            .iter()
            .any(|p| p == "Media/2020/2020-02/2020-02-29/video.mp4")
    );
    assert!(
        paths
            .iter()
            .any(|p| p == "Media/2011/2011-05/2011-05-06/image.png")
    );
    assert!(
        paths
            .iter()
            .any(|p| p == "Media/2012/2012-06/2012-06-07/movie.mov")
    );
    assert!(paths.iter().any(|p| p == "Other/same.txt"));
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with("Other/same-") && p.ends_with(".txt"))
    );
    let count_before: i64 = cat
        .db
        .query_row("SELECT COUNT(*) FROM contents", [], |r| r.get(0))
        .unwrap();
    drop(cat);
    let second = importer(&later, &destination, &catalog, &stage)
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(String::from_utf8_lossy(&second.stdout).contains("destination bytes 3"));
    let cat = Catalog::open(&catalog, false).unwrap();
    assert_eq!(cat.done(2).unwrap(), 2);
    assert_eq!(cat.phase(2).unwrap(), "verified");
    assert_eq!(
        cat.db
            .query_row("SELECT COUNT(*) FROM contents", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        count_before + 1
    );
    for content in cat.all_contents().unwrap() {
        let (digest, size) = staging::hash(
            &destination.join(&content.intended),
            None,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        )
        .unwrap();
        assert_eq!(digest, content.digest);
        assert_eq!(size, content.size);
    }
    drop(cat);
    assert_eq!(snapshot(&source), original);
    assert_eq!(snapshot(&later), original_later);
    assert_eq!(std::fs::read_dir(&stage).unwrap().count(), 0);
    // A clean run of the same inventory yields the same digest/path decisions.
    let baseline = importer(
        &source,
        &temp.path().join("baseline-dest"),
        &temp.path().join("baseline-db"),
        &temp.path().join("baseline-stage"),
    )
    .output()
    .unwrap();
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    let baseline_cat = Catalog::open(&temp.path().join("baseline-db"), false).unwrap();
    assert_eq!(layout(&baseline_cat, 1), first_layout);
}
