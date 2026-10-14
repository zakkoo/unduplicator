# Tasks

## 1. Rust project and invocation

- [x] 1.1 Create the Cargo binary/library structure and pinned dependencies from design.md; verify `cargo check`, `cargo fmt --check`, and CLI help succeed.
- [x] 1.2 Implement import/resume/verify arguments, positive budget parsing, root validation, and ExifTool preflight; add CLI tests for valid invocations, missing dependency, overlapping roots, and invalid arguments with expected exit statuses.
- [x] 1.3 Document build/runtime prerequisites and proposed invocation examples in README.md; create tests/README.md separating available tests from planned coverage and verify the documented help/build commands.

## 2. Catalog and inventory

- [x] 2.1 Implement versioned SQLite tables, indexes, foreign keys, WAL/FULL setup, exclusive command lock, and catalog/destination association; verify real-SQLite tests reject unsupported versions, concurrent commands, and destination changes.
- [x] 2.2 Implement metadata-only inventory with native path storage, original timestamps, directory-local traversal order, batched rows, finalized totals, and interrupted-inventory restart; verify nested trees, empty inventory, non-UTF-8 Unix paths, inaccessible paths, and symlink/special-file policy without source content reads.
- [x] 2.3 Implement pending content claims, unique destination reservations, source dependencies, errors, durable completion transactions, and run selection; verify pending/completed duplicates and unfinished shared copies have correct derived source completion.
- [x] 2.4 Add inventory/catalog tests and commands to tests/README.md and document root, symlink, and resume policies in README.md; verify descriptions match implemented behavior and executed results.

## 3. Media and deterministic paths

- [x] 3.1 Implement the exact extension lists, calendar validation, filename patterns, timestamp fallback, and Unknown layout; verify table-driven tests cover every extension, invalid/leap dates, offset dates, source timestamp priority, and exclusion of staging timestamps.
- [x] 3.2 Implement read-only ExifTool JSON extraction and date-tag priority from design.md for staged and oversized media; verify real image/video metadata fixtures plus malformed/absent metadata and missing-process tests, recording fixture provenance.
- [x] 3.3 Implement inventory-ordered representative/path decisions, digest suffixes, and persisted date provenance; verify same-name different-content files survive and randomized completion order/resume produces identical intended paths.
- [x] 3.4 Document date/timezone rules, metadata dependency, naming, and media fixtures in README.md and tests/README.md; verify examples against tests and distinguish formats tested with real fixtures from extension classification alone.

## 4. Durable destination publication

- [x] 4.1 Implement pending-record-before-copy ordering, exclusive destination temporary files, buffered transfers, and platform no-replace publication; verify real-filesystem tests preserve preexisting content and injected copy failures leave pending work.
- [x] 4.2 Implement file sync, newly created parent/directory sync, durable catalog completion, and staging release ordering; verify instrumented operation-order tests and injected sync/commit failures never count affected dependencies complete.
- [x] 4.3 Implement explicit unsupported-platform/filesystem errors; verify supported host publication with real files and unsupported-operation tests without weakening completion guarantees.
- [x] 4.4 Document filesystem guarantees, support actually validated, and publication tests in README.md and tests/README.md; verify process-crash/ordering coverage is not described as proof of power-loss durability.

## 5. Bounded pipeline and deduplication

- [x] 5.1 Implement byte reservations, bounded job/event channels, buffered single-pass SHA-256 staging, and coordinator dispatch; verify full-budget backpressure, release after commit, zero-byte queue bounds, and one full source content pass for normal files.
- [x] 5.2 Wire pending/completed digest resolution into the pipeline; verify empty files, same-run pending duplicates, later-import duplicates, unique destination write counts, and unchanged source trees through real-filesystem/SQLite tests.
- [x] 5.3 Implement physical-disk gates and independent-disk overlap with conservative serialization for unknown/shared topology; verify instrumented concurrency tests show at most one reader/writer per disk and overlap on independent devices without relying on elapsed-time races.
- [x] 5.4 Implement oversized hash-then-copy fallback and source metadata access under the source gate; verify oversized duplicates avoid a copy reread, unique files include the extra source bytes, and staging remains within budget.
- [x] 5.5 Implement coordinated error/interruption shutdown that unblocks waiting workers, preserves pending records, and emits resume information; verify full-queue destination failure and interruption do not deadlock or report success.
- [x] 5.6 Document staging ownership, disk scheduling, fallback reads, and pipeline test commands/results in README.md and tests/README.md; verify descriptions against counters and concurrency tests.

## 6. Recovery

- [x] 6.1 Implement pending-record reconciliation with digest/size validation of final, staged, and temporary content, including parent sync before final adoption; verify matching finals are adopted without another copy and conflicting finals remain intact and unfinished.
- [x] 6.2 Implement retry from valid staging or unchanged source, orphan cleanup limited to tool-owned files, resumed budget accounting, and completed-stage cleanup; verify partial/corrupt/missing files, recovered budget saturation, and committed copies with leftover stages.
- [x] 6.3 Add child-process test-only crash failpoints after pending commit, partial transfer, file sync, publication, directory sync, catalog commit, and before stage cleanup; verify each restarted run produces correct digests, deterministic paths, no overwrite, and accurate duplicate completion.
- [x] 6.4 Document resume/retry commands, recovery fixture setup, executed crash tests, and their limitations in README.md and tests/README.md; verify a documented interrupted-run recovery example.

## 7. Progress and optional verification

- [x] 7.1 Implement distinct source, destination, and verification byte counters plus committed dependency completion counts and terminal/redirected rendering; verify snapshots and integration tests cover pending duplicates, oversized rereads, and a draining destination queue.
- [x] 7.2 Implement smoothed byte-based phase estimates, unique-fraction refinement, overlap/serialization accounting, and Estimating startup state; verify synthetic-clock tests for unequal file sizes, source-finished destination backlog, zero-byte collections, and verification estimates.
- [x] 7.3 Implement verification after copy drain in durable write order, persisted pass/results, and verified state only on full success; verify matching content, mismatches, missing/read-failing files, shared dependencies, and a fresh full pass after interruption.
- [x] 7.4 Wire complete/unfinished/verification-failed summaries, run IDs, and exit statuses into all commands; verify subprocess tests distinguish unverified successful import, successful verification, failed copy, verification mismatch, invalid invocation, and interruption.
- [x] 7.5 Document progress fields, approximate ETA, verification, and exit statuses in README.md and tests/README.md; verify the documented command sequence and record actual results.

## 8. Integrated acceptance

- [x] 8.1 Run an end-to-end multi-source collection import, interrupted resume, cross-import deduplication, and optional verification using real directories/catalogs; verify unique hashes, deterministic layouts, bounded staging, accurate outcomes, and unchanged sources together.
- [x] 8.2 Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`; resolve failures and update tests/README.md with exact executed commands/results and remaining platform, media-fixture, and power-loss limitations.
- [x] 8.3 Review every scenario in all four specs against implemented tests or explicit validation evidence and verify no required behavior is deferred or silently narrowed before marking this change complete.
