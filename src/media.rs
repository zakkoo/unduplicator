use crate::catalog::{Source, path_bytes};
use chrono::{DateTime, NaiveDate, Utc};
use std::path::{Path, PathBuf};

pub const IMAGES: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "gif", "bmp", "tiff", "tif", "webp",
];
pub const VIDEOS: &[&str] = &["mp4", "mov", "avi", "mkv", "wmv", "m4v", "3gp", "flv"];

pub fn metadata_json(bytes: &[u8]) -> Result<Option<SelectedDate>, String> {
    let data: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("malformed ExifTool JSON: {e}"))?;
    let object = data
        .as_array()
        .and_then(|a| a.first())
        .and_then(serde_json::Value::as_object)
        .ok_or("ExifTool JSON has no metadata object")?;
    for key in [
        "EXIF:DateTimeOriginal",
        "XMP:DateCreated",
        "QuickTime:CreationDate",
    ] {
        if let Some(date) = object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .and_then(calendar)
        {
            return Ok(Some(SelectedDate {
                date: Some(date),
                provenance: key.into(),
            }));
        }
    }
    // Embedded CreateDate groups only: filesystem timestamps never enter this adapter.
    for (key, value) in object {
        if key.ends_with(":CreateDate") && !key.starts_with("File:") && !key.starts_with("System:")
        {
            if let Some(date) = value.as_str().and_then(calendar) {
                return Ok(Some(SelectedDate {
                    date: Some(date),
                    provenance: key.clone(),
                }));
            }
        }
    }
    Ok(None)
}

const EXIFTOOL_ARGS: [&str; 8] = [
    "-j",
    "-G0",
    "-api",
    "QuickTimeUTC=0",
    "-DateTimeOriginal",
    "-DateCreated",
    "-CreationDate",
    "-CreateDate",
];

/// One long-lived `exiftool -stay_open` process; spawning one per file costs
/// more than reading the file.
struct Daemon {
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    _child: std::process::Child,
}
static DAEMON: std::sync::Mutex<Option<Daemon>> = std::sync::Mutex::new(None);

