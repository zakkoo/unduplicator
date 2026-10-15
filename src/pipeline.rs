use crate::{
    catalog::{Catalog, Content, Source},
    config::Config,
    disk::Gates,
    naming, publication,
    staging::{self, Budget, Reservation},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, TrySendError},
    },
    time::Duration,
};

#[derive(Default)]
pub struct Counters {
    pub recovery: AtomicU64,
    pub source: AtomicU64,
    pub destination: AtomicU64,
    pub copied_files: AtomicU64,
    pub verification: AtomicU64,
    pub observed: AtomicU64,
    pub unique: AtomicU64,
    pub stage_peak: AtomicU64,
}
pub type Observer = Arc<dyn Fn(&str) -> Result<()> + Send + Sync>;
#[derive(Debug)]
pub struct Shutdown {
    pub interrupted: bool,
}
impl std::fmt::Display for Shutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} pipeline",
            if self.interrupted {
                "interrupted"
            } else {
                "failed"
            }
        )
    }
}
pub fn noop() -> Observer {
    Arc::new(|_| Ok(()))
}

enum Event {
    Staged(Source, String, Option<PathBuf>, Reservation),
    Published(Content),
    Failed(Option<Source>, Option<String>, String),
}
struct Job {
    content: Content,
    oversized: bool,
}

pub fn next_source(catalog: &Catalog, run: i64, after: i64) -> Result<Option<Source>> {
    Ok(catalog.db.query_row("SELECT sequence,relative,size,birth,modified FROM source_files WHERE run=?1 AND sequence>?2 AND digest IS NULL ORDER BY sequence LIMIT 1",
        params![run,after],|r|Ok(Source {run,sequence:r.get(0)?,relative:crate::catalog::native_path(r.get(1)?),size:r.get::<_,i64>(2)? as u64,birth:r.get(3)?,modified:r.get(4)?})).optional()?)
}

