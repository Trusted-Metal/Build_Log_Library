//! The assertions are tested against themselves (IP-02 task T02.21): a CDM built from each expectation conforms, and the same CDM with one thing changed does not.
//!
//! Until adapters exist the synthetic builder is the only producer, so this is also what proves that each assertion can fail.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use build_log_conformance::{check, expectations_dir, load_all, synthesise, Expectation};
use build_log_model::{
    Accuracy, BuildMeta, ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind, ClockClass,
    NumericType, Presence, Quantity, TimeValue, TimeWindow, UnitSource,
};

fn testdata() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("testdata")
}

#[test]
fn assertions_pass_on_a_synthetic_cdm_built_from_each_expectation() {
    let all = load_all().expect("the expectation files can be read");
    // one expectation for each committed miniature, and no other
    let miniatures: BTreeSet<String> = fs::read_dir(testdata())
        .expect("testdata")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".h5"))
        .collect();
    let named: BTreeSet<String> = all.iter().map(|e| e.miniature().to_string()).collect();
    assert_eq!(named, miniatures, "an expectation file for each miniature");
    assert_eq!(all.len(), miniatures.len());
    for expectation in &all {
        let meta =
            synthesise(expectation).unwrap_or_else(|e| panic!("{}: {e}", expectation.miniature()));
        let found = check(&meta, expectation);
        assert!(
            found.is_empty(),
            "{} does not conform to its own expectation:\n  {}",
            expectation.miniature(),
            found.join("\n  ")
        );
    }
}

