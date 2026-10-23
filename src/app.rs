use crate::{
    catalog::Catalog,
    cli::Command,
    config::Config,
    disk::Gates,
    inventory::inventory,
    pipeline::{self, Counters, Observer},
    recovery, report, verification,
};
use anyhow::Result;
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn observer(stop: Arc<AtomicBool>) -> Observer {
    Arc::new(move |name| {
        #[cfg(feature = "test-failpoints")]
        {
            if std::env::var("UNDUPLICATOR_FAILPOINT").ok().as_deref() == Some(name) {
                std::process::exit(99);
            }
            if std::env::var("UNDUPLICATOR_ERRORPOINT").ok().as_deref() == Some(name) {
                anyhow::bail!("injected {name} failure");
            }
            if std::env::var("UNDUPLICATOR_INTERRUPTPOINT").ok().as_deref() == Some(name) {
                stop.store(true, Ordering::Relaxed);
            }
        }
        let _ = (&stop, name);
        Ok(())
    })
}

pub fn execute(command: Command, stop: Arc<AtomicBool>) -> Result<()> {
    match &command {
        Command::Runs { catalog } => return report::list(catalog),
        Command::Report { catalog, run, full } => return report::show(catalog, *run, *full),
        _ => {}
    }
    crate::publication::platform_supported()?;
    let (mut catalog, config, run, verify, verification_only) = match command {
        Command::Import {
            source,
            destination,
            catalog,
            staging,
            staging_budget,
            verify,
        } => {
            let config =
                Config::validate(&source, &destination, &catalog, &staging, staging_budget)?;
            crate::media::preflight(&stop)?;
            let catalog = Catalog::open(&config.catalog, true)?;
            let run = catalog.new_run(&config)?;
            println!("Run ID: {run}");
            std::io::stdout().flush()?;
            (catalog, config, run, verify, false)
        }
        Command::Resume {
            catalog,
            run,
            verify,
        } => {
            let catalog = Catalog::open(&catalog, false)?;
            let config = catalog.config(run)?;
            catalog.associate(&config.destination)?;
            println!("Run ID: {run}");
            (catalog, config, run, verify, false)
        }
        Command::Verify { catalog, run } => {
            let catalog = Catalog::open(&catalog, false)?;
            let config = catalog.config(run)?;
            catalog.associate(&config.destination)?;
            println!("Run ID: {run}");
            (catalog, config, run, true, true)
        }
        Command::Runs { .. } | Command::Report { .. } => unreachable!(),
    };
    let started = std::time::Instant::now();
    let counters = Arc::new(Counters::default());
    let observer = observer(stop.clone());
    let gates = Gates::discover(&config.source, &config.destination, &config.staging);
    let result: Result<()> = (|| {
        if !verification_only {
            catalog.prepare_staging(&config)?;
            inventory(&mut catalog, &config, run, &stop)?;
            let (symlinks, special): (i64, i64) = catalog.db.query_row(
                "SELECT skipped_symlinks,skipped_special FROM runs WHERE id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            println!(
                "Inventory: {} files, {} bytes; skipped {symlinks} symlinks, {special} special files",
                catalog.totals(run)?.0,
                catalog.totals(run)?.1
            );
            recovery::reconcile(
                &mut catalog,
                &config,
                run,
                &stop,
                &counters,
                &gates,
                &observer,
            )?;
            pipeline::process(
                &mut catalog,
                &config,
                run,
                stop.clone(),
                counters.clone(),
                gates,
                observer.clone(),
            )?;
        }
        if verify {
            verification::verify(&mut catalog, &config, run, &stop, &counters, &observer)?;
        }
        println!(
            "complete: {}/{} done; source bytes {}; destination bytes {}; staging peak {}; run {run} ({})",
            catalog.done(run)?,
            catalog.totals(run)?.0,
            counters.source.load(Ordering::Relaxed),
            counters.destination.load(Ordering::Relaxed),
            counters.stage_peak.load(Ordering::Relaxed),
            if verify { "verified" } else { "unverified" }
        );
        Ok(())
    })();
    if let Err(error) = &result {
        if let Err(record_error) = catalog.error(run, &format!("{error:#}")) {
            eprintln!("could not record run error: {record_error:#}");
        }
        let outcome = if catalog
            .phase(run)
            .is_ok_and(|phase| phase == "verification-failed")
        {
            "verification-failed"
        } else {
            "unfinished"
        };
        eprintln!(
            "{outcome} run {run}: {error:#}\nResume: unduplicator resume --catalog {} --run {run}",
            catalog.path.display()
        );
    }
    let outcome = if result.is_ok() {
        "success"
    } else if result.as_ref().err().is_some_and(|error| {
        error
            .downcast_ref::<pipeline::Shutdown>()
            .map_or_else(|| stop.load(Ordering::Relaxed), |state| state.interrupted)
    }) {
        "interrupted"
    } else {
        "failed"
    };
    let review = report::save(
        &catalog,
        run,
        outcome,
        &counters,
        started.elapsed(),
        &observer,
    );
    match review {
        Ok(text) => print!("\n{text}"),
        Err(error) if result.is_err() => eprintln!("could not save run report: {error:#}"),
        Err(error) => return Err(error.context("could not save run report")),
    }
    result
}