/// Bounded worker queues; SQLite ownership and deterministic claims remain here.
pub fn process(
    catalog: &mut Catalog,
    config: &Config,
    run: i64,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
    gates: Gates,
    observer: Observer,
) -> Result<()> {
    std::fs::create_dir_all(&config.staging)?;
    let budget = Budget::new(config.budget);
    let (source_tx, source_rx) = mpsc::sync_channel::<Source>(2);
    let (copy_tx, copy_rx) = mpsc::sync_channel::<Job>(2);
    let (event_tx, event_rx) = mpsc::sync_channel::<Event>(4);
    std::thread::scope(|scope| {
        let source_events = event_tx.clone();
        let source_stop = stop.clone();
        let source_budget = budget.clone();
        let source_gates = gates.clone();
        let source_counters = counters.clone();
        let source_observer = observer.clone();
        let reader = scope.spawn(move || {
            while let Ok(source) = source_rx.recv() {
                if source_stop.load(Ordering::Relaxed) {
                    break;
                }
                let oversized = source.size > config.budget;
                let stage = (!oversized).then(|| {
                    config.staging.join(format!(
                        ".unduplicator-{}-{}.stage",
                        source.run, source.sequence
                    ))
                });
                let result = (|| {
                    let mut reservation = source_budget
                        .reserve(if oversized { 0 } else { source.size }, &source_stop)?;
                    reservation.track_stage(stage.as_deref());
                    let (digest, size) = source_gates.read(!oversized, || {
                        source_observer("source_begin")?;
                        let result = staging::hash(
                            &config.source.join(&source.relative),
                            stage.as_deref(),
                            &source_stop,
                            &source_counters.source,
                        );
                        source_observer("source_end")?;
                        result
                    })?;
                    ensure!(
                        size == source.size,
                        "source size differs from inventory: {}",
                        source.relative.display()
                    );
                    Ok((digest, reservation))
                })();
                let event = match result {
                    Ok((digest, reservation)) => Event::Staged(source, digest, stage, reservation),
                    Err(error) => Event::Failed(Some(source), None, format!("{error:#}")),
                };
                let failed = matches!(&event, Event::Failed(..));
                if source_events.send(event).is_err() {
                    break;
                }
                if failed {
                    while source_rx.recv().is_ok() {}
                    break;
                }
            }
        });
        let destination_events = event_tx.clone();
        let destination_stop = stop.clone();
        let destination_gates = gates.clone();
        let destination_counters = counters.clone();
        let destination_observer = observer.clone();
        let writer = scope.spawn(move || {
            while let Ok(job) = copy_rx.recv() {
                if destination_stop.load(Ordering::Relaxed) {
                    break;
                }
                let input = job.content.stage.as_deref().unwrap_or(&job.content.source);
                let result = destination_gates.write(job.oversized, || {
                    destination_observer("destination_begin")?;
                    if job.oversized {
                        destination_observer("fallback_source_begin")?;
                    }
                    let mut hook = |name: &str| destination_observer(name);
                    let result = publication::publish(
                        &job.content,
                        &config.destination,
                        input,
                        &destination_stop,
                        &mut hook,
                        &mut |bytes| {
                            destination_counters
                                .destination
                                .fetch_add(bytes, Ordering::Relaxed);
                            if job.oversized {
                                destination_counters
                                    .source
                                    .fetch_add(bytes, Ordering::Relaxed);
                            }
                        },
                    );
                    destination_observer("destination_end")?;
                    if job.oversized {
                        destination_observer("fallback_source_end")?;
                    }
                    result
                });
                let event = match result {
                    Ok(()) => {
                        destination_counters
                            .copied_files
                            .fetch_add(1, Ordering::Relaxed);
                        Event::Published(job.content)
                    }
                    Err(error) => {
                        Event::Failed(None, Some(job.content.digest), format!("{error:#}"))
                    }
                };
                let failed = matches!(&event, Event::Failed(..));
                if destination_events.send(event).is_err() {
                    break;
                }
                if failed {
                    while copy_rx.recv().is_ok() {}
                    break;
                }
            }
        });
        drop(event_tx);
        let result = (|| {
            let mut progress =
                crate::progress::Reporter::new(run, config.budget, gates.independent, None);
            progress.update(catalog, &counters, true)?;
            let mut after = -1;
            let mut exhausted = false;
            let mut readers = 0;
            let mut writers = 0;
            let mut copies = VecDeque::new();
            let mut reservations = BTreeMap::new();
            loop {
                progress.update(catalog, &counters, false)?;
                ensure!(
                    !stop.load(Ordering::Relaxed),
                    "interrupted; resume run {run}"
                );
                if !exhausted && readers < 3 {
                    if let Some(source) = next_source(catalog, run, after)? {
                        let sequence = source.sequence;
                        match source_tx.try_send(source) {
                            Ok(()) => {
                                after = sequence;
                                readers += 1;
                            }
                            Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Disconnected(_)) => {
                                anyhow::bail!("source worker stopped")
                            }
                        }
                    } else {
                        exhausted = true;
                    }
                }
                if let Some(job) = copies.pop_front() {
                    match copy_tx.try_send(job) {
                        Ok(()) => writers += 1,
                        Err(TrySendError::Full(job)) => {
                            copies.push_front(job);
                            observer("copy_queue_full")?;
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            anyhow::bail!("destination worker stopped")
                        }
                    }
                }
                if exhausted && readers == 0 && copies.is_empty() && writers == 0 {
                    break;
                }
                match event_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(Event::Staged(source, digest, stage, reservation)) => {
                        readers -= 1;
                        counters.observed.fetch_add(source.size, Ordering::Relaxed);
                        let content = gates.metadata(stage.is_none(), || {
                            naming::decide(catalog, config, &source, &digest, stage.as_deref())
                        })?;
                        let unique = catalog.claim(&source, &content)?;
                        if unique {
                            counters.unique.fetch_add(source.size, Ordering::Relaxed);
                            observer("pending_commit")?;
                            reservations.insert(digest, reservation);
                            copies.push_back(Job {
                                oversized: stage.is_none(),
                                content,
                            });
                        } else {
                            observer("duplicate_dependency")?;
                            if let Some(stage) = stage {
                                std::fs::remove_file(stage)?;
                            }
                            drop(reservation);
                            ensure!(
                                content.complete || reservations.contains_key(&digest),
                                "pending content must be recovered before processing"
                            );
                        }
                    }
                    Ok(Event::Published(content)) => {
                        writers -= 1;
                        let mut hook = |name: &str| observer(name);
                        publication::commit(catalog, &content, &mut hook)?;
                        reservations.remove(&content.digest);
                    }
                    Ok(Event::Failed(source, digest, error)) => {
                        if let Some(source) = source {
                            catalog.db.execute(
                                "UPDATE source_files SET error=?1 WHERE run=?2 AND sequence=?3",
                                params![error, run, source.sequence],
                            )?;
                        }
                        if let Some(digest) = digest {
                            catalog.db.execute(
                                "UPDATE contents SET error=?1 WHERE digest=?2",
                                params![error, digest],
                            )?;
                        }
                        anyhow::bail!("{error}");
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        anyhow::bail!("workers stopped with unfinished work")
                    }
                }
                counters
                    .stage_peak
                    .fetch_max(budget.peak.load(Ordering::Relaxed), Ordering::Relaxed);
            }
            ensure!(
                catalog.done(run)? == catalog.totals(run)?.0,
                "unfinished source dependencies"
            );
            catalog.db.execute(
                "UPDATE runs SET phase='complete',error=NULL WHERE id=?1",
                [run],
            )?;
            progress.update(catalog, &counters, true)?;
            Ok(())
        })();
        let was_interrupted = stop.load(Ordering::Relaxed);
        if result.is_err() {
            stop.store(true, Ordering::Relaxed);
            budget.cancel();
        }
        drop(source_tx);
        drop(copy_tx);
        drop(event_rx);
        reader
            .join()
            .map_err(|_| anyhow::anyhow!("source worker panicked"))?;
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("destination worker panicked"))?;
        counters
            .stage_peak
            .fetch_max(budget.peak.load(Ordering::Relaxed), Ordering::Relaxed);
        result
            .context(Shutdown {
                interrupted: was_interrupted,
            })
            .context(format!("unfinished run {run}"))
    })
}
