use crate::{
    catalog::{Catalog, native_path},
    pipeline::Counters,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{fmt::Write, path::Path, sync::atomic::Ordering, time::Duration};

fn bytes(value: i64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut scaled = value as f64;
    let mut index = 0;
    while scaled >= 1024.0 && index < units.len() - 1 {
        scaled /= 1024.0;
        index += 1;
    }
    format!("{scaled:.2} {} ({value} bytes)", units[index])
}
fn safe(value: &str) -> String {
    value.escape_debug().to_string()
}
fn path(value: Vec<u8>) -> String {
    safe(&native_path(value).to_string_lossy())
}
fn has_reports(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='run_reports')",
        [],
        |r| r.get(0),
    )?)
}
fn reader(catalog: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(catalog, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open catalog {}", catalog.display()))?;
    db.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    ensure!(
        (1..=2).contains(&version),
        "unsupported catalog schema version {version}"
    );
    // Keep all queries in one consistent snapshot, including during an import.
    db.execute_batch("BEGIN")?;
    Ok(db)
}

pub fn list(catalog: &Path) -> Result<()> {
    let db = reader(catalog)?;
    let sql = if has_reports(&db)? {
        "SELECT r.id,r.phase,r.total_files,r.total_bytes,r.source,p.generated_at FROM runs r LEFT JOIN run_reports p ON p.run=r.id ORDER BY r.id DESC"
    } else {
        "SELECT id,phase,total_files,total_bytes,source,NULL FROM runs ORDER BY id DESC"
    };
    let mut statement = db.prepare(sql)?;
    let mut rows = statement.query([])?;
    println!("RUN\tPHASE\tFILES\tBYTES\tREPORT SAVED (UTC)\tSOURCE");
    let mut count = 0;
    while let Some(row) = rows.next()? {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            row.get::<_, i64>(0)?,
            safe(&row.get::<_, String>(1)?),
            row.get::<_, Option<i64>>(2)?
                .map_or_else(|| "?".into(), |v| v.to_string()),
            row.get::<_, Option<i64>>(3)?
                .map_or_else(|| "?".into(), |v| v.to_string()),
            row.get::<_, Option<String>>(5)?
                .unwrap_or_else(|| "none".into()),
            path(row.get(4)?)
        );
        count += 1;
    }
    if count == 0 {
        println!("No runs recorded.");
    }
    Ok(())
}

pub fn show(catalog: &Path, run: i64, full: bool) -> Result<()> {
    let db = reader(catalog)?;
    // Completion and verdict always come from one current database snapshot.
    // Saved text contributes only explicitly historical invocation statistics.
    let mut detail = generate(&db, run)?;
    let mut has_saved = false;
    if has_reports(&db)? {
        let saved: Option<(String, String)> = db
            .query_row(
                "SELECT generated_at,report FROM run_reports WHERE run=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((generated, saved)) = saved {
            has_saved = true;
            if let Some((_, metrics)) = saved.split_once(METRICS_HEADING) {
                detail.push_str(METRICS_HEADING);
                detail.push_str(metrics);
            } else if let Some((_, metrics)) = saved.split_once("\nLAST INVOCATION I/O") {
                // Read older saved statistics without reusing their stale verdict.
                detail.push_str(METRICS_HEADING);
                writeln!(detail, "Invocation statistics saved: {generated}")?;
                if let Some(outcome) = field(&saved, "Last invocation: ") {
                    writeln!(detail, "Last saved invocation: {outcome}")?;
                }
                if let Some((_, values)) = metrics.split_once('\n') {
                    detail.push_str(values);
                }
                detail.push_str(METRICS_NOTE);
            }
        }
    }
    if !has_saved {
        println!("No saved report; current catalog snapshot (not persisted).");
    }
    print!("{}", if full { detail } else { concise(&detail) });
    Ok(())
}

const METRICS_HEADING: &str = "\nLAST SAVED INVOCATION (performance statistics; not cumulative)\n";
const METRICS_NOTE: &str = "  These statistics may predate current progress; abrupt termination can leave activity unrecorded.\n";

