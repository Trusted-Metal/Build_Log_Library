//! The exit checklist of IP-07, discovery, availability and validation (phase 7H, task T07.8; the criteria of section 7 of the plan).
//!
//! * **What can be read off the code is read off it**: each criterion lists the tests that hold it, by name and by file, and the checklist fails if one is not there (a test that was renamed or
//!   deleted is a criterion that no longer holds). The tests themselves are run by `cargo test --workspace`, which is criterion 1 with clippy, `fmt`, the MSRV, the facade without default
//!   features and the Model's `wasm32` build, all of which are the process of each phase and not a test.
//! * **Three criteria are facts, and are checked as facts, through the facade alone**: every rule of the validation report fires on a build made to break it, with the severity it always has (5); every
//!   one of the five states occurs, and an item that cannot be read changes no other (4); no `unsafe` was added and the Model still has no HDF5 (9). **A rule or a state that disappears is a failure
//!   here even if the test that held it was deleted with it.**
//! * **What lives outside the repository's tracked files is not checked here**: the lab's tests (criterion 8; the lab is its own workspace, kept local), the oracle's Python tests, the plans, Appendix B
//!   (the open cost, criterion 6, is recorded and not gated, D06-15) and the hand-offs (10). The plan records them in its section 7.

#[allow(dead_code)]
mod exit_review;

use build_log::{
    Availability, BuildIdentity, BuildMeta, BuildSource, ByLayer, ChannelDType, ChannelDescriptor,
    ChannelKey, ChannelKind, ClockClass, Error, ErrorKind, FindingCode, InMemorySource, Item,
    LaserKey, LaserLayerMeta, LaserMeta, LayerId, LayerMeta, NumericType, Origin, Presence,
    Severity, TimeValue,
};
use exit_review::{hdf5_in_the_model, tests_that_are_gone, unsafe_in_the_workspace};

