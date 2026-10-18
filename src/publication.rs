use crate::catalog::{Catalog, Content};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub const BUFFER_SIZE: usize = 1024 * 1024;
pub type Hook<'a> = dyn FnMut(&str) -> Result<()> + 'a;

pub fn platform_supported() -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "durable publication is supported only on Linux; no adapter is validated for this platform"
    );
    Ok(())
}
pub fn sync_directory(path: &Path) -> Result<()> {
    platform_supported()?;
    File::open(path)
        .with_context(|| format!("open directory for flush {}", path.display()))?
        .sync_all()
        .with_context(|| format!("directory flush unsupported or failed: {}", path.display()))
}

/// Refuse existing symlinks below the destination root before using a path.
/// Missing directories are allowed because publication creates them later.
pub fn check_parent(root: &Path, parent: &Path) -> Result<()> {
    let relative = parent
        .strip_prefix(root)
        .context("path is outside destination root")?;
    let mut current = root.to_path_buf();
    let check = |current: &Path| -> Result<()> {
        match current.symlink_metadata() {
            Ok(metadata) => {
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "symlink in destination directory: {}",
                    current.display()
                );
                ensure!(
                    metadata.is_dir(),
                    "destination parent is not a directory: {}",
                    current.display()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect destination directory {}", current.display())
                });
            }
        }
        Ok(())
    };
    check(&current)?;
    for component in relative.components() {
        ensure!(
            matches!(component, std::path::Component::Normal(_)),
            "invalid destination directory: {}",
            parent.display()
        );
        current.push(component.as_os_str());
        check(&current)?;
    }
    Ok(())
}

/// Only the final's parent changes on link/unlink; every ancestor was synced
/// when `create_parents` created it, so one directory flush is sufficient.
pub fn sync_parents(parent: &Path, root: &Path) -> Result<()> {
    check_parent(root, parent)?;
    sync_directory(parent)
}

pub(crate) fn create_parents(parent: &Path, hook: &mut Hook<'_>) -> Result<()> {
    if parent.is_dir() {
        return Ok(());
    }
    if let Some(ancestor) = parent.parent() {
        create_parents(ancestor, hook)?;
    }
    std::fs::create_dir(parent)
        .with_context(|| format!("create directory {}", parent.display()))?;
    sync_directory(parent)?;
    if let Some(ancestor) = parent.parent() {
        sync_directory(ancestor)?;
    }
    hook("new_directory_sync")?;
    Ok(())
}

/// Atomic same-filesystem hard-link publication: link never replaces an existing name.
pub fn no_replace(temporary: &Path, final_path: &Path) -> Result<()> {
    platform_supported()?;
    std::fs::hard_link(temporary, final_path).with_context(|| {
        format!(
            "no-replace publication unsupported or conflicted: {}",
            final_path.display()
        )
    })
}

pub fn publish(
    content: &Content,
    root: &Path,
    input: &Path,
    stop: &AtomicBool,
    hook: &mut Hook<'_>,
    bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    platform_supported()?;
    let final_path = root.join(&content.intended);
    let temporary = root.join(&content.temporary);
    let parent = final_path.parent().context("final has no parent")?;
    check_parent(root, parent)?;
    create_parents(parent, hook)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("create exclusive temporary {}", temporary.display()))?;
    let mut source =
        File::open(input).with_context(|| format!("read copy input {}", input.display()))?;
    let mut buffer = vec![0; BUFFER_SIZE];
    let mut copied = 0;
    loop {
        ensure!(!stop.load(Ordering::Relaxed), "interrupted during copy");
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        copied += count as u64;
        bytes(count as u64);
        hook("partial_transfer")?;
    }
    ensure!(
        copied == content.size,
        "copy input size differs from recorded size"
    );
    output.sync_all().context("destination file flush failed")?;
    hook("file_sync")?;
    drop(output);
    no_replace(&temporary, &final_path)?;
    hook("publication")?;
    std::fs::remove_file(&temporary)?;
    sync_parents(parent, root)?;
    hook("directory_sync")?;
    Ok(())
}

pub fn publish_existing_temporary(
    content: &Content,
    root: &Path,
    hook: &mut Hook<'_>,
) -> Result<()> {
    let temporary = root.join(&content.temporary);
    let final_path = root.join(&content.intended);
    check_parent(root, final_path.parent().context("final has no parent")?)?;
    File::open(&temporary)?
        .sync_all()
        .context("temporary file flush failed")?;
    hook("file_sync")?;
    no_replace(&temporary, &final_path)?;
    hook("publication")?;
    std::fs::remove_file(temporary)?;
    sync_parents(final_path.parent().context("final has no parent")?, root)?;
    hook("directory_sync")?;
    Ok(())
}

/// Coordinator calls only after successful filesystem finalization.
pub fn commit(catalog: &mut Catalog, content: &Content, hook: &mut Hook<'_>) -> Result<()> {
    hook("before_catalog_commit")?;
    catalog.complete(&content.digest)?;
    hook("catalog_commit")?;
    cleanup_stage(catalog, content, hook)
}
pub fn cleanup_stage(catalog: &Catalog, content: &Content, hook: &mut Hook<'_>) -> Result<()> {
    hook("before_stage_cleanup")?;
    if let Some(stage) = &content.stage {
        match std::fs::remove_file(stage) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("staging cleanup failed"),
        }
        catalog.db.execute(
            "UPDATE contents SET stage=NULL WHERE digest=?1",
            [&content.digest],
        )?;
    }
    hook("stage_cleanup")?;
    Ok(())
}
