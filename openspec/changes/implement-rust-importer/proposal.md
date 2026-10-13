# Proposal

## Why

Unduplicator currently describes an import workflow but has no implementation. Implement it in Rust so removable-drive collections can be consolidated without modifying sources, repeating unnecessary HDD reads, or losing resumable progress after interruption.

## What Changes

- Add a Rust CLI with inventory, resumable import, and optional destination verification backed by a local SQLite catalog.
- Stream source content through SHA-256 and bounded SSD staging; overlap one source reader with one destination writer and resolve duplicates across pending and completed imports.
- Organize media by date with deterministic naming and collision handling.
- Publish destination files without overwrite and record completion only after durable finalization; reconcile interrupted work on resume.
- Report durable file completion, byte progress, and phase-aware time estimates.
- Add unit, real-filesystem/SQLite integration, and controlled crash-recovery tests. Maintain `tests/README.md` describing actual tests, commands, fixtures, coverage, and limitations; distinguish executed results from planned coverage.

## Capabilities

### New Capabilities

- `file-import`: Source inventory, bounded staging, content deduplication, HDD scheduling, and copy-only import.
- `media-organization`: Media classification, date selection, and deterministic destination paths.
- `import-recovery`: Durable publication, catalog state transitions, interruption recovery, and destination verification.
- `import-progress`: CLI invocation, resumable run selection, progress reporting, and outcome reporting.

### Modified Capabilities

None; the project has no existing capability specs.

## Impact

Introduce a Cargo project, application modules, SQLite schema, automated tests, and test documentation. Dependencies will cover CLI parsing, SHA-256, SQLite, date parsing, and media metadata extraction. Filesystem durability support must be checked for the execution platform; unsupported guarantees must produce explicit errors. No existing application API or data migration is required. Source mutation, automatic catalog reconstruction, and detection of external source/destination changes remain outside scope as documented in the README.
