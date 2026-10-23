use crate::{
    catalog::{Catalog, Content, path_bytes},
    config::Config,
    disk::Gates,
    pipeline::{Counters, Observer},
    publication, staging,
};
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn matches(
    path: &Path,
    content: &Content,
    stop: &AtomicBool,
    bytes: &std::sync::atomic::AtomicU64,
) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).context("recovery metadata read failed"),
    };
    if !metadata.is_file() || metadata.len() != content.size {
        return Ok(false);
    }
    let (digest, size) = staging::hash(path, None, stop, bytes)?;
    Ok(digest == content.digest && size == content.size)
}

/// A newer run may have hashed this pending digest while its original drive was
/// offline. Its source reference and deterministic stage name survive a crash
/// even if the content row still names the first representative.
fn recovery_input(
    catalog: &Catalog,
    config: &Config,
    content: &Content,
    stop: &AtomicBool,
    counters: &Counters,
) -> Result<(std::path::PathBuf, bool)> {
    if let Some(stage) = &content.stage {
        if matches(stage, content, stop, &counters.recovery)? {
            return Ok((stage.clone(), true));
        }
    }
    let mut statement = catalog.db.prepare(
        "SELECT r.source,r.staging,s.run,s.sequence,s.relative FROM source_files s
        JOIN runs r ON r.id=s.run WHERE s.digest=?1 ORDER BY (r.source=?2) DESC,s.run,s.sequence",
    )?;
    let mut rows = statement.query(rusqlite::params![
        content.digest,
        path_bytes(&config.source)
    ])?;
    while let Some(row) = rows.next()? {
        let root = crate::catalog::native_path(row.get(0)?);
        let stage_root = crate::catalog::native_path(row.get(1)?);
        let run: i64 = row.get(2)?;
        let sequence: i64 = row.get(3)?;
        let stage = stage_root.join(format!(
            ".unduplicator-{}-{run}-{sequence}.stage",
            catalog.identity
        ));
        if content.stage.as_ref() != Some(&stage)
            && matches(&stage, content, stop, &counters.recovery)?
        {
            return Ok((stage, true));
        }
        let source = root.join(crate::catalog::native_path(row.get(4)?));
        if matches(&source, content, stop, &counters.source)? {
            return Ok((source, false));
        }
    }
    anyhow::bail!(
        "representative source missing or differs from pending digest, and no valid duplicate is available: {}",
        content.source.display()
    )
}

/// Count retained payloads without loading catalog records or following symlinks.
pub fn retained_bytes(config: &Config) -> Result<u64> {
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(&config.staging)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            bytes = bytes
                .checked_add(entry.metadata()?.len())
                .context("staging size overflow")?;
        }
    }
    Ok(bytes)
}

pub fn reconcile(
    catalog: &mut Catalog,
    config: &Config,
    run: i64,
    stop: &AtomicBool,
    counters: &Counters,
    gates: &Gates,
    observer: &Observer,
) -> Result<()> {
    catalog.prepare_staging(config)?;
    counters
        .stage_peak
        .fetch_max(retained_bytes(config)?, Ordering::Relaxed);
    let mut after = 0;
    while let Some((rowid, content)) = catalog.next_recovery_content(run, after)? {
        after = rowid;
        recover_one(catalog, config, &content, stop, counters, gates, observer)?;
    }
    cleanup_orphans(catalog, config, run)?;
    Ok(())
}

