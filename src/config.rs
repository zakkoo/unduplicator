use anyhow::{Context, Result, bail, ensure};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub catalog: PathBuf,
    pub staging: PathBuf,
    pub budget: u64,
}

/// Resolve existing symlinks, including ancestors of paths not created yet.
pub fn normalize(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => {
                resolved.push(other.as_os_str());
                match resolved.canonicalize() {
                    Ok(p) => resolved = p,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(e).with_context(|| format!("resolve {}", resolved.display()));
                    }
                }
            }
        }
    }
    Ok(resolved)
}

impl Config {
    pub fn validate(
        source: &Path,
        destination: &Path,
        catalog: &Path,
        staging: &Path,
        budget: u64,
    ) -> Result<Self> {
        ensure!(
            budget > 0 && budget <= i64::MAX as u64,
            "budget must be a positive SQLite-sized byte count"
        );
        let config = Self {
            source: normalize(source)?,
            destination: normalize(destination)?,
            catalog: normalize(catalog)?,
            staging: normalize(staging)?,
            budget,
        };
        ensure!(
            config.source.is_dir(),
            "source must be an existing directory"
        );
        for (name, path) in [
            ("destination", &config.destination),
            ("staging", &config.staging),
        ] {
            ensure!(
                !path.exists() || path.is_dir(),
                "{name} must be a directory"
            );
        }
        ensure!(
            !config.catalog.exists() || config.catalog.is_file(),
            "catalog must be a file"
        );
        let paths = [
            &config.source,
            &config.destination,
            &config.staging,
            &config.catalog,
        ];
        for (i, a) in paths.iter().enumerate() {
            for b in &paths[i + 1..] {
                if a.starts_with(b) || b.starts_with(a) {
                    bail!("roots overlap: {} and {}", a.display(), b.display());
                }
            }
        }
        // SQLite companions and the process lock are also generated content.
        for suffix in ["-wal", "-shm", ".lock"] {
            let mut companion = config.catalog.as_os_str().to_owned();
            companion.push(suffix);
            for root in [&config.source, &config.destination, &config.staging] {
                ensure!(
                    !Path::new(&companion).starts_with(root),
                    "catalog companion overlaps a root"
                );
            }
        }
        Ok(config)
    }
}

pub fn exiftool_preflight() -> Result<()> {
    let output = std::process::Command::new("exiftool")
        .arg("-ver")
        .output()
        .context(
            "ExifTool is required; install perl-image-exiftool and ensure exiftool is on PATH",
        )?;
    ensure!(
        output.status.success() && !output.stdout.is_empty(),
        "ExifTool preflight failed"
    );
    Ok(())
}
