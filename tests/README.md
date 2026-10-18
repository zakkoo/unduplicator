# Tests

## Available and executed

On Linux with Rust 1.98.1 and ExifTool 13.55:

```sh
cargo check --locked
cargo fmt --check
cargo build --locked
cargo run --locked -- --help
cargo run --locked -- import --help
cargo test --test cli
```

CLI tests: 4 passed. They cover the package version reported by `--version`,
argument parsing, positive budgets/run IDs,
missing arguments, forbidden resume arguments, invalid invocation exit 2,
missing ExifTool exit 1, normalized overlapping roots and symlink aliases.
Validation failures create no catalog or destination.

In a restricted agent session, commands use CARGO_HOME=/tmp/unduplicator-cargo
and RUSTUP_HOME=/tmp/unduplicator-rustup so caches stay in writable storage.
These environment variables are unnecessary in a normal terminal.

## Validation limits

Executable filesystem validation is on this Linux host only. Other platforms
are rejected. Real media metadata fixtures cover JPEG, PNG, MP4, and MOV; other extensions
have classification coverage. Disk topology/concurrency tests use instrumented
identities, not two physical HDDs. Process crashes, injected failures and successful
flush calls do not establish power-loss durability or storage firmware behavior.

## Catalog and inventory: executed

```sh
cargo test --test catalog
cargo test --test inventory
```

Catalog: 2 passed, using real SQLite. Covers version rejection, exclusive lock,
destination association, WAL/FULL and foreign keys, pending/completed duplicate
dependencies, representative preservation, durable write order and unknown runs.
Inventory: 3 passed. Covers nested directory locality, finalized totals/reuse,
empty inventory, non-UTF-8 names, symlinks, FIFO skipping, unreadable file content
(metadata inventory still succeeds), unreadable directories (scan fails), and
discarding partial rows on restart. Unix tests run as an unprivileged user.

## Media and naming: executed

```sh
cargo test --lib media
cargo test --test media
cargo test --test naming
```

2 date/classification tests, 2 metadata tests and 2 naming tests passed. Cover
every listed extension, leap/invalid dates, numeric filename boundaries,
offset literal dates, original timestamp priority, Unknown and Other layouts,
metadata tag priority and invalid/malformed/absent metadata, missing process,
real JPEG/PNG/MP4/MOV extraction, extension-less staging, persisted provenance,
representatives, completion-order variation, reopened catalog path stability,
same basenames with different content, existing-name preservation and secondary
conflicts. Fixture provenance and regeneration commands: fixtures/README.md.
JPEG, PNG, MP4, and MOV are the real formats tested; others have classification coverage.

## Publication: executed

```sh
cargo test --test publication
```

3 tests passed on Linux. Real files exercise hard-link no-replace publication,
exclusive temporary files, directory flushes and preexisting final preservation.
Publication rejects a destination directory changed to a symlink after naming.
Instrumented ordering checks pending commit before copy, file sync before
publication, directory sync before catalog commit, and stage removal after commit.
Injected transfer, sync-boundary, commit-boundary and actual SQLite trigger failures keep source
dependencies pending and stages retained. /proc rejects directory fsync explicitly.
Other platforms are unvalidated and rejected. This is operation-order and
process-failure coverage, not a power-loss experiment.

## Pipeline: executed

```sh
cargo test --lib
cargo test --test pipeline
```

6 library tests and 7 pipeline tests passed. Real directories/SQLite cover
budget saturation, four-reservation limits including empty jobs, one full source
pass per normal file, same-run pending duplicates, empty digests, cross-import
duplicates without destination writes, exact oversized unique/duplicate read
counts, stage cleanup and unchanged source content. Gates are exercised with
instrumented identities: barriers prove independent reader/writer overlap;
active counters prove conservative shared/unknown serialization and source
fallback exclusion. Deterministic full-queue barriers exercise destination
failure and interruption shutdown. These tests do not benchmark physical HDDs.
An error retaining an owned stage cancels new admission before its reservation
is released; a regression test verifies retained physical bytes stay bounded.
Staging on a source/destination device is also tested for serialized access.
Temporary namespace regressions verify that source basenames resembling an
internal temporary cannot alias or lose final content, including during cleanup.

## Recovery and crash windows: executed

```sh
cargo test --test recovery
cargo test --features test-failpoints --test crash
```

Recovery tests and child-process tests cover twelve import crash windows:
after pending commit, new directory sync, partial transfer, file sync, publication,
directory sync, before and after catalog commit, before and after stage cleanup,
and before and after report commit. Inputs exceed one transfer buffer so
the partial-copy window is actually partial. Each child exits without stack
destructors, reopens the same SQLite catalog, resumes its run, and checks full
digests/sizes, persisted paths, duplicate done counts, existing destination
preservation, source contents and empty staging. Published finals are adopted
without another transfer. Real-directory recovery tests additionally cover
complete/partial temporaries, corrupt/missing stages, final conflicts, committed
stages, orphan ownership and recovered budget saturation before new staging.

