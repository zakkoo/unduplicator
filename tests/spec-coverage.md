# Spec scenario review

Reviewed against implement-rust-importer on 2026-10-14. Test names below are
implemented and executed; `lib` means unit tests within the indicated module.
No required behavior is deferred. Platform, fixture and durability validation
limits remain as documented in README.md; they are not evidence of power-loss
survival or of metadata parsing in every listed format.

## file-import

| Scenario | Validation evidence |
| --- | --- |
| Inventory a nested collection | inventory::nested_local_order_and_empty_inventory; inventory::native_paths_skips_and_metadata_only_reads |
| Import succeeds or fails | acceptance::multi_source_resume_deduplication_layout_budget_and_verification; outcomes::source_read_failure_and_concurrent_command_are_unsuccessful; publication::real_publication_preserves_conflicts_and_failure_keeps_pending |
| Duplicates arrive before publication | pipeline::pending_duplicate_waits_for_shared_commit; catalog::pending_dependencies_completion_and_run_selection |
| Duplicate arrives in a later import | pipeline::bounded_single_pass_and_cross_import_empty_deduplication; acceptance::multi_source_resume_deduplication_layout_budget_and_verification |
| Empty files | pipeline::bounded_single_pass_and_cross_import_empty_deduplication |
| Staging budget is exhausted | lib staging::full_budget_and_zero_byte_bounds; pipeline::bounded_single_pass_and_cross_import_empty_deduplication; pipeline::retained_stage_after_read_failure_prevents_new_admission |
| Oversized duplicate | pipeline::oversized_passes_and_gate_access_counters |
| Oversized unique file | pipeline::oversized_passes_and_gate_access_counters; acceptance::multi_source_resume_deduplication_layout_budget_and_verification |
| Independent disks have queued work | pipeline::source_and_destination_workers_overlap_on_independent_devices; lib disk::independent_overlap_and_shared_serialization (barriers, no elapsed-time assertion) |
| Oversized fallback reads the source | pipeline::oversized_passes_and_gate_access_counters; lib disk::independent_overlap_and_shared_serialization |

## import-progress

| Scenario | Validation evidence |
| --- | --- |
| Start and resume a run | acceptance::multi_source_resume_deduplication_layout_budget_and_verification; crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy; inventory::nested_local_order_and_empty_inventory |
| Invalid configuration | cli::accepts_all_commands_and_rejects_invalid_arguments; cli::roots_and_missing_dependency_fail_before_execution; cli::symlink_roots_cannot_hide_overlap |
| Pending duplicate dependency | pipeline::pending_duplicate_waits_for_shared_commit (durable counts and progress snapshot) |
| Source reading finishes first | pipeline::pending_duplicate_waits_for_shared_commit (no unseen source bytes, pending destination demand); lib progress::byte_weighted_eta_overlap_serialization_duplicates_and_backlog |
| Initial observations are insufficient | lib progress::startup_zero_bytes_verification_and_smoothing; outcomes::successful_import_verification_mismatch_and_invalid_invocation |
| Import complete without verification | outcomes::successful_import_verification_mismatch_and_invalid_invocation |
| Failure or interruption | outcomes::failed_copy_and_interruption_identify_resumable_work; outcomes::source_read_failure_and_concurrent_command_are_unsuccessful; pipeline::full_queues_failure_and_interruption_shutdown |

## import-recovery

| Scenario | Validation evidence |
| --- | --- |
| Publication succeeds | publication::real_publication_preserves_conflicts_and_failure_keeps_pending (ordered hooks, real filesystem and SQLite); pipeline::pending_duplicate_waits_for_shared_commit |
| Conflicting final content | recovery::validate_adopt_retry_conflict_and_completed_cleanup; publication::no_replace_and_unsupported_flush_are_explicit |
| Crash after publication | crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy; recovery::validate_adopt_retry_conflict_and_completed_cleanup |
| Invalid staging or partial temporary content | recovery::validate_adopt_retry_conflict_and_completed_cleanup; crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy |
| Crash after completion before cleanup | crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy; recovery::validate_adopt_retry_conflict_and_completed_cleanup |
| Incomplete shared copy | catalog::pending_dependencies_completion_and_run_selection; pipeline::pending_duplicate_waits_for_shared_commit; pipeline::full_queues_failure_and_interruption_shutdown |
| Completed import with mismatched destination | verification::pass_results_order_shared_dependencies_and_fresh_restart; outcomes::successful_import_verification_mismatch_and_invalid_invocation |
| Interrupted verification | verification::pass_results_order_shared_dependencies_and_fresh_restart |
| Directory flush fails | publication::no_replace_and_unsupported_flush_are_explicit (/proc rejects fsync); publication::real_publication_preserves_conflicts_and_failure_keeps_pending (failure boundaries preserve pending dependencies) |

## media-organization

| Scenario | Validation evidence |
| --- | --- |
| Supported and other extensions | lib media::exact_extensions_and_dates |
| Metadata takes priority | media::real_image_video_and_staged_content; lib media::priority_and_layout_exclude_staging_times; acceptance::multi_source_resume_deduplication_layout_budget_and_verification |
| Unusable metadata | media::tag_priority_invalid_absent_and_filesystem_exclusion; media::real_image_video_and_staged_content; lib media::priority_and_layout_exclude_staging_times |
| Dated and undated media | lib media::priority_and_layout_exclude_staging_times |
| Equal names with different content | naming::names_representatives_and_provenance_survive_completion_order_and_resume; acceptance::multi_source_resume_deduplication_layout_budget_and_verification |
| Worker timing changes | naming::names_representatives_and_provenance_survive_completion_order_and_resume (24 seeded completion-order permutations); acceptance::multi_source_resume_deduplication_layout_budget_and_verification (clean vs interrupted digest/path maps) |
