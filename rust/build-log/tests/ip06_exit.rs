//! The exit checklist of IP-06, column access (phase 6H, task T06.8; the criteria of section 7 of the plan).
//!
//! * **What can be read off the code is read off it**: each criterion lists the tests that hold it, by name and by file, and the checklist fails if one is not there (a test that was renamed or
//!   deleted is a criterion that no longer holds). The tests themselves are run by `cargo test --workspace`, which is criterion 1 with clippy, `fmt`, the MSRV, the facade without default
//!   features and the Model's `wasm32` build, all of which are the process of each phase and not a test.
//! * **Three criteria are facts, and are checked as facts**: the reader declares what it returns (6 and 8), no `unsafe` was added and the Model still has no HDF5 (9).
//! * **What lives outside the repository's tracked files is not checked here**: the lab's tests (the lab is its own workspace, kept local), the oracle's Python tests, the plans and Appendix B. The plan
//!   records them in its section 7.

#![cfg(feature = "hdf5")]

#[allow(dead_code)]
mod exit_review;

use build_log::{open, Direction, Feature};
use build_log_testkit::{miniature, miniature_path};
use exit_review::{has_unsafe, hdf5_in_the_model, tests_that_are_gone, unsafe_in_the_workspace};

/// The criteria that name tests: the criterion, the file (below `rust/`) and the tests that hold it.
const CHECKLIST: &[(&str, &str, &[&str])] = &[
    (
        "2: every stored laser channel of the six V17 files equals its fingerprint, and a ranged read equals the slice of the full read",
        "io/tests/v17_columns.rs",
        &[
            "every_stored_column_of_the_six_v17_files_matches_its_fingerprint",
            "a_ranged_read_equals_a_slice_of_the_full_read",
        ],
    ),
    (
        "2: the conformance check reports each way a column can be served wrongly",
        "conformance/tests/columns.rs",
        &[
            "a_source_that_serves_the_columns_as_stored_passes",
            "a_changed_value_a_dropped_value_a_shifted_range_and_a_wrong_type_each_fail",
        ],
    ),
    (
        "2: ranged reads equal slices of the full read, through the primitive, the engine and the cache",
        "io/tests/ranged_reads.rs",
        &["ranged_reads_equal_slices_of_the_full_read"],
    ),
    (
        "2: the same through the engine, and across the blocks of the cache",
        "io/tests/column_engine.rs",
        &[
            "a_ranged_read_equals_a_slice_of_the_full_read",
            "the_stored_type_decides_the_variant",
            "precision_converts_block_by_block_and_equals_the_whole_column_conversion",
            "an_empty_range_is_an_empty_column",
            "a_range_outside_the_column_is_invalid_index_and_says_the_stored_count",
            "a_time_range_is_unsupported_and_says_why",
        ],
    ),
    (
        "3: a cancelled read returns Cancelled and nothing partial; progress is monotonic and ends at the total",
        "io/tests/column_engine.rs",
        &[
            "a_cancelled_read_returns_cancelled_and_no_partial_column",
            "progress_ends_at_the_total_values",
        ],
    ),
    (
        "3: the primitive checks the token before each block and reports progress in order",
        "io/tests/ranged_reads.rs",
        &[
            "cancel_returns_cancelled_at_a_block_boundary",
            "progress_is_reported_monotonically",
            "a_range_outside_the_dataset_is_invalid_index",
        ],
    ),
    (
        "4: the March one-chunk shape reads",
        "io/tests/v17_columns.rs",
        &["the_march_one_chunk_datasets_read"],
    ),
    (
        "4: with a budget a warm range read does not decode, and the cache keeps to its budget",
        "io/tests/column_cache.rs",
        &[
            "a_warm_read_does_not_touch_the_file",
            "a_context_with_no_budget_retains_nothing",
            "the_cache_never_holds_more_than_the_budget",
            "the_least_recently_used_block_goes_first",
            "a_block_larger_than_the_budget_is_served_and_not_cached",
            "a_range_inside_one_block_is_a_slice_of_the_cached_buffer",
            "a_range_across_blocks_equals_the_slice_of_the_full_read",
            "a_precision_request_is_converted_from_the_cached_native_block",
            "the_stats_count_hits_misses_and_bytes",
        ],
    ),
    (
        "5: reads from several threads give identical results, and a block is decoded once",
        "io/tests/column_concurrency.rs",
        &[
            "the_source_is_send_and_sync",
            "concurrent_reads_of_different_layers_equal_the_serial_result",
            "results_do_not_depend_on_the_number_of_threads",
            "a_block_that_two_readers_need_is_decoded_once",
            "cancelling_one_reader_does_not_fail_the_other",
            "a_reader_blocked_in_the_locator_does_not_block_the_cache",
        ],
    ),
    (
        "5: the one-fill rule, hand-driven",
        "io/src/columns/cache.rs",
        &[
            "a_waiter_takes_over_a_fill_whose_reader_was_cancelled",
            "a_block_that_two_readers_need_is_decoded_once",
        ],
    ),
    (
        "6: Samples is declared and the samples read through the facade alone",
        "io/tests/v17_columns.rs",
        &["samples_are_declared_in_the_capabilities"],
    ),
    (
        "6 and 8: the facade reads a column, an event place and a frame region with nothing but itself",
        "build-log/tests/facade.rs",
        &[
            "a_column_of_the_gate_file_reads_through_the_facade_alone",
            "an_event_place_of_the_gate_file_reads_through_the_facade_alone",
            "a_frame_region_of_the_images_file_reads_through_the_facade_alone",
        ],
    ),
    (
        "8: the event columns of the two files equal their fingerprints, and the typed view says what is not a reading",
        "io/tests/v17_event_columns.rs",
        &[
            "every_event_place_of_the_two_files_matches_its_fingerprint",
            "a_places_timestamps_and_values_read_as_stored",
            "a_double_series_reads_as_numbers_and_an_unparsable_value_is_invalid_not_zero",
            "a_boolean_series_reads_as_flags",
            "a_series_of_another_type_has_the_text_only",
            "a_values_column_shorter_than_its_timestamps_is_reported_and_not_padded",
            "a_task_has_its_start_and_its_stop",
            "event_columns_are_declared_in_the_capabilities",
        ],
    ),
    (
        "8: the event conformance check reports each way a place can be served wrongly",
        "conformance/tests/event_columns.rs",
        &[
            "the_fingerprint_of_known_places_is_what_hashlib_gives",
            "the_synthetic_source_with_events_passes_and_a_changed_one_fails",
        ],
    ),
    (
        "8: the frames of the images file equal their fingerprints, a region is the rectangle asked for with rows and columns not mixed up",
        "io/tests/v17_image_regions.rs",
        &[
            "a_frame_reads_as_stored",
            "a_region_equals_the_slice_of_the_frame_with_rows_and_columns_not_mixed_up",
            "a_region_outside_the_frame_is_invalid_index_and_an_empty_region_is_not_an_error",
            "a_cancelled_frame_read_returns_cancelled",
            "image_regions_are_declared_in_the_capabilities",
        ],
    ),
    (
        "8: the frame conformance check reports each way a frame and a region can be served wrongly",
        "conformance/tests/image_regions.rs",
        &[
            "the_synthetic_source_with_frames_passes_and_a_changed_one_fails",
            "a_source_that_serves_the_whole_frame_right_and_a_region_wrong_is_found",
        ],
    ),
    (
        "9: no layout word in the engine or the primitives, and each guard rejects a source made to break it",
        "io/tests/primitives_layout_guard.rs",
        &[
            "no_layout_name_appears_in_the_primitives",
            "the_engine_names_no_layout_word",
            "the_scan_of_a_folder_rejects_a_source_made_to_break_it",
        ],
    ),
    (
        "9: the Model's guards for the new types (who builds them, bulk data, closed or non-exhaustive enums, no layout name)",
        "model/tests/policy_guards.rs",
        &[
            "enums_are_non_exhaustive_or_marked_closed",
            "every_public_struct_declares_who_constructs_it",
            "producer_structs_are_plain_and_library_structs_are_non_exhaustive",
            "bulk_data_is_never_a_metadata_field",
            "no_layout_names_in_public_types",
            "each_policy_guard_rejects_a_synthetic_violation",
        ],
    ),
    (
        "10: IP-03's 3F (the ranged reader) is done, and IP-13's 13L (the samples of V17) is done",
        "io/tests/ranged_reads.rs",
        &[
            "ranged_reads_equal_slices_of_the_full_read",
            "cancel_returns_cancelled_at_a_block_boundary",
        ],
    ),
];

