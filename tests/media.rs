use std::path::Path;
use unduplicator::media::{extract_metadata, metadata_json};

#[test]
fn real_image_video_and_staged_content() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let (date, diagnostic) = extract_metadata(&fixtures.join("capture.jpg"));
    assert!(diagnostic.is_none());
    let date = date.unwrap();
    assert_eq!(date.date.unwrap().to_string(), "2001-01-02");
    assert_eq!(date.provenance, "EXIF:DateTimeOriginal");
    let (date, diagnostic) = extract_metadata(&fixtures.join("capture.mp4"));
    assert!(diagnostic.is_none());
    assert_eq!(date.unwrap().date.unwrap().to_string(), "2020-02-29");
    let (date, diagnostic) = extract_metadata(&fixtures.join("capture.png"));
    assert!(diagnostic.is_none());
    let date = date.unwrap();
    assert_eq!(date.date.unwrap().to_string(), "2011-05-06");
    assert_eq!(date.provenance, "XMP:DateCreated");
    let (date, diagnostic) = extract_metadata(&fixtures.join("capture.mov"));
    assert!(diagnostic.is_none());
    let date = date.unwrap();
    assert_eq!(date.date.unwrap().to_string(), "2012-06-07");
    assert_eq!(date.provenance, "QuickTime:CreateDate");
    let temp = tempfile::tempdir().unwrap();
    let stage = temp.path().join("stage-without-extension");
    std::fs::copy(fixtures.join("capture.jpg"), &stage).unwrap();
    assert_eq!(
        extract_metadata(&stage)
            .0
            .unwrap()
            .date
            .unwrap()
            .to_string(),
        "2001-01-02"
    );
    std::fs::write(&stage, b"no metadata").unwrap();
    assert!(extract_metadata(&stage).0.is_none());
    assert!(extract_metadata(&temp.path().join("missing")).1.is_some());
}

#[test]
fn tag_priority_invalid_absent_and_filesystem_exclusion() {
    let temp = tempfile::tempdir().unwrap();
    let (date, error) = unduplicator::media::extract_metadata_using(
        Path::new("image.jpg"),
        temp.path().join("missing-exiftool").as_os_str(),
    );
    assert!(date.is_none());
    assert!(error.is_some());
    let date=metadata_json(br#"[{"EXIF:DateTimeOriginal":"0000:00:00","XMP:DateCreated":"2002-03-04T23:59:00+14:00","QuickTime:CreationDate":"2003:01:01","EXIF:CreateDate":"2004:01:01"}]"#).unwrap().unwrap();
    assert_eq!(date.date.unwrap().to_string(), "2002-03-04");
    let date = metadata_json(
        br#"[{"QuickTime:CreationDate":"2003:01:01","EXIF:CreateDate":"2004:01:01"}]"#,
    )
    .unwrap()
    .unwrap();
    assert_eq!(date.provenance, "QuickTime:CreationDate");
    let date = metadata_json(br#"[{"EXIF:CreateDate":"2004:01:01"}]"#)
        .unwrap()
        .unwrap();
    assert_eq!(date.provenance, "EXIF:CreateDate");
    assert!(
        metadata_json(
            br#"[{"File:FileModifyDate":"2020:01:01","System:CreateDate":"2021:01:01"}]"#
        )
        .unwrap()
        .is_none()
    );
    for data in [b"bad".as_slice(), b"[]", b"{}"] {
        assert!(metadata_json(data).is_err());
    }
    assert!(metadata_json(b"[{}]").unwrap().is_none());
}

#[test]
fn implausible_metadata_silently_tries_other_embedded_dates() {
    let date = metadata_json(
        br#"[{"EXIF:DateTimeOriginal":"0001:11:01 00:00:00","XMP:DateCreated":"9981-12-01","QuickTime:CreationDate":"2003:01:01","EXIF:CreateDate":"2004:01:01"}]"#,
    )
    .unwrap()
    .unwrap();
    assert_eq!(date.provenance, "QuickTime:CreationDate");
    assert_eq!(date.date.unwrap().to_string(), "2003-01-01");
    assert!(
        metadata_json(
            br#"[{"EXIF:DateTimeOriginal":"0001:11:01","EXIF:CreateDate":"9981:12:01"}]"#
        )
        .unwrap()
        .is_none()
    );
}
