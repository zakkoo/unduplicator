use clap::Parser;

fn main() {
    let cli = unduplicator::cli::Cli::parse();
    if let unduplicator::cli::Command::Import {
        source,
        destination,
        catalog,
        staging,
        staging_budget,
        ..
    } = &cli.command
    {
        if let Err(error) = unduplicator::config::Config::validate(
            source,
            destination,
            catalog,
            staging,
            *staging_budget,
        ) {
            eprintln!("invalid configuration: {error:#}");
            std::process::exit(2);
        }
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let interrupted = stop.clone();
    if let Err(error) =
        ctrlc::set_handler(move || interrupted.store(true, std::sync::atomic::Ordering::Relaxed))
    {
        eprintln!("install interruption handler: {error}");
        std::process::exit(1);
    }
    if let Err(error) = unduplicator::app::execute(cli.command, stop.clone()) {
        eprintln!("{error:#}");
        let interrupted = error
            .downcast_ref::<unduplicator::pipeline::Shutdown>()
            .map_or_else(
                || stop.load(std::sync::atomic::Ordering::Relaxed),
                |state| state.interrupted,
            );
        std::process::exit(if interrupted { 130 } else { 1 });
    }
}
