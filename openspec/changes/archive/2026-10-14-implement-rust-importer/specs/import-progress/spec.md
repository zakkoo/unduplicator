# Spec Delta

## Purpose

Provide explicit CLI controls and phase-aware progress so users can start, resume, and verify imports and understand unfinished work.

## ADDED Requirements

### Requirement: Explicit import and resume controls
The CLI SHALL accept source, destination, local catalog, staging location, and staging byte budget for a new import. It SHALL identify runs and allow explicit resume and optional verification. Resume SHALL preserve recorded source, destination, catalog association, and import configuration; incompatible arguments SHALL fail before processing.

#### Scenario: Start and resume a run
- **WHEN** a user starts an import and later resumes its run identifier with the same catalog
- **THEN** the CLI continues recorded work without creating a second inventory

#### Scenario: Invalid configuration
- **WHEN** the budget is zero or resume arguments conflict with recorded configuration
- **THEN** the CLI reports the invalid configuration and exits unsuccessfully before processing content

### Requirement: Durable file progress
Import progress SHALL show durably completed source files against finalized inventory totals, source bytes read, and destination bytes copied against currently known unique bytes. Duplicates of pending content SHALL count as done only after their shared copy completes. Progress SHALL remain available when output is redirected.

#### Scenario: Pending duplicate dependency
- **WHEN** two source files share a pending destination copy
- **THEN** neither contributes to the done count until that copy is durably complete

### Requirement: Phase-aware time estimates
The tool SHALL estimate remaining time using observed byte throughput, remaining source reads, known and estimated unique destination writes, disk overlap, and oversized rereads. It SHALL show Estimating until adequate observations exist, refine estimates as duplicates are discovered, and include the destination queue after source reading ends. Verification SHALL have an independent estimate.

#### Scenario: Source reading finishes first
- **WHEN** all source files are processed but destination copies remain queued
- **THEN** the remaining estimate reflects queued destination work and the run remains in progress

#### Scenario: Initial observations are insufficient
- **WHEN** progress has no reliable throughput sample
- **THEN** the time estimate displays Estimating instead of a fabricated completion time

### Requirement: Accurate outcome reporting
The CLI SHALL report complete, unfinished, or verification-failed outcomes with relevant file errors and a resume identifier. It SHALL return success only for the requested successfully completed operation; requesting verification SHALL require successful verification for a success exit status. Interruptions SHALL leave resumable state and return an unsuccessful status.

#### Scenario: Import complete without verification
- **WHEN** every inventoried file has a durable destination dependency and verification was not requested
- **THEN** the CLI reports complete without claiming verified and exits successfully

#### Scenario: Failure or interruption
- **WHEN** a source read, destination copy, verification, or interruption prevents the requested operation completing
- **THEN** the CLI exits unsuccessfully and identifies unfinished work and the run to resume

### Requirement: Persist and print run reviews
After import/resume/verify returns on an opened or created run, the tool SHALL attempt to save a text review and UTC timestamp in `run_reports` and print the saved text by default. It SHALL keep one latest report per run, replacing it only on subsequent operations on that run. Handled failures/interruptions SHALL retain their original error and exit status; a report-save failure after successful work SHALL make the command fail explicitly.

#### Scenario: No final reporting opportunity
- **WHEN** an operation fails before its run can be opened/created or the process terminates abruptly
- **THEN** a final persisted report is not guaranteed and a prior saved review, if any, remains a snapshot of its earlier save

#### Scenario: Successful operation produces a review
- **WHEN** import, resume, or verification finishes successfully
- **THEN** the latest report is saved with a UTC timestamp and the same text is printed after the operation's progress and completion summary

#### Scenario: Resume or verification refreshes a report
- **WHEN** another operation on the same run finishes
- **THEN** that run's prior report is replaced by the new review
- **AND** saved reviews for other runs remain unchanged

#### Scenario: Handled failure or interruption
- **WHEN** an operation on an opened run returns a handled work failure or interruption and the catalog can persist its review
- **THEN** the tool saves and prints a failed or interrupted review respectively and retains exit status 1 or 130

#### Scenario: Report persistence fails
- **WHEN** saving a review fails after an operation returns
- **THEN** the tool reports the persistence error and does not claim a saved review
- **AND** otherwise successful work returns unsuccessfully, while an existing work failure retains its original error and exit status

### Requirement: Quick-check report contents
Reports SHALL summarize run configuration, inventory, hashed/distinct content, duplicates, completion, errors, extension/date provenance and latest verification. Persisted reviews SHALL include last-invocation outcome, elapsed seconds, source/destination/recovery/verification byte counters and staging peak, labeled as invocation values rather than cumulative values across resumes.

#### Scenario: Generate a comprehensive review
- **WHEN** a review is generated for a recorded run
- **THEN** it includes run ID, phase, source/destination/staging roots and byte budget, inventory completion, recorded file/byte totals, finalized skipped symlink/special-file counts and oversized-file count
- **AND** it includes hashed source files, distinct content counts/bytes, within-source duplicate groups/extra copies/redundant bytes and distinct content with/without earlier-run references
- **AND** it includes completed source dependencies, complete/pending destination content, unhashed files, current run/source/content errors, extension counts/bytes and date-provenance counts
- **AND** its latest verification section includes pass ID/state, checked/passed/failed counts, required content without results, recorded bytes and failure details

