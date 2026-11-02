//! The exit checklist of IP-13, the V17 adapter's read half (phase 13M, task T13.15; the criteria of section 7 of the plan).
//!
//! * **What can be read off the code is read off it**: each criterion lists the tests that hold it, by name and by file, and the checklist fails if one is not there (a test that was renamed or
//!   deleted is a criterion that no longer holds). The tests themselves are run by `cargo test --workspace`, which is criterion 1 with clippy, `fmt`, the MSRV, the facade without default
//!   features and the Model's `wasm32` build, all of which are the process of each phase and not a test.
//! * **Four criteria are facts, and are checked as facts, through the facade alone, on every one of the six V17 miniatures**: each is in the gated list (7); each opens as schema 17 with the capabilities
//!   that the reader returns and nothing for writing (5 and 9); what those capabilities declare is in the files that carry it (9); a sample column of each reads (10); and **nothing in any of them is
//!   unreadable or unsupported and nothing is a fault** (7). No `unsafe` was added and the Model still has no HDF5.
//! * **What lives outside the repository's tracked files is not checked here**: the lab, the oracle's Python tests, the field inventory (criterion 8's inventory half: `python -m bl_fixtures.field_inventory
//!   --check`, and the reconciliation of its rows with what the adapter types or keeps, recorded in the plan), the plans and the hand-offs (11).

#![cfg(feature = "hdf5")]

#[allow(dead_code)]
mod exit_review;

use std::fs;

use build_log::{
    open, Capabilities, ChannelKind, ColumnRange, ColumnRequest, Direction, Feature, Outcome,
    Presence, ReadContext,
};
use build_log_testkit::{miniature, miniature_path, testdata_dir};
use exit_review::{hdf5_in_the_model, rust, tests_that_are_gone, unsafe_in_the_workspace};

/// The six V17 miniatures: the name of the test kit's constant (which the gated list of the gate test must name), the file, and its expectation file.
const V17_MINIATURES: [(&str, &str, &str); 6] = [
    (
        "V17_TRIGGERED_2026_10_19",
        miniature::V17_TRIGGERED_2026_10_19,
        "v17_triggered_2026_10_19.json",
    ),
    ("V17_MOCK", miniature::V17_MOCK, "v17_mock.json"),
    ("V17_OCT15", miniature::V17_OCT15, "v17_oct15.json"),
    ("V17_MARCH", miniature::V17_MARCH, "v17_march.json"),
    (
        "V17_SPECIAL_LAYERS",
        miniature::V17_SPECIAL_LAYERS,
        "special_layers_v17.json",
    ),
    (
        "V17_2026_08_18_IMAGES",
        miniature::V17_2026_08_18_IMAGES,
        "v17_2026_08_18_images.json",
    ),
];

