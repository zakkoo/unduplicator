use crate::{
    catalog::Catalog,
    config::Config,
    pipeline::{Counters, Observer},
    progress::Reporter,
    publication, staging,
};
use anyhow::{Result, ensure};
use rusqlite::params;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn verify(
    catalog: &mut Catalog,
    config: &Config,
    run: i64,
    stop: &AtomicBool,
    counters: &Counters,
    observer: &Observer,
) -> Result<()> {
    ensure!(
        catalog.phase(run)? != "inventory" && catalog.done(run)? == catalog.totals(run)?.0,
        "cannot verify unfinished import; resume run {run} first"
    );
    let contents = catalog.contents_for_run(run)?;
    let total = contents.iter().map(|c| c.size).sum::<u64>();
    let tx = catalog.db.transaction()?;
    tx.execute("INSERT INTO verification_passes(run) VALUES(?1)", [run])?;
    let pass = tx.last_insert_rowid();
    tx.execute(
        "UPDATE runs SET phase='verifying',error=NULL WHERE id=?1",
        [run],
    )?;
    tx.commit()?;
    counters.verification.store(0, Ordering::Relaxed);
    let mut reporter = Reporter::new(run, config.budget, false, Some(total));
    reporter.update(catalog, counters, true)?;
    let mut failed = 0;
    for content in contents {
        ensure!(
            !stop.load(Ordering::Relaxed),
            "interrupted verification pass {pass}; request a fresh full pass for run {run}"
        );
        observer("verification_begin")?;
        let path = config.destination.join(&content.intended);
        let result = publication::check_parent(
            &config.destination,
            path.parent().expect("intended path has a parent"),
        )
        .and_then(|()| {
            staging::hash_with_progress(&path, None, stop, &counters.verification, &mut || {
                reporter.update(catalog, counters, false)
            })
        })
        .and_then(|(digest, size)| {
            ensure!(
                digest == content.digest && size == content.size,
                "destination digest/size mismatch: {}",
                path.display()
            );
            Ok(())
        });
        ensure!(
            !stop.load(Ordering::Relaxed),
            "interrupted verification pass {pass}; request a fresh full pass for run {run}"
        );
        let error = result.err().map(|e| format!("{}: {e:#}", path.display()));
        if let Some(error) = &error {
            failed += 1;
            eprintln!("verification error: {error}");
        }
        let tx = catalog.db.transaction()?;
        tx.execute(
            "INSERT INTO verification_results(pass,digest,success,error) VALUES(?1,?2,?3,?4)",
            params![pass, content.digest, error.is_none(), error],
        )?;
        tx.execute(
            "UPDATE verification_passes SET bytes=?1 WHERE id=?2",
            params![counters.verification.load(Ordering::Relaxed) as i64, pass],
        )?;
        observer("before_verification_result_commit")?;
        tx.commit()?;
        reporter.update(catalog, counters, false)?;
        observer("verification_result")?;
    }
    ensure!(
        !stop.load(Ordering::Relaxed),
        "interrupted verification; request a fresh full pass for run {run}"
    );
    let tx = catalog.db.transaction()?;
    tx.execute(
        "UPDATE verification_passes SET state=?1 WHERE id=?2",
        params![if failed == 0 { "verified" } else { "failed" }, pass],
    )?;
    tx.execute(
        "UPDATE runs SET phase=?1,error=?2 WHERE id=?3",
        params![
            if failed == 0 {
                "verified"
            } else {
                "verification-failed"
            },
            (failed > 0).then(|| format!("{failed} verification failures")),
            run
        ],
    )?;
    tx.commit()?;
    observer("verification_complete")?;
    reporter.update(catalog, counters, true)?;
    ensure!(
        failed == 0,
        "verification-failed: {failed} required files; run {run}"
    );
    Ok(())
}