pub fn save(
    catalog: &Catalog,
    run: i64,
    outcome: &str,
    counters: &Counters,
    elapsed: Duration,
    observer: &crate::pipeline::Observer,
) -> Result<String> {
    let generated = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut report = generate(&catalog.db, run)?;
    report.push_str(METRICS_HEADING);
    writeln!(
        report,
        "Invocation statistics saved: {generated}\nLast saved invocation: {outcome}"
    )?;
    writeln!(report, "  Elapsed: {:.2} seconds", elapsed.as_secs_f64())?;
    writeln!(
        report,
        "  Destination files copied: {}",
        counters.copied_files.load(Ordering::Relaxed)
    )?;
    for (label, value) in [
        ("Source bytes read", counters.source.load(Ordering::Relaxed)),
        (
            "Destination bytes written",
            counters.destination.load(Ordering::Relaxed),
        ),
        (
            "Recovery bytes read",
            counters.recovery.load(Ordering::Relaxed),
        ),
        (
            "Verification bytes read",
            counters.verification.load(Ordering::Relaxed),
        ),
        ("Staging peak", counters.stage_peak.load(Ordering::Relaxed)),
    ] {
        writeln!(report, "  {label}: {value} bytes")?;
    }
    report.push_str(METRICS_NOTE);
    let tx = catalog.db.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO run_reports(run,generated_at,report) VALUES(?1,?2,?3)
        ON CONFLICT(run) DO UPDATE SET generated_at=excluded.generated_at,report=excluded.report",
        params![run, generated, report],
    )?;
    observer("before_report_commit")?;
    tx.commit()?;
    observer("report_commit")?;
    Ok(concise(&report))
}

fn field<'a>(report: &'a str, label: &str) -> Option<&'a str> {
    report
        .lines()
        .map(str::trim_start)
        .find_map(|line| line.strip_prefix(label))
}

fn count(report: &str, label: &str) -> Option<i64> {
    field(report, label)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn amount(report: &str, label: &str) -> Option<String> {
    let value = field(report, label)?;
    Some(value.split(" (").next()?.to_owned())
}
fn short_bytes(value: i64) -> String {
    bytes(value).split(" (").next().unwrap_or("?").to_owned()
}
fn files(n: i64) -> &'static str {
    if n == 1 { "file" } else { "files" }
}

