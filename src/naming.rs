use crate::{
    catalog::{Catalog, Content, Source},
    config::Config,
    media,
};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

fn exists(path: &Path) -> Result<bool> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("inspect destination {}", path.display())),
    }
}

pub fn suffixed(path: &Path, digest: &str) -> Result<PathBuf> {
    let mut name = path.file_stem().context("basename has no stem")?.to_owned();
    name.push("-");
    name.push(digest);
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    Ok(path.with_file_name(name))
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
    if let Some(content) = catalog.content(digest)? {
        return Ok(content);
    }
    let original = config.source.join(&source.relative);
    let metadata = if media::is_media(&source.relative) {
        let (date, diagnostic) = media::extract_metadata(stage.unwrap_or(&original));
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
    if catalog.path_reserved(&intended)? || exists(&config.destination.join(&intended))? {
        intended = suffixed(&intended, digest)?;
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
