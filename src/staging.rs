use crate::publication::BUFFER_SIZE;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

pub const MAX_RESERVATIONS: usize = 4;
pub struct Budget {
    limit: u64,
    state: Mutex<(u64, usize, bool)>,
    changed: Condvar,
    pub peak: AtomicU64,
}
impl Budget {
    pub fn new(limit: u64) -> Arc<Self> {
        Arc::new(Self {
            limit,
            state: Mutex::new((0, 0, false)),
            changed: Condvar::new(),
            peak: AtomicU64::new(0),
        })
    }
    pub fn reserve(self: &Arc<Self>, size: u64, stop: &AtomicBool) -> Result<Reservation> {
        ensure!(size <= self.limit, "reservation exceeds budget");
        let mut state = self.state.lock().unwrap();
        while state.0 + size > self.limit || state.1 >= MAX_RESERVATIONS {
            ensure!(
                !state.2 && !stop.load(Ordering::Relaxed),
                "interrupted waiting for staging budget"
            );
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        ensure!(
            !state.2 && !stop.load(Ordering::Relaxed),
            "interrupted waiting for staging budget"
        );
        state.0 += size;
        state.1 += 1;
        self.peak.fetch_max(state.0, Ordering::Relaxed);
        Ok(Reservation {
            budget: self.clone(),
            size,
            stage: None,
        })
    }
    pub fn used(&self) -> (u64, usize) {
        let state = self.state.lock().unwrap();
        (state.0, state.1)
    }
    pub fn cancel(&self) {
        self.state.lock().unwrap().2 = true;
        self.changed.notify_all();
    }
}
pub struct Reservation {
    budget: Arc<Budget>,
    size: u64,
    stage: Option<std::path::PathBuf>,
}
impl Reservation {
    pub fn track_stage(&mut self, stage: Option<&Path>) {
        self.stage = stage.map(Path::to_owned);
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().unwrap();
        // An error can leave an owned stage on disk. Stop admission before
        // releasing its reservation, so physical bytes cannot exceed the cap.
        if self
            .stage
            .as_ref()
            .is_some_and(|p| match p.symlink_metadata() {
                Ok(_) => true,
                Err(e) => e.kind() != std::io::ErrorKind::NotFound,
            })
        {
            state.2 = true;
        }
        state.0 -= self.size;
        state.1 -= 1;
        self.budget.changed.notify_all();
    }
}

pub fn hash(
    path: &Path,
    stage: Option<&Path>,
    stop: &AtomicBool,
    bytes: &AtomicU64,
) -> Result<(String, u64)> {
    hash_with_progress(path, stage, stop, bytes, &mut || Ok(()))
}

pub fn hash_with_progress(
    path: &Path,
    stage: Option<&Path>,
    stop: &AtomicBool,
    bytes: &AtomicU64,
    on_read: &mut dyn FnMut() -> Result<()>,
) -> Result<(String, u64)> {
    let mut input = File::open(path).with_context(|| format!("content read {}", path.display()))?;
    let mut output = stage
        .map(|path| OpenOptions::new().write(true).create_new(true).open(path))
        .transpose()?;
    let mut hasher = Sha256::new();
    let mut size = 0;
    let mut buffer = vec![0; BUFFER_SIZE];
    loop {
        ensure!(!stop.load(Ordering::Relaxed), "interrupted during read");
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        size += count as u64;
        bytes.fetch_add(count as u64, Ordering::Relaxed);
        on_read()?;
        if let Some(output) = &mut output {
            output.write_all(&buffer[..count])?;
        }
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_budget_and_zero_byte_bounds() {
        let budget = Budget::new(10);
        let stop = Arc::new(AtomicBool::new(false));
        let reservation = budget.reserve(10, &stop).unwrap();
        let (started_rx_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let other = budget.clone();
        let other_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            started_rx_tx.send(()).unwrap();
            let r = other.reserve(1, &other_stop).unwrap();
            done_tx.send(()).unwrap();
            drop(r);
        });
        started_rx.recv().unwrap();
        assert_eq!(budget.used(), (10, 1));
        assert!(done_rx.try_recv().is_err());
        drop(reservation);
        done_rx.recv().unwrap();
        worker.join().unwrap();
        assert_eq!(budget.peak.load(Ordering::Relaxed), 10);
        let reservations = (0..MAX_RESERVATIONS)
            .map(|_| budget.reserve(0, &stop).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(budget.used(), (0, MAX_RESERVATIONS));
        budget.cancel();
        assert!(budget.reserve(0, &stop).is_err());
        drop(reservations);
        assert_eq!(budget.used(), (0, 0));
    }
}