/// Render a concise view of generated or saved report text.
pub fn concise(report: &str) -> String {
    let Some(verdict) = field(report, "VERDICT: ") else {
        return report.to_owned();
    };
    let Some(run) = field(report, "Run: ") else {
        return report.to_owned();
    };
    let mut short = format!("Run {run}: {verdict}\n");
    if let Some(value) =
        field(report, "Invocation statistics saved: ").or_else(|| field(report, "Report saved: "))
    {
        let _ = writeln!(short, "Invocation statistics saved: {value}");
    }
    if let Some(value) =
        field(report, "Last saved invocation: ").or_else(|| field(report, "Last invocation: "))
    {
        let _ = writeln!(short, "Last saved invocation: {value}");
    }
    if let Some(value) = field(report, "Elapsed: ") {
        let _ = writeln!(short, "Time: {value} (last saved invocation)");
    }
    if let Some(found) = count(report, "Regular files recorded: ") {
        let size = amount(report, "Recorded source size: ").unwrap_or_else(|| "?".into());
        let _ = writeln!(short, "Found: {found} {}, {size}", files(found));
        if field(report, "Inventory: ").is_some_and(|value| value.starts_with("unfinished")) {
            short.push_str("Inventory: unfinished; counts are partial\n");
        }
    }
    if let (Some(ready), Some(total)) = (
        field(report, "Required destination content complete: ")
            .and_then(|s| s.split('/').next()?.parse::<i64>().ok()),
        count(report, "Distinct content groups: "),
    ) {
        let new = count(report, "Distinct groups without earlier-run references: ");
        let reused = count(report, "Distinct groups referenced by earlier runs: ");
        let _ = write!(short, "Destination: {ready}/{total} unique files ready");
        if let (Some(new), Some(reused)) = (new, reused) {
            let _ = write!(
                short,
                " ({new} without earlier-run matches, {reused} seen in earlier runs)"
            );
        }
        short.push('\n');
        if let Some(size) = amount(report, "Completed destination content size: ") {
            let _ = writeln!(
                short,
                "Committed content: {ready} unique {}, {size} (across resumes; includes reused content)",
                files(ready)
            );
        }
    }
    if let Some(extra) = count(report, "Extra copies within this source: ") {
        let redundant =
            amount(report, "Redundant bytes within this source: ").unwrap_or_else(|| "?".into());
        let _ = writeln!(
            short,
            "Duplicates: {extra} extra source copies, {redundant}"
        );
    }
    if let Some(copied) = count(report, "Destination files copied: ") {
        let written = count(report, "Destination bytes written: ").unwrap_or(0);
        let _ = writeln!(
            short,
            "Copied in last saved invocation: {copied} {}, {} written",
            files(copied),
            short_bytes(written)
        );
    } else if let Some(written) = count(report, "Destination bytes written: ") {
        let _ = writeln!(
            short,
            "Written in last saved invocation: {} (file count not recorded)",
            short_bytes(written)
        );
    }
    if field(report, "Elapsed: ").is_some() {
        short.push_str("Saved invocation statistics may predate current progress; crashes can leave activity unrecorded.\n");
    }
    if let Some(pending) = count(report, "Pending destination content: ") {
        if pending > 0 {
            let _ = writeln!(short, "Pending: {pending} unique files");
        }
    }
    if let (Some(symlinks), Some(special)) = (
        count(report, "Skipped symlinks: "),
        count(report, "Skipped special files: "),
    ) {
        if symlinks > 0 || special > 0 {
            let _ = writeln!(
                short,
                "Skipped: {symlinks} symlinks, {special} special files"
            );
        }
    }
    if let (Some(source), Some(content)) = (
        count(report, "Current source errors: "),
        count(report, "Current content errors: "),
    ) {
        let failed = count(report, "Failed: ").unwrap_or(0);
        let _ = writeln!(
            short,
            "Failed: {source} source, {content} content, {failed} verification"
        );
    }
    let mut reasons = false;
    for line in report.lines() {
        if line.starts_with("VERDICT: ") {
            reasons = true;
        } else if reasons && line.starts_with("Run: ") {
            break;
        } else if reasons && line.starts_with("  ") {
            let reason = line.trim();
            if !reason.starts_with("Source files:")
                && !reason.starts_with("Destination groups:")
                && !reason.starts_with("Source bytes:")
            {
                let _ = writeln!(short, "{reason}");
            }
        }
    }
    short.push_str("Details: unduplicator report --full (with the same catalog and run)\n");
    short
}

