# Spec scenario review

Reviewed against the current main specs on 2026-10-20. Test names below are
implemented and executed; `lib` means unit tests within the indicated module.
This map identifies test evidence, not exhaustive proof of every input. Platform,
fixture and durability limits remain as documented in README.md.

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
| No final reporting opportunity | crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy; report::missing_catalog_is_not_created_and_empty_catalog_lists_cleanly |
| Successful operation produces a review | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification |
| Resume or verification refreshes a report | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification; report::verdict_distinguishes_unverified_skips_and_broken_inventory_totals |
| Handled failure or interruption | outcomes::failed_copy_and_interruption_identify_resumable_work; report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification |
| Report persistence fails | report::report_save_failure_preserves_completed_work_and_current_report |
| Prominent reconciled verdict | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification; report::legacy_catalog_migration_and_unfinished_inventory_are_honest; report::verdict_distinguishes_unverified_skips_and_broken_inventory_totals |
| Resume after completion was committed but the report was not saved | crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy |
| Generate a comprehensive review | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification; report::verdict_distinguishes_unverified_skips_and_broken_inventory_totals |
| Duplicates share a destination dependency | pipeline::pending_duplicate_waits_for_shared_commit; report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification |
| Latest verification contains failures | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification; outcomes::successful_import_verification_mismatch_and_invalid_invocation |
| Saved invocation statistics are historical | report::stale_legacy_and_missing_saved_text_do_not_control_current_totals_or_verdict |
| No verification pass exists | report::verdict_distinguishes_unverified_skips_and_broken_inventory_totals |
| Select and display duplicate details | report::duplicate_details_limit_groups_and_paths_and_escape_names; report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification |
| Interpret a catalog snapshot | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification; report::stale_legacy_and_missing_saved_text_do_not_control_current_totals_or_verdict |
| Same content appears in earlier runs and twice in this source | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification |
| Inventory or hashing is unfinished | report::legacy_catalog_migration_and_unfinished_inventory_are_honest |
| Duplicate details exceed the display limit | report::duplicate_details_limit_groups_and_paths_and_escape_names |
| List catalog runs | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification; report::missing_catalog_is_not_created_and_empty_catalog_lists_cleanly |
| View a run without recorded invocation metrics | report::stale_legacy_and_missing_saved_text_do_not_control_current_totals_or_verdict |
| Unsupported catalog version | catalog::version_lock_pragmas_and_destination_association; report::legacy_catalog_migration_and_unfinished_inventory_are_honest |
| Inspect a saved run while a writer holds the lock | report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification |
| Saved verification verdict is stale after a crash | crash::crashes_during_verification_or_report_replacement_never_show_a_stale_verdict |
| An older or abruptly terminated run has no saved report | report::stale_legacy_and_missing_saved_text_do_not_control_current_totals_or_verdict; crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy |
| Missing catalog or unknown run | report::missing_catalog_is_not_created_and_empty_catalog_lists_cleanly; report::saved_reviews_cross_run_duplicates_read_only_commands_and_failed_verification |
| Writer opens a version-1 catalog | report::legacy_catalog_migration_and_unfinished_inventory_are_honest |
| Inspect a version-1 catalog | report::legacy_catalog_migration_and_unfinished_inventory_are_honest |

## import-recovery

| Scenario | Validation evidence |
| --- | --- |
| Catalog path contains new directories | crash::every_crash_window_recovers_without_overwrite_or_duplicate_copy |
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
| Metadata takes priority | media::real_image_video_and_staged_content (JPEG, PNG, MP4, MOV); lib media::priority_and_layout_exclude_filesystem_times; acceptance::multi_source_resume_deduplication_layout_budget_and_verification |
| Unusable metadata | media::tag_priority_invalid_absent_and_filesystem_exclusion; media::real_image_video_and_staged_content; lib media::priority_and_layout_exclude_filesystem_times |
| Implausible dates are rejected silently | lib media::plausible_dates_include_1900_and_tomorrow_with_literal_offsets; media::implausible_metadata_silently_tries_other_embedded_dates; acceptance::implausible_dates_are_silent_and_unknown_media_uses_filename_pattern_folders |
| Identifier digits resemble a date | lib media::exact_extensions_and_dates; acceptance::implausible_dates_are_silent_and_unknown_media_uses_filename_pattern_folders |
| Filesystem timestamps are available but no capture date is usable | lib media::rejected_metadata_falls_back_to_filename_without_using_filesystem_dates; acceptance::implausible_dates_are_silent_and_unknown_media_uses_filename_pattern_folders |
| Dated and undated media | lib media::priority_and_layout_exclude_filesystem_times; acceptance::implausible_dates_are_silent_and_unknown_media_uses_filename_pattern_folders |
| Similar unknown filenames share a folder | lib media::unknown_filename_patterns_group_numbers_identifiers_and_safe_names; acceptance::implausible_dates_are_silent_and_unknown_media_uses_filename_pattern_folders |
| Unknown identifiers and unmatched names | lib media::unknown_filename_patterns_group_numbers_identifiers_and_safe_names; acceptance::implausible_dates_are_silent_and_unknown_media_uses_filename_pattern_folders |
| Equal names with different content | naming::names_representatives_and_provenance_survive_completion_order_and_resume; acceptance::multi_source_resume_deduplication_layout_budget_and_verification |
| Worker timing changes | naming::names_representatives_and_provenance_survive_completion_order_and_resume (24 seeded completion-order permutations); acceptance::multi_source_resume_deduplication_layout_budget_and_verification (clean vs interrupted digest/path maps) |