pub fn recover_one(
    catalog: &mut Catalog,
    config: &Config,
    content: &Content,
    stop: &AtomicBool,
    counters: &Counters,
    gates: &Gates,
    observer: &Observer,
) -> Result<()> {
    ensure!(!stop.load(Ordering::Relaxed), "interrupted during recovery");
    let mut hook = |name: &str| observer(name);
    if content.complete {
        return publication::cleanup_stage(catalog, content, &mut hook);
    }
    let result = (|| {
        let final_path = config.destination.join(&content.intended);
        let temporary = config.destination.join(&content.temporary);
        publication::check_parent(
            &config.destination,
            final_path.parent().context("final has no parent")?,
        )?;
        publication::check_parent(
            &config.destination,
            temporary.parent().context("temporary has no parent")?,
        )?;
        if final_path.symlink_metadata().is_ok() {
            ensure!(
                matches(&final_path, content, stop, &counters.recovery)?,
                "conflicting final content: {}; pending copy remains unfinished",
                final_path.display()
            );
            staging::open_regular(&final_path)?
                .sync_all()
                .context("adopted final file flush failed")?;
            publication::sync_parents(
                final_path.parent().context("final has no parent")?,
                &config.destination,
            )?;
            hook("directory_sync")?;
            if !catalog.final_reserved(&content.temporary)? && temporary.symlink_metadata().is_ok()
            {
                std::fs::remove_file(&temporary)?;
                publication::sync_directory(
                    temporary.parent().context("temporary has no parent")?,
                )?;
            }
        } else if catalog.final_reserved(&content.temporary)? {
            anyhow::bail!(
                "recorded temporary aliases a reserved final path: {}; cannot remove or replace it",
                temporary.display()
            );
        } else if matches(&temporary, content, stop, &counters.recovery)? {
            publication::publish_existing_temporary(content, &config.destination, &mut hook)?;
        } else {
            if temporary.symlink_metadata().is_ok() {
                std::fs::remove_file(&temporary).context("remove invalid tool temporary")?;
            }
            let (input, valid_stage) = recovery_input(catalog, config, content, stop, counters)?;
            gates.write(!valid_stage, || {
                publication::publish(
                    content,
                    &config.destination,
                    &input,
                    stop,
                    &mut hook,
                    &mut |bytes| {
                        counters.destination.fetch_add(bytes, Ordering::Relaxed);
                        if !valid_stage {
                            counters.source.fetch_add(bytes, Ordering::Relaxed);
                        }
                    },
                )
            })?;
            counters.copied_files.fetch_add(1, Ordering::Relaxed);
        }
        publication::commit(catalog, content, &mut hook)
    })();
    if let Err(error) = result {
        if let Err(record_error) = catalog.db.execute(
            "UPDATE contents SET error=?1 WHERE digest=?2",
            rusqlite::params![format!("{error:#}"), content.digest],
        ) {
            eprintln!("could not record content error: {record_error:#}");
        }
        return Err(error);
    }
    Ok(())
}

fn stage_name(name: &str) -> Option<(i64, i64)> {
    let rest = name
        .strip_prefix(".unduplicator-")?
        .strip_suffix(".stage")?;
    let (run, sequence) = rest.split_once('-')?;
    let run = run.parse().ok()?;
    let sequence = sequence.parse().ok()?;
    (run > 0 && sequence >= 0).then_some((run, sequence))
}
fn temp_name(name: &str) -> bool {
    name.strip_prefix(".unduplicator-")
        .and_then(|n| n.strip_suffix(".tmp"))
        .is_some_and(|rest| {
            let (digest, ordinal) = rest
                .split_once('-')
                .map_or((rest, None), |(digest, ordinal)| (digest, Some(ordinal)));
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && ordinal.is_none_or(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}
pub fn cleanup_orphans(catalog: &Catalog, config: &Config, run: i64) -> Result<()> {
    let prefix = format!(".unduplicator-{}-", catalog.identity);
    if config.staging.is_dir() {
        for entry in std::fs::read_dir(&config.staging)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(rest) = name.to_str().and_then(|name| name.strip_prefix(&prefix)) else {
                continue;
            };
            let Some((owner, _)) = stage_name(&format!(".unduplicator-{rest}")) else {
                continue;
            };
            if owner != run {
                continue;
            }
            let path = entry.path();
            let referenced: bool = catalog.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM contents WHERE stage=?1)",
                [path_bytes(&path)],
                |r| r.get(0),
            )?;
            if !referenced {
                std::fs::remove_file(path)?;
            }
        }
    }
    // Inspect only this run's recorded paths; never walk the destination tree.
    let mut after = 0;
    while let Some((rowid, content)) = catalog.next_recovery_content(run, after)? {
        after = rowid;
        let path = config.destination.join(&content.temporary);
        if !content.complete
            || catalog.final_reserved(&content.temporary)?
            || !path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(temp_name)
        {
            continue;
        }
        publication::check_parent(
            &config.destination,
            path.parent().context("temporary has no parent")?,
        )?;
        if path.symlink_metadata().is_ok_and(|m| m.is_file()) {
            std::fs::remove_file(&path)?;
            publication::sync_directory(path.parent().unwrap())?;
        }
    }
    Ok(())
}