pub fn generate(db: &Connection, run: i64) -> Result<String> {
    let mut report = String::new();
    let mut statement = db.prepare("SELECT source,destination,staging,budget,phase,total_files,total_bytes,skipped_symlinks,skipped_special,error FROM runs WHERE id=?1")?;
    let mut rows = statement.query([run])?;
    let row = rows.next()?.with_context(|| format!("unknown run {run}"))?;
    let phase: String = row.get(4)?;
    let inventory_complete = phase != "inventory";
    let total: Option<i64> = row.get(5)?;
    let total_bytes: Option<i64> = row.get(6)?;
    let skipped_symlinks: i64 = row.get(7)?;
    let skipped_special: i64 = row.get(8)?;
    writeln!(
        report,
        "UNDUPLICATOR — RUN QUICK CHECK\nRun: {run}\nRecorded phase: {}",
        safe(&phase)
    )?;
    for (label, column) in [("Source", 0), ("Destination", 1), ("Staging", 2)] {
        writeln!(report, "{label}: {}", path(row.get(column)?))?;
    }
    let budget: i64 = row.get(3)?;
    writeln!(report, "Staging budget: {}", bytes(budget))?;
    let run_error: Option<String> = row.get(9)?;
    let (files, size, hashed, hashed_size, done, oversized, source_errors): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(s.size),0),COUNT(s.digest),
        COALESCE(SUM(CASE WHEN s.digest IS NOT NULL THEN s.size ELSE 0 END),0),
        COALESCE(SUM(c.state='complete'),0),COALESCE(SUM(s.size>?2),0),COUNT(s.error)
        FROM source_files s LEFT JOIN contents c ON s.digest=c.digest WHERE s.run=?1",
        params![run, budget],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        },
    )?;
    writeln!(
        report,
        "\nSOURCE INVENTORY\n  Inventory: {}\n  Regular files recorded: {files}\n  Recorded source size: {}",
        if inventory_complete {
            "complete"
        } else {
            "unfinished; counts below are partial"
        },
        bytes(size)
    )?;
    if inventory_complete {
        writeln!(
            report,
            "  Skipped symlinks: {}\n  Skipped special files: {}",
            skipped_symlinks, skipped_special
        )?;
    } else {
        writeln!(
            report,
            "  Skipped entry totals: unavailable until inventory completes"
        )?;
    }
    writeln!(report, "  Files larger than staging budget: {oversized}")?;
    // Sizes and completion states are already committed together with progress.
    // Derive totals here, without extra counters, writes, or per-file flushes.
    let (unique, unique_bytes, complete, pending, content_errors, complete_bytes): (i64,i64,i64,i64,i64,i64) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(size),0),COALESCE(SUM(state='complete'),0),COALESCE(SUM(state!='complete'),0),COUNT(error),COALESCE(SUM(CASE WHEN state='complete' THEN size ELSE 0 END),0)
        FROM contents WHERE digest IN (SELECT digest FROM source_files WHERE run=?1)", [run],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    let (groups, extra, redundant): (i64,i64,i64) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(n-1),0),COALESCE(SUM((n-1)*size),0) FROM
        (SELECT COUNT(*) n,MAX(size) size FROM source_files WHERE run=?1 AND digest IS NOT NULL GROUP BY digest HAVING COUNT(*)>1)",
        [run], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let (shared, shared_bytes): (i64, i64) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(c.size),0) FROM contents c
        WHERE c.digest IN (SELECT digest FROM source_files WHERE run=?1)
        AND EXISTS(SELECT 1 FROM source_files p WHERE p.digest=c.digest AND p.run<?1)",
        [run],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    writeln!(
        report,
        "\nCONTENT AND DUPLICATES\n  Source files with a digest: {hashed}/{files}\n  Distinct content groups: {unique}\n  Distinct content size: {}\n  Groups with multiple source files: {groups}\n  Extra copies within this source: {extra}\n  Redundant bytes within this source: {}\n  Distinct groups referenced by earlier runs: {shared} / {}\n  Distinct groups without earlier-run references: {} / {}\n  Duplicate counts cover hashed files only and can overlap cross-run matches.\n  Earlier-run references are catalog matches, not a destination scan or copy history.",
        bytes(unique_bytes),
        bytes(redundant),
        bytes(shared_bytes),
        unique - shared,
        bytes(unique_bytes - shared_bytes)
    )?;
    writeln!(
        report,
        "\nCOMPLETION\n  Source files mapped to completed content: {done}/{files}\n  Required destination content complete: {complete}/{unique}\n  Pending destination content: {pending}\n  Source files without a digest: {}\n  Current source errors: {source_errors}\n  Current content errors: {content_errors}\n  Current run error: {}",
        files - hashed,
        run_error
            .as_deref()
            .map(safe)
            .unwrap_or_else(|| "none".into())
    )?;

    writeln!(
        report,
        "  Completed destination content size: {}\n  Completion totals span all resumes and include reused content; each digest is counted once.",
        bytes(complete_bytes)
    )?;

    writeln!(report, "\nSOURCE EXTENSIONS (counts include duplicates)")?;
    let mut extensions = db.prepare("SELECT extension,COUNT(*),SUM(size) FROM source_files WHERE run=?1 GROUP BY extension ORDER BY COUNT(*) DESC,extension")?;
    let mut ext_rows = extensions.query([run])?;
    while let Some(ext) = ext_rows.next()? {
        let extension = ext
            .get::<_, Option<Vec<u8>>>(0)?
            .map(path)
            .unwrap_or_else(|| "(none)".into());
        writeln!(
            report,
            "  {extension}: {} files / {}",
            ext.get::<_, i64>(1)?,
            bytes(ext.get(2)?)
        )?;
    }
    writeln!(report, "\nDATE PROVENANCE (distinct content groups)")?;
    let mut dates = db.prepare("SELECT provenance,COUNT(*) FROM contents WHERE digest IN (SELECT digest FROM source_files WHERE run=?1) GROUP BY provenance ORDER BY COUNT(*) DESC,provenance")?;
    let mut date_rows = dates.query([run])?;
    while let Some(date) = date_rows.next()? {
        writeln!(
            report,
            "  {}: {}",
            safe(&date.get::<_, String>(0)?),
            date.get::<_, i64>(1)?
        )?;
    }

    let latest: Option<(i64, String, i64)> = db
        .query_row(
            "SELECT id,state,bytes FROM verification_passes WHERE run=?1 ORDER BY id DESC LIMIT 1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    writeln!(report, "\nLATEST VERIFICATION PASS")?;
    let mut verified = false;
    let mut verification_failed = false;
    if let Some((pass, state, read)) = latest {
        let (checked,passed,failed): (i64,i64,i64) = db.query_row("SELECT COUNT(*),COALESCE(SUM(success=1),0),COALESCE(SUM(success=0),0) FROM verification_results WHERE pass=?1",[pass],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let missing: i64 = db.query_row("SELECT COUNT(*) FROM contents c WHERE c.digest IN (SELECT digest FROM source_files WHERE run=?1) AND NOT EXISTS(SELECT 1 FROM verification_results v WHERE v.pass=?2 AND v.digest=c.digest)",params![run,pass],|r|r.get(0))?;
        writeln!(
            report,
            "  Pass ID: {pass}\n  State: {}\n  Destination files checked: {checked}\n  Passed: {passed}\n  Failed: {failed}\n  Required content without a result: {missing}\n  Recorded verification bytes: {}",
            safe(&state),
            bytes(read)
        )?;
        verified = state == "verified"
            && checked == unique
            && passed == unique
            && failed == 0
            && missing == 0;
        verification_failed = state == "failed" || failed > 0;
        let mut errors = db.prepare("SELECT c.intended,v.error FROM verification_results v JOIN contents c ON c.digest=v.digest WHERE v.pass=?1 AND v.success=0 ORDER BY c.intended")?;
        let mut error_rows = errors.query([pass])?;
        while let Some(error) = error_rows.next()? {
            writeln!(
                report,
                "  FAILURE {}: {}",
                path(error.get(0)?),
                safe(
                    &error
                        .get::<_, Option<String>>(1)?
                        .unwrap_or_else(|| "unspecified".into())
                )
            )?;
        }
    } else {
        writeln!(
            report,
            "  No verification pass recorded; destination bytes not independently checked."
        )?;
    }

    writeln!(
        report,
        "\nLARGEST WITHIN-SOURCE DUPLICATE GROUPS (up to 10 groups, 10 paths each)"
    )?;
    let mut duplicates = db.prepare("SELECT s.digest,COUNT(*),MAX(s.size),c.intended FROM source_files s JOIN contents c ON c.digest=s.digest WHERE s.run=?1 GROUP BY s.digest HAVING COUNT(*)>1 ORDER BY (COUNT(*)-1)*MAX(s.size) DESC,s.digest LIMIT 10")?;
    let mut dup_rows = duplicates.query([run])?;
    while let Some(dup) = dup_rows.next()? {
        let digest: String = dup.get(0)?;
        let count: i64 = dup.get(1)?;
        writeln!(
            report,
            "  SHA-256: {}\n  Copies: {count}; size each: {}\n  Destination: {}",
            safe(&digest),
            bytes(dup.get(2)?),
            path(dup.get(3)?)
        )?;
        let mut paths = db.prepare("SELECT relative FROM source_files WHERE run=?1 AND digest=?2 ORDER BY sequence LIMIT 10")?;
        let mut path_rows = paths.query(params![run, digest])?;
        while let Some(source) = path_rows.next()? {
            writeln!(report, "    {}", path(source.get(0)?))?;
        }
        if count > 10 {
            writeln!(report, "    ... {} more source paths", count - 10)?;
        }
    }
    if groups == 0 {
        writeln!(report, "  None among hashed files.")?;
    }
    if groups > 10 {
        writeln!(report, "  ... {} more duplicate groups", groups - 10)?;
    }
    if source_errors > 0 || content_errors > 0 {
        writeln!(report, "\nCURRENT FILE ERRORS")?;
        for sql in [
            "SELECT relative,error FROM source_files WHERE run=?1 AND error IS NOT NULL ORDER BY sequence",
            "SELECT intended,error FROM contents WHERE digest IN (SELECT digest FROM source_files WHERE run=?1) AND error IS NOT NULL ORDER BY intended",
        ] {
            let mut errors = db.prepare(sql)?;
            let mut error_rows = errors.query([run])?;
            while let Some(error) = error_rows.next()? {
                writeln!(
                    report,
                    "  {}: {}",
                    path(error.get(0)?),
                    safe(&error.get::<_, String>(1)?)
                )?;
            }
        }
    }
    writeln!(report, "\nCHECK RESULT")?;
    if inventory_complete && total == Some(files) && done == files {
        writeln!(
            report,
            "  Every inventoried source file maps to completed content."
        )?;
    } else {
        writeln!(
            report,
            "  Import unfinished; inventory or destination work remains."
        )?;
    }
    if verified {
        writeln!(
            report,
            "  Latest verification passed for all required content at verification time."
        )?;
    }
    writeln!(
        report,
        "  This is a catalog snapshot; it does not recheck current filesystem contents."
    )?;
    let mut failures = Vec::new();
    let mut reviews = Vec::new();
    if run_error.is_some() || source_errors > 0 || content_errors > 0 {
        failures.push("Current run or file errors are recorded.".into());
    }
    if phase == "verification-failed" || verification_failed {
        failures.push("The latest destination verification failed.".into());
    }
    if inventory_complete && (total != Some(files) || total_bytes != Some(size)) {
        failures.push("Inventory file or byte totals do not reconcile.".into());
    }
    if hashed != unique + extra
        || unique != complete + pending
        || hashed_size != unique_bytes + redundant
    {
        failures.push("Content group, source file, or source byte totals do not reconcile.".into());
    }
    if phase == "verified" && !verified {
        failures.push("Verified phase lacks a complete passing verification pass.".into());
    }
    if !inventory_complete || hashed != files || done != files || pending != 0 {
        if matches!(phase.as_str(), "complete" | "verified") {
            failures.push("Completed run has unfinished source or destination work.".into());
        } else {
            reviews.push("Inventory, hashing, or destination work remains unfinished.".into());
        }
    }
    if !verified {
        reviews.push("Destination content has no complete passing verification pass.".into());
    } else if phase != "verified" {
        reviews.push("Run is not currently recorded as verified; verify it again.".into());
    }
    if skipped_symlinks > 0 || skipped_special > 0 {
        reviews.push(format!(
            "Skipped entries need review: {skipped_symlinks} symlinks, {skipped_special} special files."
        ));
    }
    let (verdict, reasons) = if !failures.is_empty() {
        ("FAIL", failures)
    } else if !reviews.is_empty() {
        ("REVIEW", reviews)
    } else {
        (
            "PASS",
            vec!["Inventory, content, and destination verification reconcile.".into()],
        )
    };
    let mut heading = format!(
        "VERDICT: {verdict}\n  Source files: {done}/{files} complete; {hashed}/{files} hashed\n  Destination groups: {complete}/{unique} complete; {pending} pending\n  Source bytes: {hashed_size} hashed = {unique_bytes} distinct + {redundant} repeated\n"
    );
    for reason in reasons {
        writeln!(heading, "  {reason}")?;
    }
    report.insert_str("UNDUPLICATOR — RUN QUICK CHECK\n".len(), &heading);
    Ok(report)
}