#[test]
fn ip06_exit_checklist_holds() {
    // the criteria that name tests: each must still be where the plan says
    let mut missing = tests_that_are_gone(CHECKLIST);

    // 9: no `unsafe` in any crate of the workspace and the lint that denies it is still there, and the Model still has no HDF5
    missing.extend(unsafe_in_the_workspace());
    missing.extend(hdf5_in_the_model());

    // 6 and 8: V17 declares what its reader returns, and declares nothing it cannot do (the writer arrives with IP-08)
    let opened =
        open(miniature_path(miniature::V17_TRIGGERED_2026_10_19)).expect("the gate file opens");
    let reads = opened.capabilities().features(Direction::Read);
    for feature in [
        Feature::Samples,
        Feature::EventColumns,
        Feature::ImageRegions,
        Feature::Events,
        Feature::Images,
    ] {
        if !reads.contains(&feature) {
            missing.push(format!(
                "criterion 6 and 8: V17 does not declare {feature:?} for reading"
            ));
        }
    }
    let writes = opened.capabilities().features(Direction::Write);
    if !writes.is_empty() {
        missing.push(format!(
            "criterion 6: V17 declares {writes:?} for writing before its writer exists"
        ));
    }

    assert!(
        missing.is_empty(),
        "the exit checklist of IP-06 does not hold:\n{}",
        missing.join("\n")
    );
}

#[test]
fn the_unsafe_scan_sees_unsafe_and_nothing_else() {
    // the scan that criterion 9 relies on finds the keyword, and not a lint, a comment or a longer word
    assert!(has_unsafe("    unsafe { call() }"));
    assert!(has_unsafe("pub unsafe fn f() {}"));
    assert!(has_unsafe("unsafe impl Send for X {}"));
    assert!(!has_unsafe("#![forbid(unsafe_code)]"));
    assert!(!has_unsafe("    // an unsafe block would be here"));
    assert!(!has_unsafe("let unsafely = 1;"));
}
