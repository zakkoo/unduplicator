# file-import Specification

## Purpose

Import unique file content from removable storage while preserving sources and minimizing repeated HDD reads.

## Requirements

### Requirement: Inventory before content processing
The tool SHALL persist source paths, sizes, extensions, original timestamps, traversal order, total file count, and total bytes before importing content. Inventory SHALL avoid content reads and retain directory locality without a separate size-grouping pass.

#### Scenario: Inventory a nested collection
- **WHEN** a source tree is inventoried
- **THEN** persisted totals match the discovered files and the processing queue retains the recorded traversal order with files from the same directory grouped together
- **AND** no source file content is read during inventory

### Requirement: Copy-only source access
The tool SHALL never write, move, or delete source files. Failures SHALL be reported and SHALL prevent a falsely successful run.

#### Scenario: Import succeeds or fails
- **WHEN** an import finishes or encounters a read or copy error
- **THEN** source paths and content remain unchanged and any failed files are reported as unfinished

### Requirement: Full-content deduplication
The tool SHALL calculate SHA-256 over every source file's full content and store one destination copy per digest across imports. Duplicate resolution SHALL include both completed and pending content; pending duplicates SHALL share the original copy dependency.

#### Scenario: Duplicates arrive before publication
- **WHEN** two differently named files have identical content and the first copy is pending
- **THEN** only one destination copy is queued and both files wait for its durable completion

#### Scenario: Duplicate arrives in a later import
- **WHEN** a file matches a completed catalog digest
- **THEN** its full source digest is calculated and it is linked to the existing destination copy without another destination write

#### Scenario: Empty files
- **WHEN** multiple zero-byte files are imported
- **THEN** they share the SHA-256 digest of empty content and one destination copy

### Requirement: Bounded staged processing
The tool SHALL normally read each source file once, hashing while staging to SSD. Staging SHALL stay within a configurable byte budget and release redundant or durably completed content. When space is unavailable, source processing SHALL wait while destination work can continue.

#### Scenario: Staging budget is exhausted
- **WHEN** pending staged files occupy the available budget
- **THEN** further staging waits without exceeding the budget and resumes after completed copies release space

### Requirement: Oversized-file fallback
For a file larger than the staging budget, the tool SHALL hash on the source without fully staging it, reread it for copying only if unique, and obtain required metadata from the source. The fallback SHALL preserve the staging limit.

#### Scenario: Oversized duplicate
- **WHEN** an oversized file matches a known digest
- **THEN** no second source content pass for copying and no destination write occur

#### Scenario: Oversized unique file
- **WHEN** an oversized file has a new digest
- **THEN** it is reread for its destination copy and reported source byte counts include that additional pass

### Requirement: HDD-aware scheduling
The tool SHALL allow at most one active content reader per physical source HDD and one writer per physical destination HDD. It SHALL overlap work across separate disks with bounded queues, preserve recorded source order, and avoid running verification concurrently with copying.

#### Scenario: Independent disks have queued work
- **WHEN** one staged file is ready to copy and the next source file fits the staging budget
- **THEN** source reading and destination copying can overlap without interleaving multiple content streams on either HDD

#### Scenario: Oversized fallback reads the source
- **WHEN** the destination worker needs to reread an oversized source file
- **THEN** other source content reads wait until that source access completes
