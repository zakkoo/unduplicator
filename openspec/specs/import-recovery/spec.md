# import-recovery Specification

## Purpose

Preserve resumable import progress through interruption and distinguish durable copies from independently verified destination content.

## Requirements

### Requirement: Durable copy completion
The tool SHALL persist pending digests, source references, and intended paths before copying. It SHALL flush destination temporary content, publish without replacement, and flush affected directory entries including newly created parents before durably committing completion in SQLite. Staging SHALL be released only after that commit.

#### Scenario: Publication succeeds
- **WHEN** a unique file finishes copying
- **THEN** its content and directory entries are flushed before catalog completion and staging release
- **AND** dependent duplicate files become complete only after catalog completion

### Requirement: Preserve destination conflicts
The tool SHALL never replace an existing final file. A conflicting file SHALL remain intact, and the affected copy and dependent duplicates SHALL remain unfinished with an actionable error.

#### Scenario: Conflicting final content
- **WHEN** a pending copy's intended final path contains a different digest
- **THEN** that file remains unchanged and the pending copy is reported as unfinished

### Requirement: Reconcile interrupted publication
On resume, the tool SHALL reconcile pending records against final, temporary, and staged files. A matching final digest SHALL be adopted only after required directory flushes and a durable catalog commit. Recovered temporary or staged content SHALL be validated before reuse. Missing or invalid work SHALL be retried from valid staging or the unchanged source.

#### Scenario: Crash after publication
- **WHEN** a final file exists but its catalog copy remains pending
- **THEN** resume checks its full digest before recording durable completion without writing another copy

#### Scenario: Invalid staging or partial temporary content
- **WHEN** recovered work does not match its expected digest and size
- **THEN** the tool does not publish that work and retries from valid content

#### Scenario: Crash after completion before cleanup
- **WHEN** completed catalog content still has a staging file
- **THEN** resume releases the redundant staging file without repeating the destination copy

### Requirement: Resume durable inventory
Resume SHALL use the existing recorded inventory and intended paths. Interrupted inventory SHALL not be treated as complete; processing SHALL start only after inventory totals and order are finalized. A run SHALL be complete only when every inventoried source file has a durably completed destination dependency.

#### Scenario: Incomplete shared copy
- **WHEN** one unique copy fails and multiple inventoried files depend on it
- **THEN** those files remain unfinished and the run cannot be reported complete

### Requirement: Separate destination verification
Optional verification SHALL start after copying finishes, reread required destination content in recorded write order, and compare full SHA-256 digests. It SHALL have separate progress and persist outcomes. A run SHALL be marked verified only when every required digest matches; read failures or mismatches SHALL produce unsuccessful verification.

#### Scenario: Completed import with mismatched destination
- **WHEN** verification encounters a digest mismatch
- **THEN** the mismatch is recorded and reported and the run is not marked verified

#### Scenario: Interrupted verification
- **WHEN** verification is interrupted and requested again
- **THEN** the tool performs a new verification pass over every required digest before marking the run verified

### Requirement: Explicit durability failures
Unsupported publication or flush guarantees and failed durability operations SHALL produce explicit errors. The tool SHALL not mark affected content complete by silently weakening those guarantees.

#### Scenario: Directory flush fails
- **WHEN** the destination cannot flush an affected directory
- **THEN** the affected copy remains unfinished and the tool reports the failed operation