/// The criteria that name tests: the criterion, the file (below `rust/`) and the tests that hold it.
const CHECKLIST: &[(&str, &str, &[&str])] = &[
    // ---- the gate exit (criteria 2 to 5, true after 13D)
    (
        "2: the gate file conforms to its whole expectation file, and the other files are reported",
        "io/tests/v17_gate.rs",
        &[
            "the_gate_file_conforms",
            "the_other_v17_files_are_reported_and_not_asserted",
            "a_file_that_says_17_but_has_no_monitoring_data_is_corrupt",
            "a_nested_v17_payload_reads_the_same_as_a_standalone_file",
        ],
    ),
    (
        "4: layout names appear only in the layout module of an adapter, and the guard rejects one outside it",
        "io/tests/adapter_isolation.rs",
        &[
            "layout_names_appear_only_in_the_layout_module",
            "the_layout_guard_rejects_a_name_outside_the_layout_module",
        ],
    ),
    (
        "5: the V17 adapter is registered under 17, and the facade opens a V17 file and reports schema 17",
        "io/tests/v17_adapter.rs",
        &["the_v17_adapter_is_registered_under_17"],
    ),
    (
        "5: the facade opens the gate file and reports schema 17 and the capabilities that the reader returns",
        "build-log/tests/facade.rs",
        &["the_facade_opens_the_gate_file_and_reports_schema_17"],
    ),
    // ---- the tolerant values, the build level, the layers, the lasers
    (
        "7: the tolerant values (a number as a scalar or an array of one, text as variable or fixed length, an absent attribute is none, several values are not read as the first)",
        "io/tests/v17_adapter.rs",
        &[
            "a_number_stored_as_a_scalar_or_an_array_of_one_reads_the_same",
            "text_stored_as_variable_or_fixed_length_reads_the_same",
            "an_absent_optional_attribute_is_none_and_not_a_default",
            "an_attribute_with_several_values_where_one_is_expected_is_not_read_as_its_first",
            "the_build_level_of_the_gate_file_conforms",
        ],
    ),
    (
        "7: the conformance crate holds the parts, the images and the events, and each is shown to fail on a changed build",
        "conformance/tests/conformance.rs",
        &[
            "the_parts_together_equal_the_whole",
            "the_synthetic_cdm_with_images_passes_and_a_changed_one_fails",
            "the_synthetic_cdm_with_events_passes_and_a_changed_one_fails",
        ],
    ),
    (
        "7: layers (ids from the group names, times as written, a placeholder thickness is not recorded)",
        "io/tests/v17_layers.rs",
        &[
            "layer_ids_come_from_the_group_names_and_may_have_gaps",
            "layer_times_are_the_integers_as_written",
            "a_placeholder_thickness_is_not_recorded_and_keeps_its_raw_value",
            "the_layers_of_the_gate_file_conform",
        ],
    ),
    (
        "7: lasers, presence, channels and the time axis, and a laser that cannot be read is reported on itself",
        "io/tests/v17_lasers.rs",
        &[
            "a_laser_group_without_samples_is_recorded_empty",
            "a_missing_laser_group_is_not_recorded",
            "a_channel_carries_its_kind_axis_type_stored_type_and_declared_unit",
            "the_time_axis_window_is_the_lasers_own_times_as_written",
            "the_sample_period_is_paired_with_the_laser_by_number",
            "a_laser_that_cannot_be_read_is_recorded_as_not_read_and_the_build_still_opens",
            "the_lasers_of_the_gate_file_conform",
        ],
    ),
    // ---- criterion 7: all six conform
    (
        "7: the mock capture, the 2026-10-15 build, the March build and the special-layer file conform, and the March flavor is read as recorded",
        "io/tests/v17_files.rs",
        &[
            "the_mock_capture_conforms",
            "the_2026_10_15_file_conforms_with_its_second_laser_recorded_empty",
            "fixed_length_text_reads_as_text_without_its_padding",
            "the_march_file_conforms",
            "the_special_layers_conform_with_laser_2_recorded_empty_on_layers_2007_and_2008",
            "the_declared_count_stays_2110_with_seven_layers_present",
            "march_power_min_and_max_are_kept_in_extra_as_recorded_with_their_stored_size",
        ],
    ),
    (
        "7 and 8: the build's context (IP-02 phase 2V): the seven fields of every flavor, the plate in the vocabulary of its file and neither converted, the placeholders that are not recorded, and what cannot be read reported on itself and kept",
        "io/tests/v17_context.rs",
        &[
            "the_seven_fields_of_every_flavor_become_context_fields",
            "the_current_plate_vocabulary_becomes_axes_with_their_units",
            "the_older_plate_vocabulary_becomes_a_dimension_and_a_shape",
            "a_zero_thickness_weight_or_dimension_is_not_recorded_and_keeps_its_raw_value",
            "a_starting_height_of_zero_is_recorded",
            "a_dimension_of_280_is_recorded_as_written",
            "a_context_attribute_with_several_values_is_reported_on_itself_and_kept",
            "what_is_typed_leaves_extra",
        ],
    ),
    (
        "7: the 2026-08-18 file conforms, with its camera images described and declared",
        "io/tests/v17_images.rs",
        &[
            "the_images_file_conforms",
            "an_image_is_a_channel_of_the_build_with_its_frame_and_its_layers",
            "images_are_declared_in_the_capabilities",
        ],
    ),
    (
        "7: parameters (the scanner, the galvos, the optical train and the light source, per laser)",
        "io/tests/v17_parameters.rs",
        &[
            "the_scanner_and_light_source_attributes_become_parameters_per_laser",
            "volts_to_watts_becomes_named_constants_in_both_flavors",
            "an_unknown_algorithm_stays_other",
            "the_formula_images_are_kept_opaque",
            "both_sample_period_forms_become_one_quantity",
            "a_power_limit_stored_as_text_is_not_recorded_as_a_number",
        ],
    ),
    (
        "7: outcomes (the complete flag as written and never derived, the build status, the join)",
        "io/tests/v17_outcomes.rs",
        &[
            "the_complete_flag_is_kept_as_written_and_never_derived",
            "build_status_becomes_the_builds_outcome",
            "the_join_status_and_version_become_provenance",
            "outcomes_are_declared_in_the_capabilities",
        ],
    ),
    (
        "7: the OPC UA events (per layer and unassigned, a task with its interval and its height)",
        "io/tests/v17_events.rs",
        &[
            "the_gate_files_events_are_all_in_the_bucket_that_no_layer_owns",
            "a_task_event_keeps_its_interval_and_its_height",
            "events_are_declared_in_the_capabilities",
        ],
    ),
    // ---- criterion 8: nothing is dropped, on every file
    (
        "8: nothing in any of the six miniatures is dropped: each node and attribute is typed or kept in `extra`, and never both",
        "io/tests/v17_gate.rs",
        &[
            "nothing_in_the_gate_file_is_dropped",
            "nothing_in_the_images_file_is_dropped",
            "nothing_in_the_other_v17_files_is_dropped",
        ],
    ),
    (
        "8: nothing on the mount is dropped by the build level, and unknown attributes and nodes are kept with their encoding, their shape and their type",
        "io/tests/v17_adapter.rs",
        &["nothing_on_the_mount_is_dropped_by_the_build_level"],
    ),
    (
        "8: `extra` keeps what is not typed, never holds bulk data, and compares equal after a clone",
        "model/src/extra.rs",
        &[
            "unknown_attributes_are_kept_with_their_encoding_and_value",
            "an_unmapped_node_is_kept_by_path_with_its_shape_and_type",
            "extra_never_holds_bulk_data",
            "extra_compares_equal_after_a_clone",
        ],
    ),
    // ---- criterion 9: the capabilities match what the reader returns
    (
        "9: Samples, EventColumns and ImageRegions are declared, and each is read",
        "io/tests/v17_columns.rs",
        &["samples_are_declared_in_the_capabilities"],
    ),
    (
        "9: EventColumns are declared in the capabilities",
        "io/tests/v17_event_columns.rs",
        &["event_columns_are_declared_in_the_capabilities"],
    ),
    (
        "9: ImageRegions are declared in the capabilities",
        "io/tests/v17_image_regions.rs",
        &["image_regions_are_declared_in_the_capabilities"],
    ),
    (
        "9: for each of the columns of the six files, the report says readable exactly when the read succeeds",
        "io/tests/v17_reports.rs",
        &["capabilities_and_availability_agree"],
    ),
    // ---- criterion 10: the samples
    (
        "10: samples read through the primitives, ranged reads equal slices of the full read, a read can be cancelled, and the March one-chunk datasets read",
        "io/tests/v17_columns.rs",
        &[
            "a_ranged_read_equals_a_slice_of_the_full_read",
            "a_read_can_be_cancelled",
            "the_march_one_chunk_datasets_read",
            "every_stored_column_of_the_six_v17_files_matches_its_fingerprint",
        ],
    ),
    (
        "10: the facade reads a column of the gate file with nothing but itself",
        "build-log/tests/facade.rs",
        &["a_column_of_the_gate_file_reads_through_the_facade_alone"],
    ),
    // ---- the shape of the code
    (
        "no layout word in the engine or the primitives, and each guard rejects a source made to break it",
        "io/tests/primitives_layout_guard.rs",
        &[
            "no_layout_name_appears_in_the_primitives",
            "the_engine_names_no_layout_word",
            "the_scan_of_a_folder_rejects_a_source_made_to_break_it",
        ],
    ),
];