type Mutation = (&'static str, fn(&mut BuildMeta) -> bool);

/// Replaces laser `index` by an edited copy; `false` (and nothing changed) if there is no such laser or the edit did not apply.
fn edit_laser(
    meta: &mut BuildMeta,
    index: usize,
    edit: impl FnOnce(&mut build_log_model::LaserMeta) -> bool,
) -> bool {
    let Some(original) = meta.lasers().get(index).cloned() else {
        return false;
    };
    let mut laser = original;
    if !edit(&mut laser) {
        return false;
    }
    meta.add_laser(laser);
    true
}

/// The same build without its last laser.
fn without_last_laser(meta: &BuildMeta) -> Option<BuildMeta> {
    let (last, rest) = meta.lasers().split_last()?;
    let _ = last;
    let mut copy = BuildMeta::new(meta.identity.clone());
    copy.layers_declared = meta.layers_declared;
    copy.layer_thickness = meta.layer_thickness.clone();
    copy.layers = meta.layers.clone();
    copy.sensors = meta.sensors.clone();
    copy.origins = meta.origins.clone();
    for laser in rest {
        copy.add_laser(laser.clone());
    }
    Some(copy)
}

fn mutations() -> Vec<Mutation> {
    vec![
        ("the build id changes", |m| {
            m.identity.build_id.push('x');
            true
        }),
        ("the build date is normalised", |m| {
            match m.identity.date.as_mut() {
                Some(date) => {
                    date.push('Z');
                    true
                }
                None => false,
            }
        }),
        (
            "the declared layer count is replaced by the count of layers present",
            |m| {
                let present = m.layers_present() as i64;
                let changed = m.layers_declared != Some(present);
                m.layers_declared = Some(present);
                changed
            },
        ),
        ("the declared layer count goes", |m| {
            m.layers_declared.take().is_some()
        }),
        ("the build's layer thickness is dropped", |m| {
            m.layer_thickness.take().is_some()
        }),
        ("a layer disappears", |m| {
            let Some(first) = m.layers.ids().next() else {
                return false;
            };
            m.layers = m
                .layers
                .iter()
                .filter(|(id, _)| *id != first)
                .map(|(id, l)| (id, l.clone()))
                .collect();
            true
        }),
        ("a layer start is converted to another unit", |m| {
            let ids: Vec<_> = m.layers.ids().collect();
            ids.into_iter().any(|id| {
                let layer = m.layers.get_mut(id).expect("a layer");
                match layer.start.as_ref() {
                    Some(start) => {
                        layer.start = Some(TimeValue::from_file(start.value * 1000, None));
                        true
                    }
                    None => false,
                }
            })
        }),
        ("a layer time is said to come from the file", |m| {
            let ids: Vec<_> = m.layers.ids().collect();
            ids.into_iter().any(|id| {
                let layer = m.layers.get_mut(id).expect("a layer");
                match layer.end.as_mut() {
                    Some(end) => {
                        end.unit_source = UnitSource::FromFile;
                        true
                    }
                    None => false,
                }
            })
        }),
        ("a placeholder thickness is read as a number", |m| {
            let ids: Vec<_> = m.layers.ids().collect();
            ids.into_iter().any(|id| {
                let layer = m.layers.get_mut(id).expect("a layer");
                if layer.origins.get("thickness").is_some() {
                    layer.thickness = Some(Quantity::from_file(0.0, None));
                    true
                } else {
                    false
                }
            })
        }),
        ("a laser disappears", |m| match without_last_laser(m) {
            Some(copy) => {
                *m = copy;
                true
            }
            None => false,
        }),
        ("the samples of a laser in a layer change", |m| {
            edit_laser(m, 0, |laser| {
                let Some(id) = laser.layers.ids().next() else {
                    return false;
                };
                let layer = laser.layers.get_mut(id).expect("a layer");
                let samples = layer.presence.samples().unwrap_or(0);
                layer.presence = Presence::from_count(samples + 1);
                true
            })
        }),
        ("a recorded empty stream is said to be not recorded", |m| {
            (0..m.lasers().len()).any(|i| {
                edit_laser(m, i, |laser| {
                    let empty = laser
                        .layers
                        .iter()
                        .find(|(_, l)| l.presence == Presence::RecordedEmpty)
                        .map(|(id, _)| id);
                    let Some(empty) = empty else { return false };
                    laser.layers = laser
                        .layers
                        .iter()
                        .filter(|(id, _)| *id != empty)
                        .map(|(id, l)| (id, l.clone()))
                        .collect();
                    true
                })
            })
        }),
        ("the accuracy of a time axis changes", |m| {
            edit_laser(m, 0, |laser| {
                let Some(id) = laser.layers.ids().next() else {
                    return false;
                };
                let axis = &mut laser.layers.get_mut(id).expect("a layer").time_axis;
                axis.accuracy = if axis.accuracy == Accuracy::Measured {
                    Accuracy::Unknown
                } else {
                    Accuracy::Measured
                };
                true
            })
        }),
        ("the window of a stream is converted to another unit", |m| {
            (0..m.lasers().len()).any(|i| {
                edit_laser(m, i, |laser| {
                    let ids: Vec<_> = laser.layers.ids().collect();
                    ids.into_iter().any(|id| {
                        let axis = &mut laser.layers.get_mut(id).expect("a layer").time_axis;
                        match axis.window.take() {
                            Some(w) => {
                                axis.window = Some(TimeWindow {
                                    start: TimeValue::from_file(w.start.value * 1000, None),
                                    end: w.end,
                                });
                                true
                            }
                            None => false,
                        }
                    })
                })
            })
        }),
        ("the sample period is dropped", |m| {
            (0..m.lasers().len()).any(|i| {
                edit_laser(m, i, |laser| {
                    let ids: Vec<_> = laser.layers.ids().collect();
                    ids.into_iter().any(|id| {
                        laser
                            .layers
                            .get_mut(id)
                            .expect("a layer")
                            .time_axis
                            .period
                            .take()
                            .is_some()
                    })
                })
            })
        }),
        ("a channel disappears", |m| {
            edit_laser(m, 0, |laser| {
                if laser.channels.is_empty() {
                    return false;
                }
                laser.channels.remove(0);
                true
            })
        }),
        ("a channel appears that nobody expects", |m| {
            edit_laser(m, 0, |laser| {
                laser.channels.push(ChannelDescriptor::new(
                    ChannelKey::new(["extra"]),
                    ChannelKind::Sensor,
                    ClockClass::Timestamped,
                    ChannelDType::Numeric(NumericType::F32),
                ));
                true
            })
        }),
        ("a declared unit is said to come from the file", |m| {
            edit_laser(m, 0, |laser| {
                match laser
                    .channels
                    .iter_mut()
                    .find(|c| c.unit_source == UnitSource::DeclaredByAdapter)
                {
                    Some(channel) => {
                        channel.unit_source = UnitSource::FromFile;
                        true
                    }
                    None => false,
                }
            })
        }),
        ("a channel loses its unit", |m| {
            edit_laser(m, 0, |laser| {
                match laser.channels.iter_mut().find(|c| c.unit.is_known()) {
                    Some(channel) => {
                        channel.unit = build_log_model::Unit::unknown();
                        channel.unit_source = UnitSource::Unknown;
                        true
                    }
                    None => false,
                }
            })
        }),
        ("a channel is stored as another type", |m| {
            edit_laser(m, 0, |laser| match laser.channels.first_mut() {
                Some(channel) => {
                    channel.dtype = ChannelDType::Text;
                    true
                }
                None => false,
            })
        }),
        ("a channel is recorded in another layer than it was", |m| {
            edit_laser(m, 0, |laser| match laser.channels.first_mut() {
                Some(channel) => {
                    channel
                        .presence
                        .insert(build_log_model::LayerId(9_999), Presence::from_count(1));
                    true
                }
                None => false,
            })
        }),
        ("a sensor disappears", |m| m.sensors.pop().is_some()),
        ("a sensor changes its clock class", |m| {
            match m.sensors.first_mut() {
                Some(sensor) => {
                    sensor.clock_class = if sensor.clock_class == ClockClass::Image {
                        ClockClass::Static
                    } else {
                        ClockClass::Image
                    };
                    true
                }
                None => false,
            }
        }),
    ]
}

#[test]
fn each_assertion_fails_on_a_mutated_cdm() {
    let all = load_all().expect("the expectation files can be read");
    for (name, mutate) in mutations() {
        let mut applied = 0;
        for expectation in &all {
            let mut meta = synthesise(expectation).expect("a synthetic CDM");
            if !mutate(&mut meta) {
                continue;
            }
            applied += 1;
            assert!(
                !check(&meta, expectation).is_empty(),
                "the assertions accepted {} after: {name}",
                expectation.miniature()
            );
        }
        assert!(
            applied > 0,
            "the mutation `{name}` applied to none of the expectation files, so it proves nothing"
        );
    }
}

#[test]
fn the_check_lists_every_disagreement_and_says_what_it_found_and_expected() {
    let all = load_all().expect("the expectation files can be read");
    let expectation = all
        .iter()
        .find(|e| e.miniature() == "v17_mock.h5")
        .expect("the mock capture");
    let mut meta = synthesise(expectation).unwrap();
    meta.identity.build_id.push('x');
    meta.layers_declared = Some(99);
    edit_laser(&mut meta, 0, |laser| {
        laser.channels.remove(0);
        true
    });
    let found = check(&meta, expectation);
    assert!(found.len() >= 3, "{found:?}");
    assert!(
        found
            .iter()
            .any(|f| f.starts_with("the build id: found") && f.contains("expected")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|f| f.contains("the declared layer count: found Some(99), expected Some(2)")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|f| f.contains("not described")),
        "{found:?}"
    );
}

#[test]
fn an_expectation_file_of_another_version_or_that_is_not_json_is_refused() {
    let refused = Expectation::parse("{\"expectations_version\": 2}").unwrap_err();
    assert!(refused.contains("expectations_version"), "{refused}");
    assert!(Expectation::parse("not json").is_err());
    assert!(Expectation::parse("{}").is_err());
    assert!(
        expectations_dir().is_dir(),
        "{}",
        expectations_dir().display()
    );
}