#### Scenario: Duplicates share a destination dependency
- **WHEN** several hashed source files share a digest
- **THEN** the report counts one distinct content group, `count - 1` extra within-source copies and `(count - 1) * size` redundant source bytes
- **AND** source-file completion counts reflect their shared durable destination dependency

#### Scenario: Latest verification contains failures
- **WHEN** the latest verification pass contains a read failure or digest/size mismatch
- **THEN** the report includes the failure counts and recorded error details without claiming that the latest verification passed

#### Scenario: No verification pass exists
- **WHEN** a run has no recorded verification pass
- **THEN** the review explicitly reports that destination bytes have not been independently checked

### Requirement: Honest report interpretation and bounded duplicate details
Duplicate statistics SHALL cover hashed files only and label cross-run matches as potentially overlapping catalog references. Reports SHALL label incomplete inventory as partial and distinguish catalog completion from verification at pass time. Duplicate details SHALL be limited to ten groups and ten paths per group, with omission counts. Current stored errors SHALL be included fully. Displayed paths/messages SHALL escape control characters.

#### Scenario: Select and display duplicate details
- **WHEN** duplicate groups are rendered
- **THEN** groups are ordered by redundant bytes descending with digest tie-breaking, and source paths within a group follow inventory order
- **AND** each displayed group includes full SHA-256 digest, content size and destination mapping
- **AND** non-UTF-8 paths may be displayed lossily while native path bytes remain unchanged in the database

#### Scenario: Interpret a catalog snapshot
- **WHEN** a report shows earlier-run matches or completed/verified content
- **THEN** earlier matches count distinct digests referenced by source rows in lower run IDs rather than arbitrary destination files or an exact historical copy count
- **AND** completion and verification claims describe recorded state and verification at pass time, without claiming a fresh filesystem check
- **AND** current stored source/content errors and latest-pass verification failures are not truncated by duplicate-detail limits

#### Scenario: Same content appears in earlier runs and twice in this source
- **WHEN** a digest has references in a lower run ID and multiple references in the selected run
- **THEN** it contributes once to the distinct earlier-run match count and contributes its extra copies to the within-source duplicate count
- **AND** the report explains the overlap rather than adding those counts as mutually exclusive categories

#### Scenario: Inventory or hashing is unfinished
- **WHEN** inventory is incomplete or source files lack digests
- **THEN** partial inventory and unhashed counts are explicit and the report does not present duplicate statistics as covering unprocessed files

#### Scenario: Duplicate details exceed the display limit
- **WHEN** more than ten duplicate groups exist or a displayed group has more than ten source paths
- **THEN** the report limits the detailed display and states the number of omitted groups or paths while retaining complete aggregate counts

### Requirement: Read-only run listing and report viewing
The CLI SHALL provide `runs --catalog PATH` and `report --catalog PATH --run ID` with a positive report run ID. Inspection SHALL use a consistent read-only snapshot without acquiring the importer lock, accessing source/destination files, requiring ExifTool, migrating schemas or creating databases. Viewing SHALL print saved text verbatim, or a labeled current snapshot without persistence when no report exists. Missing/unsupported catalogs and unknown run IDs SHALL fail explicitly.

#### Scenario: List catalog runs
- **WHEN** a user invokes `runs --catalog PATH`
- **THEN** runs are listed in descending ID order with phase, finalized inventory file/byte totals, source path and saved-report UTC timestamp
- **AND** unavailable totals/reports have explicit placeholders and an empty catalog produces an explicit no-runs message

#### Scenario: View a run without recorded invocation metrics
- **WHEN** report viewing generates a current snapshot because no saved report exists
- **THEN** it does not invent elapsed time, invocation outcome or runtime byte/peak counters that are unavailable from the catalog

#### Scenario: Unsupported catalog version
- **WHEN** a listing or report command opens a catalog with an unsupported schema version
- **THEN** it reports an error and exits unsuccessfully without modifying the catalog

#### Scenario: Inspect a saved run while a writer holds the lock
- **WHEN** a user lists runs or views a saved report while import/resume/verify holds the catalog lock
- **THEN** inspection succeeds through a read-only snapshot without acquiring the writer lock
- **AND** viewing returns the latest saved report rather than rerunning import or verification

#### Scenario: An older or abruptly terminated run has no saved report
- **WHEN** the requested run exists but has no saved review
- **THEN** the CLI renders a labeled current catalog snapshot without saving a report or modifying the schema

#### Scenario: Missing catalog or unknown run
- **WHEN** inspection names a missing catalog or report viewing names an unknown positive run ID
- **THEN** the CLI exits unsuccessfully without creating a database or a run

### Requirement: Compatible report schema migration
New catalogs SHALL use schema version 2 with `run_reports(run INTEGER PRIMARY KEY REFERENCES runs(id), generated_at TEXT NOT NULL, report TEXT NOT NULL)`. Import/resume/verify SHALL add this table and advance version-1 catalogs to version 2 transactionally without losing existing records. Read-only inspection SHALL support both versions without migration and reject unsupported versions. Version-1-only binaries cannot be assumed to open migrated catalogs.

#### Scenario: Writer opens a version-1 catalog
- **WHEN** import, resume, or verification opens an existing version-1 catalog
- **THEN** it adds report storage and advances the version to 2 atomically while preserving existing run, source, content and verification records

#### Scenario: Inspect a version-1 catalog
- **WHEN** listing or viewing opens a version-1 catalog
- **THEN** inspection succeeds without adding tables or changing its version, and reports without saved text use the current-snapshot fallback
