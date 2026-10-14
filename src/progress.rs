use crate::{catalog::Catalog, pipeline::Counters};
use anyhow::Result;
use rusqlite::{Connection, params};
use std::{
    io::{IsTerminal, Write},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub done: u64,
    pub total: u64,
    pub total_source: u64,
    pub source_read: u64,
    pub destination_written: u64,
    pub verification_read: u64,
    pub recovery_read: u64,
    pub source_remaining: u64,
    pub oversized_remaining: u64,
    pub fallback_remaining: u64,
    pub known_unique: u64,
    pub destination_remaining: u64,
    pub observed: u64,
    pub unique_observed: u64,
    pub verification_total: Option<u64>,
}
impl Snapshot {
    pub fn load(
        db: &Connection,
        run: i64,
        counters: &Counters,
        budget: u64,
        verification_total: Option<u64>,
    ) -> Result<Self> {
        let (total, total_source): (i64, i64) = db.query_row(
            "SELECT total_files,total_bytes FROM runs WHERE id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let done:i64=db.query_row("SELECT COUNT(*) FROM source_files s JOIN contents c ON s.digest=c.digest WHERE s.run=?1 AND c.state='complete'",[run],|r|r.get(0))?;
        let (unseen,oversized):(i64,i64)=db.query_row("SELECT COALESCE(SUM(size),0),COALESCE(SUM(CASE WHEN size>?2 THEN size ELSE 0 END),0) FROM source_files WHERE run=?1 AND digest IS NULL",params![run,budget as i64],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let (known,pending,fallback):(i64,i64,i64)=db.query_row("SELECT COALESCE(SUM(size),0),COALESCE(SUM(CASE WHEN state!='complete' THEN size ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN state!='complete' AND stage IS NULL THEN size ELSE 0 END),0) FROM contents WHERE digest IN (SELECT digest FROM source_files WHERE run=?1)",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let source_read = counters.source.load(Ordering::Relaxed);
        let destination_written = counters.destination.load(Ordering::Relaxed);
        let unique_observed = counters.unique.load(Ordering::Relaxed);
        let uncommitted =
            destination_written.saturating_sub(unique_observed.saturating_sub(pending as u64));
        Ok(Self {
            done: done as u64,
            total: total as u64,
            total_source: total_source as u64,
            source_read,
            destination_written,
            verification_read: counters.verification.load(Ordering::Relaxed),
            recovery_read: counters.recovery.load(Ordering::Relaxed),
            source_remaining: unseen as u64,
            oversized_remaining: oversized as u64,
            fallback_remaining: fallback as u64,
            known_unique: known as u64,
            destination_remaining: (pending as u64).saturating_sub(uncommitted),
            observed: counters.observed.load(Ordering::Relaxed),
            unique_observed,
            verification_total,
        })
    }
    pub fn line(&self, eta: Option<Duration>) -> String {
        let estimate = eta
            .map(|d| format!("~{}s", d.as_secs()))
            .unwrap_or_else(|| "Estimating".into());
        if let Some(total) = self.verification_total {
            format!(
                "verification bytes {}/{total}; ETA {estimate}",
                self.verification_read
            )
        } else {
            format!(
                "{}/{} done; source bytes {}; destination bytes {}/{} known unique; recovery bytes {}; ETA {estimate}",
                self.done,
                self.total,
                self.source_read,
                self.destination_written,
                self.known_unique,
                self.recovery_read
            )
        }
    }
}

#[derive(Default)]
pub struct Estimator {
    last_time: Option<Duration>,
    last_bytes: [u64; 3],
    rates: [Option<f64>; 3],
}
impl Estimator {
    pub fn sample(&mut self, time: Duration, snapshot: &Snapshot) {
        let bytes = [
            snapshot.source_read,
            snapshot.destination_written,
            snapshot.verification_read,
        ];
        if let Some(previous) = self.last_time {
            let elapsed = time.saturating_sub(previous).as_secs_f64();
            if elapsed > 0.0 {
                for (i, value) in bytes.iter().enumerate() {
                    let delta = value.saturating_sub(self.last_bytes[i]);
                    if delta > 0 {
                        let sample = delta as f64 / elapsed;
                        self.rates[i] =
                            Some(self.rates[i].map_or(sample, |rate| rate * 0.7 + sample * 0.3));
                    }
                }
            }
        }
        self.last_time = Some(time);
        self.last_bytes = bytes;
    }
    pub fn eta(&self, snapshot: &Snapshot, independent: bool) -> Option<Duration> {
        let demand = |bytes: f64, index: usize| -> Option<f64> {
            if bytes == 0.0 {
                Some(0.0)
            } else {
                Some(bytes / self.rates[index]?)
            }
        };
        if let Some(total) = snapshot.verification_total {
            self.rates[2]?;
            return Some(Duration::from_secs_f64(demand(
                total.saturating_sub(snapshot.verification_read) as f64,
                2,
            )?));
        }
        if self.rates[0].is_none() && self.rates[1].is_none() {
            return None;
        }
        let fraction = if snapshot.observed > 0 {
            (snapshot.unique_observed as f64 / snapshot.observed as f64).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let source = demand(
            snapshot.source_remaining as f64
                + snapshot.fallback_remaining as f64
                + snapshot.oversized_remaining as f64 * fraction,
            0,
        )?;
        let destination = demand(
            snapshot.destination_remaining as f64 + snapshot.source_remaining as f64 * fraction,
            1,
        )?;
        Some(Duration::from_secs_f64(if independent {
            source.max(destination)
        } else {
            source + destination
        }))
    }
}

pub struct Reporter {
    run: i64,
    budget: u64,
    independent: bool,
    verification_total: Option<u64>,
    start: Instant,
    last_render: Option<Duration>,
    estimator: Estimator,
    terminal: bool,
}
impl Reporter {
    pub fn new(run: i64, budget: u64, independent: bool, verification_total: Option<u64>) -> Self {
        Self {
            run,
            budget,
            independent,
            verification_total,
            start: Instant::now(),
            last_render: None,
            estimator: Estimator::default(),
            terminal: std::io::stdout().is_terminal(),
        }
    }
    pub fn update(&mut self, catalog: &Catalog, counters: &Counters, force: bool) -> Result<()> {
        let time = self.start.elapsed();
        if !force
            && self
                .last_render
                .is_some_and(|last| time.saturating_sub(last) < Duration::from_millis(250))
        {
            return Ok(());
        }
        let snapshot = Snapshot::load(
            &catalog.db,
            self.run,
            counters,
            self.budget,
            self.verification_total,
        )?;
        self.estimator.sample(time, &snapshot);
        let line = snapshot.line(self.estimator.eta(&snapshot, self.independent));
        if self.terminal {
            print!("\r\x1b[2K{line}");
            std::io::stdout().flush()?;
        } else {
            println!("{line}");
        }
        self.last_render = Some(time);
        Ok(())
    }
}
impl Drop for Reporter {
    fn drop(&mut self) {
        if self.terminal {
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_weighted_eta_overlap_serialization_duplicates_and_backlog() {
        let mut estimator = Estimator::default();
        let mut snapshot = Snapshot {
            total: 100,
            source_remaining: 1000,
            destination_remaining: 500,
            observed: 1000,
            unique_observed: 500,
            ..Snapshot::default()
        };
        estimator.sample(Duration::ZERO, &snapshot);
        assert!(estimator.eta(&snapshot, true).is_none());
        snapshot.source_read = 1000;
        snapshot.destination_written = 500;
        estimator.sample(Duration::from_secs(10), &snapshot);
        assert_eq!(estimator.eta(&snapshot, true).unwrap().as_secs(), 20);
        assert_eq!(estimator.eta(&snapshot, false).unwrap().as_secs(), 30);
        snapshot.unique_observed = 0;
        assert_eq!(estimator.eta(&snapshot, true).unwrap().as_secs(), 10);
        snapshot.source_remaining = 0;
        snapshot.destination_remaining = 500;
        assert_eq!(estimator.eta(&snapshot, true).unwrap().as_secs(), 10);
        snapshot.total = 1;
        assert_eq!(estimator.eta(&snapshot, true).unwrap().as_secs(), 10); // file count never sets throughput
        snapshot.source_remaining = 100;
        snapshot.oversized_remaining = 100;
        snapshot.unique_observed = 1000;
        assert_eq!(estimator.eta(&snapshot, false).unwrap().as_secs(), 14);
    }
    #[test]
    fn startup_zero_bytes_verification_and_smoothing() {
        let mut estimator = Estimator::default();
        let mut snapshot = Snapshot::default();
        estimator.sample(Duration::ZERO, &snapshot);
        estimator.sample(Duration::from_secs(1), &snapshot);
        assert!(estimator.eta(&snapshot, true).is_none());
        assert!(snapshot.line(None).contains("Estimating"));
        snapshot.verification_total = Some(1000);
        assert!(estimator.eta(&snapshot, true).is_none());
        snapshot.verification_read = 100;
        estimator.sample(Duration::from_secs(2), &snapshot);
        assert_eq!(estimator.eta(&snapshot, true).unwrap().as_secs(), 9);
        snapshot.verification_read = 300;
        estimator.sample(Duration::from_secs(3), &snapshot);
        assert_eq!(estimator.eta(&snapshot, true).unwrap().as_secs(), 5);
        assert_eq!(
            snapshot.line(None),
            "verification bytes 300/1000; ETA Estimating"
        );
    }
}