impl Daemon {
    fn spawn(executable: &std::ffi::OsStr) -> std::io::Result<Self> {
        let mut child = std::process::Command::new(executable)
            .args(["-stay_open", "True", "-@", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        Ok(Self {
            stdin: child.stdin.take().unwrap(),
            stdout: std::io::BufReader::new(child.stdout.take().unwrap()),
            _child: child,
        })
    }
    fn query(&mut self, path: &Path) -> std::io::Result<Vec<u8>> {
        use std::io::{BufRead, Write};
        let mut request = Vec::new();
        for arg in EXIFTOOL_ARGS {
            request.extend_from_slice(arg.as_bytes());
            request.push(b'\n');
        }
        request.extend_from_slice(&path_bytes(path));
        request.extend_from_slice(b"\n-execute\n");
        self.stdin.write_all(&request)?;
        self.stdin.flush()?;
        let mut output = Vec::new();
        loop {
            let mut line = Vec::new();
            if self.stdout.read_until(b'\n', &mut line)? == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            if line.strip_suffix(b"\n").map(|l| l.strip_suffix(b"\r").unwrap_or(l)) == Some(b"{ready}") {
                return Ok(output);
            }
            output.extend_from_slice(&line);
        }
    }
}

pub fn extract_metadata(path: &Path) -> (Option<SelectedDate>, Option<String>) {
    let executable = std::ffi::OsStr::new("exiftool");
    if path_bytes(path).contains(&b'\n') {
        return extract_metadata_using(path, executable);
    }
    let mut daemon = DAEMON.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if daemon.is_none() {
        match Daemon::spawn(executable) {
            Ok(spawned) => *daemon = Some(spawned),
            Err(error) => return (None, Some(format!("ExifTool {}: {error}", path.display()))),
        }
    }
    match daemon.as_mut().unwrap().query(path) {
        Ok(output) if output.is_empty() => (
            None,
            Some(format!("ExifTool {}: no output", path.display())),
        ),
        Ok(output) => match metadata_json(&output) {
            Ok(date) => (date, None),
            Err(error) => (None, Some(error)),
        },
        Err(error) => {
            *daemon = None;
            (None, Some(format!("ExifTool {}: {error}", path.display())))
        }
    }
}

pub fn extract_metadata_using(
    path: &Path,
    executable: &std::ffi::OsStr,
) -> (Option<SelectedDate>, Option<String>) {
    let output = std::process::Command::new(executable)
        .args(EXIFTOOL_ARGS)
        .arg(path)
        .output();
    match output {
        Err(error) => (None, Some(format!("ExifTool {}: {error}", path.display()))),
        Ok(output) if !output.status.success() => (
            None,
            Some(format!(
                "ExifTool {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        ),
        Ok(output) => match metadata_json(&output.stdout) {
            Ok(date) => (date, None),
            Err(error) => (None, Some(error)),
        },
    }
}

pub fn is_media(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    IMAGES.contains(&extension.as_str()) || VIDEOS.contains(&extension.as_str())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedDate {
    pub date: Option<NaiveDate>,
    pub provenance: String,
}

/// Preserve literal calendar dates, including offset and timezone-less timestamps.
pub fn calendar(value: &str) -> Option<NaiveDate> {
    let bytes = value.as_bytes();
    if bytes.len() < 10 || !matches!(bytes[4], b':' | b'-' | b'_') || bytes[7] != bytes[4] {
        return None;
    }
    let year = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    if !bytes[..4]
        .iter()
        .chain(&bytes[5..7])
        .chain(&bytes[8..10])
        .all(u8::is_ascii_digit)
    {
        return None;
    }
    let month = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let day = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    if !(1..=9999).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

pub fn filename_date(path: &Path) -> Option<NaiveDate> {
    let bytes = path.file_name().map(|n| path_bytes(Path::new(n)))?;
    for start in 0..bytes.len() {
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            continue;
        }
        for width in [10, 8] {
            if start + width > bytes.len()
                || bytes.get(start + width).is_some_and(u8::is_ascii_digit)
            {
                continue;
            }
            let value = &bytes[start..start + width];
            let date = if width == 10 {
                if !matches!(value[4], b'-' | b'_') {
                    continue;
                }
                std::str::from_utf8(value).ok().and_then(calendar)
            } else {
                if !value.iter().all(u8::is_ascii_digit) {
                    continue;
                }
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|s| calendar(&format!("{}-{}-{}", &s[..4], &s[4..6], &s[6..])))
            };
            if date.is_some() {
                return date;
            }
        }
    }
    None
}

pub fn select(source: &Source, metadata: Option<SelectedDate>) -> SelectedDate {
    if let Some(selected) = metadata.filter(|d| d.date.is_some()) {
        return selected;
    }
    if let Some(date) = filename_date(&source.relative) {
        return SelectedDate {
            date: Some(date),
            provenance: "filename".into(),
        };
    }
    for (timestamp, provenance) in [
        (source.birth, "source birth (UTC)"),
        (source.modified, "source modification (UTC)"),
    ] {
        if let Some(date) = timestamp
            .and_then(|t| DateTime::<Utc>::from_timestamp(t, 0))
            .map(|t| t.date_naive())
        {
            return SelectedDate {
                date: Some(date),
                provenance: provenance.into(),
            };
        }
    }
    SelectedDate {
        date: None,
        provenance: "unknown".into(),
    }
}

pub fn directory(path: &Path, selected: &SelectedDate) -> PathBuf {
    if !is_media(path) {
        return "Other".into();
    }
    match selected.date {
        Some(date) => PathBuf::from("Media")
            .join(date.format("%Y").to_string())
            .join(date.format("%Y-%m").to_string())
            .join(date.to_string()),
        None => "Media/Unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_extensions_and_dates() {
        for extension in IMAGES.iter().chain(VIDEOS) {
            assert!(is_media(Path::new(&format!(
                "f.{}",
                extension.to_uppercase()
            ))));
        }
        for name in ["a.txt", "a.raw", "a", "a.mp3"] {
            assert!(!is_media(Path::new(name)));
        }
        for value in [
            "2020-02-29",
            "2020:02:29 23:30:00-12:00",
            "2020-02-29T23:30:00+14:00",
        ] {
            assert_eq!(calendar(value).unwrap().to_string(), "2020-02-29");
        }
        for value in [
            "2019-02-29",
            "0000:00:00",
            "2020-13-01",
            "2020-04-31",
            "+020-01-02",
            "2020-+2-29",
        ] {
            assert!(calendar(value).is_none());
        }
        for name in [
            "IMG_20200229.jpg",
            "a2020-02-29x.jpg",
            "a2020_02_29.jpg",
            "2019-02-29_20200229.jpg",
        ] {
            assert_eq!(
                filename_date(Path::new(name)).unwrap().to_string(),
                "2020-02-29"
            );
        }
        for name in ["120200229.jpg", "202002290.jpg", "2020-02-290.jpg"] {
            assert!(filename_date(Path::new(name)).is_none());
        }
        assert_eq!(
            filename_date(Path::new("20010101_20200229.jpg"))
                .unwrap()
                .to_string(),
            "2001-01-01"
        );
    }
    #[test]
    fn priority_and_layout_exclude_staging_times() {
        let mut source = Source {
            run: 1,
            sequence: 0,
            relative: "20200229.jpg".into(),
            size: 0,
            birth: Some(0),
            modified: Some(86400),
        };
        let selected = select(
            &source,
            Some(SelectedDate {
                date: calendar("2001-01-01"),
                provenance: "metadata".into(),
            }),
        );
        assert_eq!(selected.provenance, "metadata");
        let selected = select(&source, None);
        assert_eq!(selected.provenance, "filename");
        assert_eq!(
            directory(&source.relative, &selected),
            Path::new("Media/2020/2020-02/2020-02-29")
        );
        source.relative = "image.jpg".into();
        assert_eq!(
            select(&source, None).date.unwrap().to_string(),
            "1970-01-01"
        );
        source.birth = None;
        assert_eq!(
            select(&source, None).date.unwrap().to_string(),
            "1970-01-02"
        );
        source.modified = None;
        let unknown = select(&source, None);
        assert_eq!(
            directory(&source.relative, &unknown),
            Path::new("Media/Unknown")
        );
        assert_eq!(directory(Path::new("a.txt"), &unknown), Path::new("Other"));
    }
}
