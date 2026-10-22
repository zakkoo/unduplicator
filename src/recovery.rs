use crate::{
    catalog::{Catalog, Content, path_bytes},
    config::Config,
    disk::Gates,
    pipeline::{Counters, Observer},
    publication, staging,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
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

pub fn reconcile(
    catalog: &mut Catalog,
    config: &Config,
    stop: &AtomicBool,
    counters: &Counters,
    gates: &Gates,
    observer: &Observer,
) -> Result<()> {
    let contents = catalog.all_contents()?;
    let final_paths = contents
        .iter()
        .map(|c| config.destination.join(&c.intended))
        .collect::<HashSet<_>>();
    let recovered = contents
        .iter()
        .filter_map(|content| content.stage.as_ref())
        .filter(|p| p.starts_with(&config.staging))
        .filter_map(|p| std::fs::symlink_metadata(p).ok())
        .map(|m| m.len())
        .sum::<u64>();
    counters.stage_peak.fetch_max(recovered, Ordering::Relaxed);
    // No new reservations are accepted until all recovered work has drained.
    for content in contents {
        ensure!(!stop.load(Ordering::Relaxed), "interrupted during recovery");
        let mut hook = |name: &str| observer(name);
        if content.complete {
            publication::cleanup_stage(catalog, &content, &mut hook)?;
            continue;
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
                    matches(&final_path, &content, stop, &counters.recovery)?,
                    "conflicting final content: {}; pending copy remains unfinished",
                    final_path.display()
                );
                std::fs::File::open(&final_path)?
                    .sync_all()
                    .context("adopted final file flush failed")?;
                publication::sync_parents(
                    final_path.parent().context("final has no parent")?,
                    &config.destination,
                )?;
                hook("directory_sync")?;
                if !final_paths.contains(&temporary) && temporary.symlink_metadata().is_ok() {
                    std::fs::remove_file(&temporary)?;
                    publication::sync_directory(
                        temporary.parent().context("temporary has no parent")?,
                    )?;
                }
            } else if final_paths.contains(&temporary) {
                anyhow::bail!(
                    "recorded temporary aliases a reserved final path: {}; cannot remove or replace it",
                    temporary.display()
                );
            } else if matches(&temporary, &content, stop, &counters.recovery)? {
                publication::publish_existing_temporary(&content, &config.destination, &mut hook)?;
            } else {
                if temporary.symlink_metadata().is_ok() {
                    std::fs::remove_file(&temporary).context("remove invalid tool temporary")?;
                }
                let valid_stage = content
                    .stage
                    .as_deref()
                    .map(|stage| matches(stage, &content, stop, &counters.recovery))
                    .transpose()?
                    .unwrap_or(false);
                let input = if valid_stage {
                    content.stage.as_deref().unwrap()
                } else {
                    ensure!(
                        matches(&content.source, &content, stop, &counters.source)?,
                        "representative source missing or differs from pending digest: {}",
                        content.source.display()
                    );
                    content.source.as_path()
                };
                gates.write(!valid_stage, || {
                    publication::publish(
                        &content,
                        &config.destination,
                        input,
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
            publication::commit(catalog, &content, &mut hook)
        })();
        if let Err(error) = result {
            catalog.db.execute(
                "UPDATE contents SET error=?1 WHERE digest=?2",
                rusqlite::params![format!("{error:#}"), content.digest],
            )?;
            return Err(error);
        }
    }
    cleanup_orphans(catalog, config)?;
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
pub fn cleanup_orphans(catalog: &Catalog, config: &Config) -> Result<()> {
    let contents = catalog.all_contents()?;
    let stages = contents
        .iter()
        .filter_map(|c| c.stage.clone())
        .collect::<HashSet<_>>();
    let temporaries = contents
        .iter()
        .filter(|c| !c.complete)
        .map(|c| config.destination.join(&c.temporary))
        .collect::<HashSet<_>>();
    let owned_temporaries = contents
        .iter()
        .map(|c| config.destination.join(&c.temporary))
        .collect::<HashSet<_>>();
    let finals = contents
        .iter()
        .map(|c| config.destination.join(&c.intended))
        .collect::<HashSet<_>>();
    if config.staging.is_dir() {
        for entry in std::fs::read_dir(&config.staging)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() || stages.contains(&path) {
                continue;
            }
            if let Some((run, _)) = entry.file_name().to_str().and_then(stage_name) {
                // Prove this catalog created the naming namespace in this staging root.
                let owned: bool = catalog.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND staging=?2)",
                    rusqlite::params![run, path_bytes(&config.staging)],
                    |r| r.get(0),
                )?;
                if owned {
                    std::fs::remove_file(path)?;
                }
            }
        }
    }
    // Hard-link publication can leave a redundant tool temporary after adoption.
    // Only catalog-owned paths are candidates, so check them directly instead of
    // walking the destination (which may hold unreadable directories like lost+found).
    for path in &owned_temporaries {
        if temporaries.contains(path)
            || finals.contains(path)
            || !path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(temp_name)
            || !path.symlink_metadata().is_ok_and(|m| m.is_file())
        {
            continue;
        }
        std::fs::remove_file(path)?;
        if let Some(parent) = path.parent() {
            publication::sync_directory(parent)?;
        }
    }
    Ok(())
}