/// The criteria that name tests: the criterion, the file (below `rust/`) and the tests that hold it.
const CHECKLIST: &[(&str, &str, &[&str])] = &[
    (
        "2: a laser with positions and no power is recorded with the positions' count, a laser whose channels differ takes the longest and says so, a null power does not remove the laser",
        "io/tests/v17_streams.rs",
        &[
            "a_laser_with_positions_and_no_power_is_recorded_with_the_positions_count",
            "a_laser_whose_channels_differ_takes_the_longest_and_says_so",
            "a_laser_of_empty_channels_and_a_laser_with_no_channel_are_recorded_empty_with_no_origin_for_the_stream",
            "a_null_power_does_not_remove_the_laser",
            "a_power_that_cannot_be_read_leaves_the_laser_recorded_from_its_other_channels",
        ],
    ),
    (
        "2: a null-dataspace channel is recorded and empty with a typed origin, and a scalar is not a null dataspace",
        "io/tests/v17_streams.rs",
        &[
            "a_null_dataspace_channel_is_recorded_empty_with_an_origin",
            "a_scalar_channel_is_not_a_null_dataspace_and_is_not_read",
        ],
    ),
    (
        "2: the 12 scenarios of the edge-case run give the expected presence, lasers are numbered and not sorted as text, and the eight miniatures keep their expectations",
        "io/tests/v17_streams.rs",
        &[
            "the_twelve_edge_case_scenarios_give_the_expected_presence",
            "lasers_are_numbered_not_sorted_as_text",
            "the_eight_miniatures_keep_their_expectations",
        ],
    ),
    (
        "2: the primitive tells a null dataspace from a scalar",
        "io/tests/dataset_reader.rs",
        &["a_dataset_with_a_null_dataspace_is_described_as_null_and_a_scalar_is_not"],
    ),
    (
        "2: the engine reads a null dataspace as an empty column of the stored type",
        "io/tests/column_engine.rs",
        &["a_null_dataspace_channel_reads_as_an_empty_column_of_the_stored_type"],
    ),
    (
        "2: the two origins of a laser's stream are typed facts and not text",
        "model/src/provenance.rs",
        &["the_origins_of_a_laser_stream_are_two_typed_facts_and_not_text"],
    ),
    (
        "2: the conformance check holds both origins both ways",
        "conformance/tests/origins.rs",
        &[
            "the_origins_the_expectation_states_are_in_the_synthetic_build_and_the_check_holds_them",
            "an_origin_that_is_missing_changed_or_not_expected_is_reported_on_its_own_laser_layer_and_channel",
            "an_origin_that_the_expectation_does_not_state_is_reported",
        ],
    ),
    (
        "3: `int16` reads as stored, in ranges, converted, and cached, and the commanded positions of the local XY2-100 file read",
        "io/tests/column_widths.rs",
        &[
            "an_int16_dataset_reads_as_stored",
            "an_int16_range_equals_a_slice_of_the_full_read",
            "an_int16_column_converts_to_f32_and_f64_as_the_model_does",
            "an_int16_block_is_cached_by_its_size_and_served_warm",
            "the_xy2_100_fixture_commanded_positions_read",
        ],
    ),
    (
        "3: the engine's one list of readable types equals what the engine reads",
        "io/tests/column_widths.rs",
        &["the_readable_list_equals_what_the_engine_reads"],
    ),
    (
        "3: the fingerprint of the oracle and the library's agree on `int16` bytes",
        "conformance/tests/columns.rs",
        &["the_oracle_and_the_fingerprint_agree_on_int16_bytes"],
    ),
    (
        "3 and 4: a type the engine cannot read is Unsupported, and a read of it says the same",
        "io/tests/v17_reports.rs",
        &["a_type_the_engine_cannot_read_is_unsupported_and_a_read_of_it_says_the_same"],
    ),
    (
        "4: every item has one of the five states, which are derived from the presence, the cause of a failed read and the findings; the counts aggregate; one bad item changes no other",
        "model/src/availability.rs",
        &[
            "every_item_has_one_of_the_five_states",
            "a_recorded_channel_is_present_and_an_unrecorded_one_is_not_recorded",
            "a_channel_with_a_warning_is_degraded_and_lists_its_issues",
            "two_warnings_at_one_item_are_one_degradation_with_both_issues",
            "a_note_degrades_nothing",
            "a_channel_not_read_is_unreadable_and_the_others_are_present",
            "a_type_the_engine_cannot_read_is_unsupported_and_names_the_feature",
            "a_channel_that_the_source_cannot_read_is_unsupported_only_where_it_is_recorded",
            "the_strongest_state_wins_and_a_cause_that_is_the_librarys_is_unsupported",
            "the_counts_aggregate_the_states_overall_and_by_kind",
            "a_channel_is_counted_only_where_its_laser_is_recorded",
            "a_bad_laser_changes_no_other_laser",
        ],
    ),
    (
        "4 and 5: the findings and the availability of the six V17 files equal the independent expectations, a laser that cannot be read is a fault and changes no other, a short channel degrades only itself",
        "io/tests/v17_reports.rs",
        &[
            "the_findings_and_the_availability_of_the_six_v17_files_equal_the_expectations",
            "the_findings_of_the_special_layer_file_are_the_one_note_that_the_oracle_gives",
            "a_channel_shorter_than_the_stream_is_a_warning_and_degrades_only_that_channel",
            "a_laser_that_cannot_be_read_is_a_fault_and_unreadable_and_changes_no_other_laser",
            "a_layer_with_no_laser_is_a_note",
        ],
    ),
    (
        "4 and 5: the conformance checks report each way a finding and an availability can be wrong",
        "conformance/tests/reports.rs",
        &[
            "the_synthetic_builds_of_the_eight_expectations_give_the_findings_and_the_availability_they_state",
            "a_finding_that_is_missing_extra_of_another_severity_or_in_another_place_is_reported",
            "an_availability_that_is_changed_is_reported_by_its_count_and_its_item",
            "an_exception_that_is_missing_or_of_another_state_is_reported",
            "a_degraded_channel_that_the_expectation_states_is_held_to_it",
            "an_expectation_that_states_neither_is_an_error_and_not_a_pass",
        ],
    ),
    (
        "5: each rule fires on a build made to break it and is silent where it should be; the findings are counted and in a fixed order",
        "model/src/validation.rs",
        &[
            "a_declared_layer_count_that_differs_from_the_layers_present_is_a_note",
            "a_layer_that_ends_before_it_starts_is_a_warning",
            "overlapping_layers_are_a_warning_and_a_gap_is_not",
            "channels_of_different_lengths_give_a_warning_for_each_short_one",
            "a_channel_that_is_not_recorded_is_not_a_finding",
            "a_not_read_item_is_a_fault_and_an_unsupported_one_a_note",
            "a_layer_with_no_laser_is_a_note",
            "a_clean_build_has_no_findings",
            "the_counts_equal_the_findings_and_the_order_is_by_code_then_place",
            "a_finding_names_its_layer_laser_and_channel",
            "every_code_has_a_name_and_a_severity_and_the_codes_are_in_the_order_of_the_rules",
        ],
    ),
    (
        "6: the open of the many-layer file is timed (recorded, not gated), a second ask is an `Arc` clone, and the trims change no metadata (the golden over 28 V17 files)",
        "io/tests/open_cost.rs",
        &[
            "the_open_of_the_many_layer_file_is_timed",
            "a_second_ask_of_an_open_source_is_an_arc_clone",
            "trimming_changes_no_metadata",
        ],
    ),
    (
        "6: the light description of a dataset equals the full one where they overlap",
        "io/tests/dataset_reader.rs",
        &["the_light_description_of_a_dataset_equals_the_full_one_where_they_overlap"],
    ),
    (
        "6: a group's remainder kept from the names that were listed is the one that lists them",
        "io/tests/extra_nodes.rs",
        &["a_group_remainder_from_names_that_were_listed_is_the_one_that_lists_them"],
    ),
    (
        "6: a group of channels that is not a group, and a channel that is a group, are a fault of the laser",
        "io/tests/v17_streams.rs",
        &["a_group_of_channels_that_is_not_a_group_and_a_channel_that_is_a_group_are_a_fault_of_the_laser"],
    ),
    (
        "7: what V17 declares and what its availability says agree for every column, and the reports are computed once for a source",
        "io/tests/v17_reports.rs",
        &[
            "capabilities_and_availability_agree",
            "the_reports_are_computed_once_for_a_source",
        ],
    ),
    (
        "7: discovery and validation read no sample",
        "model/src/validation.rs",
        &["validation_reads_no_sample", "the_report_is_send_and_sync"],
    ),
    (
        "7: a source that is not a file has an availability that reads no sample and calls everything readable",
        "model/src/availability.rs",
        &["a_source_that_is_not_a_file_has_an_availability_that_reads_no_sample_and_calls_everything_readable"],
    ),
    (
        "4, 5 and 7: the facade asks for the findings and the availability with nothing but itself",
        "build-log/tests/facade.rs",
        &["availability_and_findings_through_the_facade_alone"],
    ),
    (
        "9: a failed read is a typed origin at every site and no free-text `not read` origin remains, and the guard rejects a source made to break it",
        "io/tests/typed_origins_guard.rs",
        &[
            "no_free_text_not_read_origin_remains",
            "the_guard_rejects_a_source_made_to_break_it",
        ],
    ),
    (
        "9: a channel that cannot be read is recorded as not read, with its cause",
        "io/tests/v17_streams.rs",
        &["a_channel_that_cannot_be_read_is_recorded_as_not_read_with_its_cause"],
    ),
    (
        "9: a laser that cannot be read is recorded as not read and the build still opens",
        "io/tests/v17_lasers.rs",
        &["a_laser_that_cannot_be_read_is_recorded_as_not_read_and_the_build_still_opens"],
    ),
    (
        "9: an unsupported type is told from a fault by the typed cause",
        "model/src/provenance.rs",
        &["an_unsupported_type_is_told_from_a_fault"],
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
        "9: layout names appear only in the layout module of an adapter, and the guard rejects one outside it",
        "io/tests/adapter_isolation.rs",
        &[
            "layout_names_appear_only_in_the_layout_module",
            "the_layout_guard_rejects_a_name_outside_the_layout_module",
        ],
    ),
    (
        "9: the Model's guards cover the new modules (who builds the types, bulk data, closed or non-exhaustive enums, no layout name)",
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
];

/// The build that criteria 4 and 5 are checked on, **made by hand through the facade so that no file is in it and no layout word**: five layers, nine declared; layer 1 ends before it starts and layer 2
/// starts before it ended; laser `L1` is recorded in layers 1 to 4 with channel `A` whole, channel `B` short in layer 1 (and recorded there only) and a channel `C` that the version does not read in layer
/// 2; laser `L2` cannot be read in layer 3; and no laser is recorded in layer 5. **Every one of the seven rules fires once on it, and every one of the five states occurs.**
fn a_build_in_which_every_rule_fires() -> InMemorySource {
    let mut build = BuildMeta::new(BuildIdentity {
        build_id: "a build".into(),
        date: None,
        username: None,
        alloy: None,
        part_count: None,
        project_id: None,
        source_mcf_name: None,
        source_mcf_hash: None,
    });
    build.layers_declared = Some(9);
    for (id, times) in [
        (1, Some((10, 5))),
        (2, Some((3, 8))),
        (3, None),
        (4, None),
        (5, None),
    ] {
        let mut layer = LayerMeta::new(LayerId(id));
        if let Some((start, end)) = times {
            layer.start = Some(TimeValue::from_file(start, None));
            layer.end = Some(TimeValue::from_file(end, None));
        }
        build.layers.insert(LayerId(id), layer);
    }
    let channel = |name: &str, recorded: &[(u32, u64)]| {
        let mut presence = ByLayer::new();
        for &(layer, samples) in recorded {
            presence.insert(LayerId(layer), Presence::from_count(samples));
        }
        ChannelDescriptor::new(
            ChannelKey::new([name]),
            ChannelKind::LaserPower,
            ClockClass::LaserSynchronous,
            ChannelDType::Numeric(NumericType::F32),
        )
        .with_presence(presence)
    };
    let mut one = LaserMeta::new(LaserKey::new("L1", 1));
    for layer in 1..=4 {
        one.layers.insert(
            LayerId(layer),
            LaserLayerMeta::new(Presence::from_count(100)),
        );
    }
    one.channels
        .push(channel("A", &[(1, 100), (2, 100), (3, 100), (4, 100)]));
    one.channels.push(channel("B", &[(1, 40)]));
    let not_offered = Error::new(ErrorKind::UnsupportedInVersion {
        version: "17".into(),
        feature: "a column stored as a kind this version does not read".into(),
    });
    one.layers
        .get_mut(LayerId(2))
        .expect("the entry")
        .origins
        .record("channel C", Origin::not_read(&not_offered));
    build.add_laser(one);
    let mut two = LaserMeta::new(LaserKey::new("L2", 2));
    let corrupt = Error::new(ErrorKind::Corrupt {
        detail: "the start of the laser is two numbers".into(),
    });
    let mut entry = LaserLayerMeta::new(Presence::NotRecorded);
    entry.origins.record("laser", Origin::not_read(&corrupt));
    two.layers.insert(LayerId(3), entry);
    build.add_laser(two);
    let mut source = InMemorySource::new();
    source.set_build(build);
    source
}

#[test]
fn ip07_exit_checklist_holds() {
    // the criteria that name tests: each must still be where the plan says
    let mut missing = tests_that_are_gone(CHECKLIST);

    // 9: no `unsafe` in any crate of the workspace and the lint that denies it is still there, and the Model still has no HDF5
    missing.extend(unsafe_in_the_workspace());
    missing.extend(hdf5_in_the_model());

    let source = a_build_in_which_every_rule_fires();

    // 5: every rule fires, once, with the severity it always has, and the counts are of the findings
    let report = source.validate().expect("the findings");
    for (code, severity) in [
        (FindingCode::LayerCountDiffers, Severity::Note),
        (FindingCode::LayerEndsBeforeItStarts, Severity::Warning),
        (FindingCode::LayersOverlap, Severity::Warning),
        (FindingCode::ChannelShorterThanStream, Severity::Warning),
        (FindingCode::NotRead, Severity::Fault),
        (FindingCode::NotSupported, Severity::Note),
        (FindingCode::LayerWithoutLasers, Severity::Note),
    ] {
        let of_code: Vec<_> = report.findings.iter().filter(|f| f.code == code).collect();
        if of_code.len() != 1 {
            missing.push(format!(
                "criterion 5: the rule of {code:?} found {} on a build made to break it, and should find 1",
                of_code.len()
            ));
        }
        if of_code.iter().any(|f| f.severity != severity) || code.severity() != severity {
            missing.push(format!(
                "criterion 5: {code:?} no longer has the severity {severity:?}"
            ));
        }
    }
    let counts = (
        report.counts.notes,
        report.counts.warnings,
        report.counts.faults,
    );
    if counts != (3, 3, 1) || report.findings.len() != 7 {
        missing.push(format!(
            "criterion 5: the counts of the findings are {counts:?} of {} and should be (3, 3, 1) of 7",
            report.findings.len()
        ));
    }

    // 4: every one of the five states occurs, and the item that is at fault is the only one that changes
    let availability = source.availability().expect("the availability");
    let all = availability.counts();
    for (state, count) in [
        ("present", all.present),
        ("not recorded", all.not_recorded),
        ("degraded", all.degraded),
        ("unreadable", all.unreadable),
        ("unsupported", all.unsupported),
    ] {
        if count == 0 {
            missing.push(format!(
                "criterion 4: no item is {state} on a build made to have one of each state"
            ));
        }
    }
    let laser = |key: &str, number| LaserKey::new(key, number);
    let channel = |layer: u32, name: &str| Item::Channel {
        layer: LayerId(layer),
        laser: Some(laser("L1", 1)),
        channel: ChannelKey::new([name]),
    };
    let item_laser = |layer: u32, key: &str, number| Item::Laser {
        layer: LayerId(layer),
        laser: laser(key, number),
    };
    for (what, item, holds) in [
        (
            "a short channel is degraded",
            channel(1, "B"),
            matches!(
                availability.status(&channel(1, "B")),
                Availability::Degraded { .. }
            ),
        ),
        (
            "the whole channel beside it is present",
            channel(1, "A"),
            matches!(availability.status(&channel(1, "A")), Availability::Present),
        ),
        (
            "a channel that is not recorded in a layer where its laser is, is not recorded",
            channel(2, "B"),
            matches!(
                availability.status(&channel(2, "B")),
                Availability::NotRecorded
            ),
        ),
        (
            "a channel of a type that is not offered is unsupported",
            channel(2, "C"),
            matches!(
                availability.status(&channel(2, "C")),
                Availability::Unsupported { .. }
            ),
        ),
        (
            "a laser that cannot be read is unreadable",
            item_laser(3, "L2", 2),
            matches!(
                availability.status(&item_laser(3, "L2", 2)),
                Availability::Unreadable { .. }
            ),
        ),
        (
            "and the other laser in that layer is present",
            item_laser(3, "L1", 1),
            matches!(
                availability.status(&item_laser(3, "L1", 1)),
                Availability::Present
            ),
        ),
    ] {
        if !holds {
            missing.push(format!("criterion 4: {what} ({item:?})"));
        }
    }

    assert!(
        missing.is_empty(),
        "the exit checklist of IP-07 does not hold:\n{}",
        missing.join("\n")
    );
}