The design notes recovery invocation is exercised by this suite with temporary paths
and the actual printed run ID. Crash/error controls exist only in builds with
the `test-failpoints` feature. Production builds ignore those environment variables.
Process crashes and successful fsync calls are not evidence of power-loss survival.

## Progress, verification and outcomes: executed

```sh
cargo test --lib progress
cargo test --test pipeline
cargo test --test verification
cargo test --features test-failpoints --test outcomes
```

2 synthetic-clock ETA tests, 7 pipeline tests with progress snapshots, 1
verification integration test and 4 outcome subprocess tests passed. Covers
pending duplicate snapshots and draining destination demand, distinct counters
and oversized rereads, byte-weighted estimates independent of file count,
unique-fraction refinement, overlap vs serialization, startup/zero-byte Estimating,
smoothed verification estimates, durable verification order and pass results,
shared dependencies, fresh full verification after interruption, mismatch and
missing/permission-denied destinations. Outcome subprocesses verify unverified
import success, requested verification success, failed copy exit 1, mismatch
exit 1, invalid invocation exit 2, interruption exit 130 and verified resume.
They also cover a real source permission failure, concurrent-command rejection,
and a symlinked destination directory that would otherwise write into the source.
Recovery and verification reject symlinked destination directories as well.
Redirected output includes initial/final progress and summaries; terminals use
line refresh. ETA remains approximate and is not a hardware throughput benchmark.

## Integrated acceptance and final checks: executed

```sh
cargo test --features test-failpoints --test acceptance
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features test-failpoints -- -D warnings
cargo test
cargo test --features test-failpoints
openspec validate implement-rust-importer --strict --json
git diff --check
```

On 2026-10-18, the feature-enabled suite passed 47 tests with zero failures or
ignored tests. It includes twelve import crash windows, five verification/report
crash windows, and an interrupted multi-source acceptance run. A clean baseline yields
the same digest/path map as resume; a later source import writes only new content.
Acceptance checks real JPEG/PNG/MP4/MOV layouts, same-name collision suffixes, native Unix
names, oversized files, unchanged source paths/content/mtime, staged byte peak,
full final digests, distinct dependencies and verified run states together.

The design notes small-directory workflow was also executed using an isolated
`/tmp/unduplicator-doc-trial.*` tree: import completed 1/1 with 8 source and
destination bytes; resume with verification copied 0 bytes and verified 8 bytes;
standalone verification reread 8 bytes. All three commands exited 0. The normal
binary ignored a test failpoint environment variable, confirming the controls
are excluded from production builds. Build/check and help commands also passed.
An actual PTY verification invocation confirmed terminal line refresh and a
successful verified summary; subprocess suites exercise redirected line output.

Every scenario is mapped to validation evidence in [spec-coverage.md](spec-coverage.md).

## Saved run reports

```sh
cargo test --locked --test report
cargo test --locked --features test-failpoints
cargo clippy --locked --all-targets --features test-failpoints -- -D warnings
```

Report integration tests use real CLI imports and SQLite to check within-source
and cross-run duplicate arithmetic, destination mappings, identical printed and
persisted text, stable prior-run snapshots, list/show commands while the writer
lock is held, failed verification and subsequent resume, unknown run IDs,
read-only legacy inspection, non-destructive version-1 to version-2 migration,
partial inventories, empty catalogs, and missing catalogs without file creation.
Verdict tests cover passing verification, unverified imports, skipped symlinks,
failed verification, unfinished inventory, and a corrupted inventory byte total.
The outcome failpoint tests also check stored reviews on failed copies and
handled interruptions, preserving exit statuses 1 and 130. Abrupt process-crash
tests continue to exercise recovery; a process that exits without cleanup cannot
save a final report, and report viewing always uses a current read-only snapshot
for work totals and verdict, whether or not saved text exists.

## Crash-safe catalog reports

Reports now always use a current catalog snapshot, including when saved text
exists. Completion totals derive from committed content states and sizes without
new accounting writes. The crash suite checks database integrity, live report
counts/bytes before and after recovery, and repeated resumes without double
counting. Five additional verification/report crash windows check atomic result
and byte updates, stale saved PASS handling, and report replacement rollback.
Imports exercise a newly created nested catalog directory.

Report tests cover zero-byte files, reused content, historical performance
statistics, legacy report text, missing cached text, and a failed report save
that preserves completed work. Filesystem flush ordering is tested separately;
these tests do not simulate a physical power cut or failing storage hardware.