/// What the V17 reader returns, as the facade's own test of the gate file says it: the metadata, the outcomes, the images, the events, the samples of the lasers, the columns of the events and the
/// regions of the frames, and nothing for writing (the writer arrives with IP-08).
fn v17_capabilities() -> Capabilities {
    Capabilities::new()
        .with(Direction::Read, Feature::Outcomes)
        .with(Direction::Read, Feature::Images)
        .with(Direction::Read, Feature::Events)
        .with(Direction::Read, Feature::Samples)
        .with(Direction::Read, Feature::EventColumns)
        .with(Direction::Read, Feature::ImageRegions)
}

#[test]
fn ip13_read_half_exit_checklist_holds() {
    // the criteria that name tests: each must still be where the plan says
    let mut missing = tests_that_are_gone(CHECKLIST);

    // 9: no `unsafe` in any crate of the workspace (13I, the one shim, was dropped) and the lint that denies it is still there, and the Model still has no HDF5
    missing.extend(unsafe_in_the_workspace());
    missing.extend(hdf5_in_the_model());

    // 7: every V17 miniature is in the gated list of the gate test, and has its expectation file
    let gate = fs::read_to_string(rust().join("io/tests/v17_gate.rs")).expect("the gate test");
    let gated = gate
        .split("const GATED")
        .nth(1)
        .and_then(|after| after.split("];").next())
        .unwrap_or("");
    if !gated.contains("[&str; 6]") {
        missing
            .push("criterion 7: the gated list of v17_gate.rs does not hold six files".to_string());
    }
    for (constant, file, expectation) in V17_MINIATURES {
        let constant = format!("miniature::{constant}");
        if !gated.contains(&constant) {
            missing.push(format!(
                "criterion 7: {file} is not in the gated list of v17_gate.rs ({constant})"
            ));
        }
        let json = testdata_dir().join("expectations").join(expectation);
        if !json.is_file() {
            missing.push(format!(
                "criterion 7: the expectation file of {file} is not there ({})",
                json.display()
            ));
        }
    }

    // 5, 7, 9 and 10, through the facade alone, on every one of the six
    let (mut has_events, mut has_images, mut has_outcome) = (false, false, false);
    for (_, file, _) in V17_MINIATURES {
        let opened = match open(miniature_path(file)) {
            Ok(opened) => opened,
            Err(error) => {
                missing.push(format!("criterion 7: {file} does not open: {error}"));
                continue;
            }
        };
        let report = opened.report();
        if (report.schema_version.as_str(), report.adapter.as_str()) != ("17", "v17") {
            missing.push(format!(
                "criterion 5: {file} opens as schema {} with adapter {}",
                report.schema_version, report.adapter
            ));
        }
        // 9: what V17 declares is what its reader returns, the same for every file of the version, and nothing is declared for writing before the writer exists
        if *opened.capabilities() != v17_capabilities() {
            missing.push(format!(
                "criterion 9: {file} declares {:?} and the reader returns {:?}",
                opened.capabilities(),
                v17_capabilities()
            ));
        }
        let source = opened.source();
        let build = source.build().expect("the metadata");
        // 7: the read is complete: nothing in the file is unreadable or unsupported, and nothing is a fault
        let availability = source.availability().expect("the availability");
        let counts = availability.counts();
        if counts.unreadable != 0 || counts.unsupported != 0 {
            missing.push(format!(
                "criterion 7: {file} has {} unreadable and {} unsupported items",
                counts.unreadable, counts.unsupported
            ));
        }
        let findings = source.validate().expect("the findings");
        if findings.counts.faults != 0 {
            missing.push(format!(
                "criterion 7: {file} has {} faults in its validation report",
                findings.counts.faults
            ));
        }
        // 9: what is declared is in the files that carry it
        has_events |= !build.events.is_empty();
        has_images |= build.sensors.iter().any(|c| c.kind == ChannelKind::Image);
        has_outcome |= build.quality.outcome != Outcome::NotRecorded;
        // 10: a sample column of the file reads, as stored and as the metadata counts it
        let column = build.lasers().iter().find_map(|laser| {
            laser
                .channels
                .iter()
                .filter(|c| c.kind == ChannelKind::LaserPower)
                .find_map(|channel| {
                    channel
                        .presence
                        .iter()
                        .find(|(_, p)| matches!(p, Presence::Recorded { .. }))
                        .map(|(layer, p)| {
                            (laser.key.clone(), channel.key.clone(), layer, p.samples())
                        })
                })
        });
        match column {
            None => missing.push(format!("criterion 10: {file} has no recorded power column")),
            Some((laser, channel, layer, samples)) => {
                let asked = ColumnRequest::new(layer, Some(laser), channel)
                    .with_range(ColumnRange::Indices(0..samples.unwrap_or(0).min(1_000)));
                match source.read(&asked, &ReadContext::new()) {
                    Ok(got) if got.len() as u64 == samples.unwrap_or(0).min(1_000) => {}
                    Ok(got) => missing.push(format!(
                        "criterion 10: a read of {file} gave {} values and the metadata says {samples:?}",
                        got.len()
                    )),
                    Err(error) => {
                        missing.push(format!("criterion 10: a read of {file} fails: {error}"))
                    }
                }
            }
        }
    }
    for (found, what) in [
        (has_events, "events"),
        (has_images, "camera images"),
        (has_outcome, "a build outcome"),
    ] {
        if !found {
            missing.push(format!(
                "criterion 9: V17 declares it reads {what} and none of the six files holds any"
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "the exit checklist of IP-13's read half does not hold:\n{}",
        missing.join("\n")
    );
}
