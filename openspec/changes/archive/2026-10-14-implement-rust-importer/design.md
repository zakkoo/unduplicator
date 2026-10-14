# Design

## Context

The repository currently contains README.md and OpenSpec artifacts, with no application, Cargo manifest, schema, or tests. See proposal.md for motivation and the four capability specs for behavior. The expected deployment has a local SSD for catalog/staging and separate slow source and destination HDDs. Sources and completed destination content are assumed stable; the catalog survives across runs.

## Goals / Non-Goals

**Goals:** Make filesystem/catalog ordering auditable, preserve deterministic decisions while overlapping disk work, and bound both staged bytes and queued work. Exercise failure boundaries using real files and SQLite.

**Non-Goals:** Detect external changes, reconstruct a lost catalog, optimize physical block order, or promise power-loss durability on storage that does not honor flush operations. Unsupported durability operations produce errors rather than successful completion.

## Decisions

### 1. Synchronous Rust pipeline with explicit ownership

Create a Cargo binary and library with CLI, inventory, catalog, staging, media, naming, publication, recovery, verification, progress, and report modules. Use clap for arguments, sha2 for streaming hashing, rusqlite with bundled SQLite, serde_json for metadata responses, and a date parser with explicit calendar validation. Pin resolved dependencies in Cargo.lock at implementation time.

Use a coordinator owning the SQLite connection, one source worker, and one destination worker with bounded channels. Perform deterministic digest/path decisions in inventory order before sending copy jobs. This is simpler to audit than an async runtime or concurrent SQLite writers. Destination events carry copy progress and publication results; only the coordinator commits completion and releases staging.

CLI shape: `import --source PATH --destination PATH --catalog PATH --staging PATH --staging-budget BYTES [--verify]`, `resume --catalog PATH --run ID [--verify]`, `verify --catalog PATH --run ID`, `runs --catalog PATH`, and `report --catalog PATH --run ID`. Print the run ID immediately after creation. Store normalized configuration with the run; resume uses it rather than accepting silent root changes. Use exit 0 for successful requested work, 1 for work/verification failures or unknown report run IDs, 2 for invalid invocation, and 130 for interruption.

Validate roots before inventory: destination, staging, and catalog storage must not be inside the source tree or overlap each other in ways that import generated files. Do not follow source symlinks; report skipped symlinks and special files separately from inventoried regular files. Preserve native path bytes in the catalog, including non-UTF-8 Unix names. Display paths lossily only for messages. Inventory permission errors leave inventory unfinished instead of silently omitting inaccessible files.

### 2. Persistent state separates source observations and unique content

Use schema-versioned tables:

- `runs`: roots/configuration, phase, finalized totals, next processing position, and errors.
- `source_files`: run, sequence, relative path, size, original modification/birth timestamps, digest dependency, and processing error; unique `(run, sequence)`.
- `contents`: SHA-256 primary key, size, representative source, intended relative destination path, staging/temporary paths, copy state, and durable write sequence; unique intended path.
- `verification_passes`: run, pass state, and recorded verification bytes.
- `verification_results`: verification pass, digest, and result/error; associate required digests through each run's source dependencies.
- `run_reports`: run primary key referencing `runs(id)`, UTC generation timestamp, and the complete rendered report as text. Keep one latest snapshot per run, replacing it on a later import/resume/verify invocation for that run.

