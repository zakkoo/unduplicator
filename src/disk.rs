use anyhow::Result;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Gates {
    source: Arc<Mutex<()>>,
    destination: Arc<Mutex<()>>,
    stage_source: bool,
    stage_destination: bool,
    pub independent: bool,
}
impl Gates {
    pub fn discover(source: &Path, destination: &Path, staging: &Path) -> Self {
        Self::from_ids(identity(source), identity(destination), identity(staging))
    }
    pub fn from_ids(
        source: Option<String>,
        destination: Option<String>,
        staging: Option<String>,
    ) -> Self {
        let separate =
            source.is_some() && destination.is_some() && staging.is_some() && source != destination;
        let source_gate = Arc::new(Mutex::new(()));
        Self {
            destination: if separate {
                Arc::new(Mutex::new(()))
            } else {
                source_gate.clone()
            },
            source: source_gate,
            stage_source: staging == source,
            stage_destination: staging == destination,
            independent: separate && staging != source && staging != destination,
        }
    }
    pub fn access<T>(
        &self,
        source: bool,
        destination: bool,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let _s = source.then(|| self.source.lock().unwrap());
        let _d = (destination && !(source && Arc::ptr_eq(&self.source, &self.destination)))
            .then(|| self.destination.lock().unwrap());
        f()
    }
    pub fn read<T>(&self, staged: bool, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.access(true, staged && self.stage_destination, f)
    }
    pub fn write<T>(&self, oversized: bool, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.access(oversized || self.stage_source, true, f)
    }
    pub fn metadata<T>(&self, oversized: bool, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.access(
            oversized || self.stage_source,
            !oversized && self.stage_destination,
            f,
        )
    }
}

fn identity(path: &Path) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let mut existing = path;
        while !existing.exists() {
            existing = existing.parent()?;
        }
        let device = std::fs::metadata(existing).ok()?.dev();
        let sys = Path::new("/sys/dev/block")
            .join(format!("{}:{}", libc::major(device), libc::minor(device)))
            .canonicalize()
            .ok()?;
        let mut disk = sys;
        if disk.join("partition").exists() {
            disk = disk.parent()?.to_owned();
        }
        let slaves = std::fs::read_dir(disk.join("slaves"))
            .ok()?
            .collect::<std::io::Result<Vec<_>>>()
            .ok()?;
        if !slaves.is_empty() || disk.to_string_lossy().contains("/virtual/") {
            return None;
        }
        Some(disk.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_overlap_and_shared_serialization() {
        use std::sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let gates = Gates::from_ids(
            Some("source".into()),
            Some("destination".into()),
            Some("ssd".into()),
        );
        let barrier = Arc::new(Barrier::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for source in [true, false] {
                let gates = &gates;
                let barrier = barrier.clone();
                let active = active.clone();
                scope.spawn(move || {
                    gates
                        .access(source, !source, || {
                            active.fetch_add(1, Ordering::SeqCst);
                            barrier.wait();
                            assert_eq!(active.load(Ordering::SeqCst), 2);
                            barrier.wait();
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        })
                        .unwrap()
                });
            }
        });
        for ids in [
            (None, None, None),
            (Some("same".into()), Some("same".into()), Some("ssd".into())),
            (
                Some("source".into()),
                Some("dest".into()),
                Some("source".into()),
            ),
            (
                Some("source".into()),
                Some("dest".into()),
                Some("dest".into()),
            ),
        ] {
            let gates = Gates::from_ids(ids.0, ids.1, ids.2);
            let active = AtomicUsize::new(0);
            let peak = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for source in [true, false] {
                    let gates = &gates;
                    let active = &active;
                    let peak = &peak;
                    scope.spawn(move || {
                        for _ in 0..100 {
                            gates
                                .access(
                                    source || (!source && gates.stage_source),
                                    !source || (source && gates.stage_destination),
                                    || {
                                        let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                                        peak.fetch_max(n, Ordering::SeqCst);
                                        std::thread::yield_now();
                                        active.fetch_sub(1, Ordering::SeqCst);
                                        Ok(())
                                    },
                                )
                                .unwrap();
                        }
                    });
                }
            });
            assert_eq!(peak.load(Ordering::SeqCst), 1);
        }
    }
}
