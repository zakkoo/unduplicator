use crate::{
    catalog::{Catalog, Source, path_bytes},
    config::Config,
};
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

fn timestamp(value: std::io::Result<SystemTime>) -> Option<i64> {
    let time = value.ok()?;
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).ok(),
        Err(e) => i64::try_from(e.duration().as_secs())
            .ok()
            .map(|n| -n - i64::from(e.duration().subsec_nanos() > 0)),
    }
}

struct Scan<'a> {
    catalog: &'a mut Catalog,
    config: &'a Config,
    run: i64,
    stop: &'a AtomicBool,
    batch: Vec<Source>,
    count: i64,
    bytes: i64,
    symlinks: i64,
    special: i64,
}

impl Scan<'_> {
    fn flush(&mut self) -> Result<()> {
        let tx = self.catalog.db.transaction()?;
        for row in self.batch.drain(..) {
            tx.execute("INSERT INTO source_files(run,sequence,relative,extension,size,birth,modified) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![self.run,row.sequence,path_bytes(&row.relative),row.relative.extension().map(|e|path_bytes(Path::new(e))),row.size as i64,row.birth,row.modified])?;
        }
        tx.commit()?;
        Ok(())
    }
    fn directory(&mut self, directory: &Path) -> Result<()> {
        ensure!(
            !self.stop.load(Ordering::Relaxed),
            "interrupted during inventory"
        );
        let mut entries = std::fs::read_dir(directory)
            .with_context(|| format!("inventory {}", directory.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut children = Vec::new();
        for entry in entries {
            ensure!(
                !self.stop.load(Ordering::Relaxed),
                "interrupted during inventory"
            );
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("inventory metadata {}", path.display()))?;
            if metadata.file_type().is_symlink() {
                self.symlinks += 1;
            } else if metadata.is_dir() {
                children.push(path);
            } else if metadata.is_file() {
                let size =
                    i64::try_from(metadata.len()).context("file is too large for catalog")?;
                self.bytes = self
                    .bytes
                    .checked_add(size)
                    .context("inventory byte total overflow")?;
                self.batch.push(Source {
                    run: self.run,
                    sequence: self.count,
                    relative: path.strip_prefix(&self.config.source)?.to_owned(),
                    size: metadata.len(),
                    birth: timestamp(metadata.created()),
                    modified: timestamp(metadata.modified()),
                });
                self.count += 1;
                if self.batch.len() == 256 {
                    self.flush()?;
                }
            } else {
                self.special += 1;
            }
        }
        for child in children {
            self.directory(&child)?;
        }
        Ok(())
    }
}

/// Only metadata and directory entries are opened. No regular-file content reads.
pub fn inventory(
    catalog: &mut Catalog,
    config: &Config,
    run: i64,
    stop: &AtomicBool,
) -> Result<()> {
    if catalog.phase(run)? != "inventory" {
        return Ok(());
    }
    catalog
        .db
        .execute("DELETE FROM source_files WHERE run=?1", [run])?;
    let mut scan = Scan {
        catalog,
        config,
        run,
        stop,
        batch: Vec::with_capacity(256),
        count: 0,
        bytes: 0,
        symlinks: 0,
        special: 0,
    };
    scan.directory(&config.source)?;
    scan.flush()?;
    let tx = scan.catalog.db.transaction()?;
    tx.execute("UPDATE runs SET phase='import',total_files=?1,total_bytes=?2,skipped_symlinks=?3,skipped_special=?4,next_position=0,error=NULL WHERE id=?5",
        params![scan.count,scan.bytes,scan.symlinks,scan.special,run])?;
    tx.commit()?;
    Ok(())
}