Enable foreign keys, WAL, and synchronous FULL on every writer connection. SQLite documents commit synchronization in [WAL mode](https://www.sqlite.org/wal.html). Use short transactions for inventory batches, digest claims, and completion; no transaction spans file I/O. Inventory is finalized with totals in a transaction before workers start. On interrupted inventory, discard only that run's unfinalized inventory and scan again, since no content work began. Inspection commands use a read-only connection and a read transaction for a consistent catalog snapshot.

Take an exclusive process lock on the catalog for the entire import/resume/verify command. Associate a catalog with one destination; reject mismatches. With the documented single-tool destination assumption, this avoids competing digest/path claims. Reject concurrent writer commands rather than adding multi-process scheduling. Read-only `runs` and `report` bypass the writer lock and can inspect the catalog while an import is active; they do not need source files, destination files, or ExifTool.

Content transitions are pending → published but not committed → complete. Publication is an on-disk observation, not evidence of completion. Source completion is derived from its content dependency, avoiding duplicate rows becoming done prematurely. Persist errors for retry without removing the dependency. Completed content is trusted until optional verification is requested.

### 3. Byte reservations and disk gates prevent unbounded or conflicting work

Reserve a file's full recorded size before staging, stream through a large reusable buffer while hashing, and persist its digest and staging reference before enqueueing a unique copy. Bound job counts as well as bytes so empty files cannot create an unbounded queue. Release duplicate reservations immediately; release unique reservations only after durable catalog completion. Staging is a tool-owned directory; account for recovered files before allowing new reservations. Staging need not be flushed for durability because resume validates it and can reread the unchanged source.

An oversized file is hashed without staging. The unique fallback copy acquires the same source disk gate as normal staging, so the source worker cannot read ahead concurrently. Metadata source access also takes that gate. The destination worker keeps exclusive destination access through copy and publication. Recover pending work before new processing; destination errors stop producing new jobs, unblock waiting reservations, and preserve unfinished work.

Resolve physical devices where the platform exposes topology; share gates for roots on the same physical disk, using deterministic gate acquisition order. When topology is unknown or ambiguous, serialize content I/O conservatively. Overlap is enabled for confirmed independent source/destination disks; SSD staging access does not introduce extra source reads. Verification runs only after the copy workers drain.

### 4. Broad media metadata through read-only ExifTool

Use ExifTool as a required external runtime dependency, invoked with argument arrays, read-only JSON output, selected capture/date tags, and explicit exclusion of FileModifyDate/FileCreateDate from metadata priority. Check availability before a new import so installation failures are explicit. This avoids implementing multiple image/video parsers; native EXIF alone would not cover the required video and HEIC formats. Missing or malformed per-file metadata falls back and is reported as diagnostic information.

Prefer EXIF DateTimeOriginal, XMP DateCreated, QuickTime CreationDate, then embedded CreateDate. Consult ExifTool's [tag reference](https://www.exiftool.org/TagNames.pdf) for supported groups. Preserve the calendar date written in a timestamp, including an explicit offset; do not shift dates using the host timezone. For timezone-less container dates, use their literal calendar date. Parse leftmost valid filename dates in YYYY-MM-DD, YYYY_MM_DD, and YYYYMMDD forms with numeric boundaries. Fall back to original source birth time then modification time, interpreted in UTC; no usable value means Unknown. Reject invalid calendar dates and zero metadata placeholders. Persist date and provenance with the intended path so resume does not change naming.

Classify extensions using the exact case-insensitive lists in media-organization. Retain the representative basename. The first recorded source claiming a new digest is its representative; already cataloged digests retain their existing representative. Reserve the basename if free, otherwise insert the full lowercase SHA-256 before the extension. If even that path conflicts with another reservation or existing unrelated content, report a conflict. Path claims occur in the coordinator before worker dispatch, so timing cannot decide names.

### 5. No-replace publication followed by catalog commitment

Persist pending digest, source, stage reference, and final path before destination I/O. Create an exclusive temporary file in the final parent directory, copy with bounded buffers, and sync the finished file. Use a platform publication adapter that atomically creates the final name without replacing an existing file. Prefer no-replace rename where supported; a same-filesystem hard-link publication followed by temporary unlink is an alternative where supported. Never use ordinary replacing rename as a fallback.

Create directories one at a time, recording affected parents. Flush each new directory and its parent and flush the temporary/final parent after publication and cleanup. Only then send a publication-success event for a FULL-synchronous catalog commit. Cleanup staging after that commit. Failure at any operation leaves pending state. Filesystems lacking the required no-replace or directory-flush operations fail explicitly. Implement and test platform adapters separately; initial executable validation is on the available Linux host, without claiming untested platforms.

On resume, first check final content for pending records: hash and size must match, then perform required directory flushes and commit adoption. Different final content is a conflict. Otherwise validate recorded staging and temporary files against size/digest. Reuse a complete temporary file through the same publication protocol; retry a partial temporary copy from valid staging or the unchanged representative source. Clean only tool-owned orphan files, after proving they have no pending catalog references. A completed row with leftover staging requires cleanup, not another destination write.

### 6. Progress and verification are separate observations

Maintain byte counters for source passes (including recovery and oversized rereads), destination transfer, and verification. Read durable done counts from committed dependencies. Render periodic line output for redirected streams and a refreshed view on a terminal. Display inventory totals only after inventory finalization and distinguish known unique byte totals from estimates.

For ETA, use smoothed phase throughput; estimate unseen unique bytes from the observed unique fraction. For independent disks use the larger source/destination remaining time, add source fallback reads to source demand, and include queued destination work. For serialized disks use summed demands. Show Estimating until nonzero timing/byte samples support the relevant rates. Rates are session observations; durable completion survives restart. Verification enumerates distinct run dependencies in recorded durable write order and starts a fresh pass after interruption. Persist each result; mark verified only at successful pass completion.

### 7. Persisted quick-check reviews

After an import, resume, or verification operation returns, generate a review from recorded run/source/content/verification data, add the last invocation outcome and runtime counters, upsert the text with a UTC timestamp into `run_reports`, and then print that same text to stdout by default. Handled failures and interruptions also attempt to save and print a report, preserving the original error and exit status. Use the pipeline's recorded shutdown reason to distinguish failure from interruption; the worker-cancellation flag alone is insufficient. A report-save failure after otherwise successful work makes the command fail explicitly; after failed work it is reported without replacing the original error. Failures before a run is created/opened and abrupt process termination cannot produce a final saved review.

Include roots/budget/phase, inventory completion and file/byte/skipped totals, oversized-file counts, hashed and distinct content counts/bytes, duplicate groups/extra copies/redundant bytes within the source, and distinct content with or without references from lower run IDs. Include completed source dependencies, complete/pending destination content, unhashed source files, current run/source/content errors, extension counts/bytes, date-provenance counts, and latest verification pass state/counts/bytes/errors. The check result reports catalog completion separately from verification at pass time. Elapsed seconds and source/destination/recovery/verification byte counters plus staging peak describe the last invocation only, not cumulative work across resumes.

Duplicate groups use recorded SHA-256 dependencies. Within-source extra copies equal the sum of `group_count - 1`; redundant bytes equal that number multiplied by content size. Earlier-run references count distinct digests with source dependencies in lower run IDs and can overlap within-source duplicates. They do not establish historical destination presence or newly copied bytes. Unhashed files are excluded from duplicate statistics. Incomplete inventories show partial counts and unavailable finalized skipped totals. Show at most ten duplicate groups, ordered by redundant bytes descending with digest tie-breaking, and up to ten source paths in inventory order per group, with full digest and destination mapping; explicitly count omitted groups/paths. Include all current stored file errors and latest-pass verification failures. Escape control characters in displayed paths/messages and display non-UTF-8 paths lossily without changing native database path bytes.

`runs` lists run IDs descending, phase, finalized file/byte totals, source path, and report timestamp, with placeholders for unavailable totals/reports and an explicit empty-catalog message. `report` prints the saved snapshot verbatim. When no report exists, including legacy or abruptly terminated runs, render a labeled current catalog snapshot without saving it or claiming unavailable invocation metrics. Both commands reject missing/unsupported catalogs and unknown report run IDs without creating a database. Neither command rechecks filesystem content. A saved review remains unchanged when another run executes, and can be stale while its own run is active until the next report save.

### 8. Tests target contracts and crash windows

Use unit tests for dates, naming, classification, budget accounting, and ETA. Use temporary real directory trees and SQLite integration tests for nested inventory, empty files, duplicates within/across runs, collisions, staging backpressure, oversized source-read counts, and unchanged source trees. Inject disk-access instrumentation to prove worker overlap and serialization independently of timing-sensitive HDD benchmarks.

Run the CLI as a child process with deterministic test-only failpoints after pending commit, partial copy, file sync, publication, directory sync, catalog completion, and before staging cleanup. Terminate without destructors, reopen the same catalog, and assert digest correctness, no overwrite, correct duplicate completion, and cleanup. Inject operation failures for read/write/sync/catalog errors; add corrupted staging/temp and final-conflict recovery cases. Tests demonstrate process-crash recovery and ordering, not actual power-loss guarantees. Maintain tests/README.md with implemented coverage, fixture provenance, exact commands, executed results, and explicit limitations.

Report integration tests in `tests/report.rs` verify actual CLI output against persisted text, same-source/cross-run duplicate arithmetic, stable prior-run snapshots, inspection while the writer lock is held, verification-failure refresh and subsequent resume, unknown/missing/empty catalogs, read-only legacy inspection, preserved rows through migration, and honest partial-inventory counts. `tests/outcomes.rs` additionally verifies saved reviews on failed copies and handled interruptions with exit 1/130. Crash tests parse the completion-summary line explicitly because the report follows it.

## Risks / Trade-offs

- [Drive firmware or filesystems ignore durability operations] → Explain storage assumptions and distinguish operation-order tests from power-loss testing.
- [ExifTool installation and process overhead] → Preflight dependency, query only needed tags, extract only unique content, and document installation. Keep the adapter replaceable without changing date priority.
- [Physical disk identity unavailable] → Conservatively serialize; do not sacrifice the single-reader guarantee for throughput.
- [FULL commits are expensive] → Batch inventory and completion events only when filesystem finalization has already succeeded; release no dependent staging before the batch commit.
- [Unknown unique fraction causes noisy ETA] → Label estimates approximate, smooth observations, and show Estimating until meaningful samples exist.
- [Filesystems reject source basenames] → Report actionable path errors, preserve source content, and leave the copy resumable; avoid silently changing filename rules.

## Migration Plan

Create schema version 2 for new catalogs. For version-1 catalogs opened by import/resume/verify, add `run_reports` and set `PRAGMA user_version=2` in one transaction, preserving existing records. Read-only `runs`/`report` accept versions 1 and 2 without migration. Reject unsupported versions explicitly. Build with Cargo, install ExifTool, and document a small-directory import/resume/verify trial plus run-list/report commands. A version-1-only binary cannot open a migrated version-2 catalog; retain a compatible binary or a pre-migration backup for rollback, and never automatically delete imported content or reinterpret an incompatible catalog.
