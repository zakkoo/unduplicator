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
