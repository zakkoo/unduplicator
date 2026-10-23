use crate::{
    catalog::{Catalog, Content, Source},
    config::Config,
    media, publication,
};
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

fn exists(path: &Path) -> Result<bool> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("inspect destination {}", path.display())),
    }
}

pub fn suffixed(path: &Path, digest: &str) -> Result<PathBuf> {
    suffixed_with_limit(path, digest, 255)
}

fn suffixed_with_limit(path: &Path, digest: &str, limit: usize) -> Result<PathBuf> {
    use crate::catalog::{native_path, path_bytes};
    let mut stem = path_bytes(Path::new(path.file_stem().context("basename has no stem")?));
    let mut extension = path
        .extension()
        .map(|e| path_bytes(Path::new(e)))
        .unwrap_or_default();
    ensure!(
        limit > digest.len() + 1,
        "destination filename limit cannot hold a digest suffix"
    );
    let available = limit - digest.len() - 1;
    // Keep at least one stem byte; unusually long extensions must also be shortened.
    let extension_limit = available.saturating_sub(2);
    truncate_name(&mut extension, extension_limit);
    truncate_name(
        &mut stem,
        available
            - if extension.is_empty() {
                0
            } else {
                extension.len() + 1
            },
    );
    stem.push(b'-');
    stem.extend_from_slice(digest.as_bytes());
    if !extension.is_empty() {
        stem.push(b'.');
        stem.extend(extension);
    }
    Ok(path.with_file_name(native_path(stem)))
}

fn truncate_name(bytes: &mut Vec<u8>, limit: usize) {
    let mut end = bytes.len().min(limit);
    if let Ok(text) = std::str::from_utf8(bytes) {
        while !text.is_char_boundary(end) {
            end -= 1;
        }
    }
    bytes.truncate(end);
}

fn name_limit(path: &Path) -> Result<usize> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let existing = path
            .ancestors()
            .find(|p| p.is_dir())
            .context("destination has no existing ancestor")?;
        let native = std::ffi::CString::new(existing.as_os_str().as_bytes())?;
        let limit = unsafe { libc::pathconf(native.as_ptr(), libc::_PC_NAME_MAX) };
        if limit > 0 {
            return Ok(limit as usize);
        }
    }
    Ok(255)
}

/// Called by the coordinator in recorded inventory order, before copy dispatch.
/// Caller holds the source gate when metadata must be read from an oversized source.
pub fn decide(
    catalog: &Catalog,
    config: &Config,
    source: &Source,
    digest: &str,
    stage: Option<&Path>,
) -> Result<Content> {
    decide_with_stop(
        catalog,
        config,
        source,
        digest,
        stage,
        &AtomicBool::new(false),
    )
}

pub fn decide_with_stop(
    catalog: &Catalog,
    config: &Config,
    source: &Source,
    digest: &str,
    stage: Option<&Path>,
    stop: &AtomicBool,
) -> Result<Content> {
    if let Some(content) = catalog.content(digest)? {
        return Ok(content);
    }
    let original = config.source.join(&source.relative);
    let metadata = if media::is_media(&source.relative) {
        let (date, diagnostic) =
            media::extract_metadata_cancellable(stage.unwrap_or(&original), stop)?;
        if let Some(diagnostic) = diagnostic {
            eprintln!("metadata diagnostic: {diagnostic}");
        }
        date
    } else {
        None
    };
    let selected = media::select(source, metadata);
    let basename = source
        .relative
        .file_name()
        .context("source has no basename")?;
    let mut intended = media::directory(&source.relative, &selected).join(basename);
    publication::check_parent(
        &config.destination,
        &config
            .destination
            .join(intended.parent().context("destination has no parent")?),
    )?;
    let limit = name_limit(&config.destination.join(&intended))?;
    if crate::catalog::path_bytes(Path::new(basename)).len() > limit
        || catalog.path_reserved(&intended)?
        || exists(&config.destination.join(&intended))?
    {
        intended = suffixed_with_limit(&intended, digest, limit)?;
        ensure!(
            !catalog.path_reserved(&intended)? && !exists(&config.destination.join(&intended))?,
            "destination name conflict: {}",
            config.destination.join(&intended).display()
        );
    }
    let parent = intended.parent().context("destination has no parent")?;
    let mut ordinal = 0u64;
    let temporary = loop {
        let name = if ordinal == 0 {
            format!(".unduplicator-{digest}.tmp")
        } else {
            format!(".unduplicator-{digest}-{ordinal}.tmp")
        };
        let candidate = parent.join(name);
        if candidate != intended
            && !catalog.path_reserved(&candidate)?
            && !exists(&config.destination.join(&candidate))?
        {
            break candidate;
        }
        ordinal = ordinal
            .checked_add(1)
            .context("temporary filename space exhausted")?;
    };
    Ok(Content {
        digest: digest.into(),
        size: source.size,
        source: original,
        intended,
        stage: stage.map(Path::to_owned),
        temporary,
        complete: false,
        date: selected.date.map(|d| d.to_string()),
        provenance: selected.provenance,
    })
}
