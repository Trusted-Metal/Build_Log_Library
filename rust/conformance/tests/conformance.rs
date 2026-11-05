//! The assertions are tested against themselves (IP-02 task T02.21): a CDM built from each expectation conforms, and the same CDM with one thing changed does not.
//!
//! Until adapters exist the synthetic builder is the only producer, so this is also what proves that each assertion can fail.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use build_log_conformance::{
    check, check_parts, expectations_dir, load_all, synthesise, Expectation, Part,
};
use build_log_model::{
    Accuracy, BuildMeta, ChannelDType, ChannelDescriptor, ChannelKey, ChannelKind, ClockClass,
    EquationKind, EventDataType, EventKind, EventPart, EventSeries, GalvoProtocol, ImageFormat,
    JoinStatus, LaserParameters, LayerId, NumericType, Origin, Outcome, Presence, Quantity,
    TimeValue, TimeWindow, UnitSource,
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
    copy.quality = meta.quality.clone();
    copy.provenance = meta.provenance.clone();
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
        ("a placeholder actual thickness is read as a number", |m| {
            let ids: Vec<_> = m.layers.ids().collect();
            ids.into_iter().any(|id| {
                let layer = m.layers.get_mut(id).expect("a layer");
                if layer.origins.get("actual_thickness").is_some() {
                    layer.actual_thickness = Some(Quantity::from_file(0.0, None));
                    true
                } else {
                    false
                }
            })
        }),
        (
            "a placeholder commanded thickness is read as a number",
            |m| {
                let ids: Vec<_> = m.layers.ids().collect();
                ids.into_iter().any(|id| {
                    let layer = m.layers.get_mut(id).expect("a layer");
                    if layer.origins.get("commanded_thickness").is_some() {
                        layer.commanded_thickness = Some(Quantity::from_file(0.0, None));
                        true
                    } else {
                        false
                    }
                })
            },
        ),
        (
            "the commanded thickness is reported as the actual one",
            |m| {
                // the two are separate facts: the value of one must not stand in for the other
                let ids: Vec<_> = m.layers.ids().collect();
                ids.into_iter().any(|id| {
                    let layer = m.layers.get_mut(id).expect("a layer");
                    if layer.commanded_thickness.is_none()
                        && layer.origins.get("commanded_thickness").is_some()
                    {
                        layer.commanded_thickness = Some(Quantity::from_file(0.03, None));
                        layer.origins = build_log_model::Origins::new();
                        true
                    } else {
                        false
                    }
                })
            },
        ),
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
        ("the build's outcome is lost", |m| {
            let had = m.quality.outcome != Outcome::NotRecorded;
            m.quality.outcome = Outcome::NotRecorded;
            had
        }),
        ("an outcome the file does not have is invented", |m| {
            let had_none = m.quality.outcome == Outcome::NotRecorded;
            if had_none {
                m.quality.outcome = Outcome::Complete;
            }
            had_none
        }),
        ("a status is read as another outcome", |m| {
            let changed = m.quality.outcome != Outcome::NotRecorded;
            if changed {
                m.quality.outcome = Outcome::InProgress;
            }
            changed && m.quality.outcome != Outcome::Complete
        }),
        ("the status of the OPC UA join is lost", |m| {
            m.provenance.opcua_join_status.take().is_some()
        }),
        (
            "the status of the OPC UA join is read as another one",
            |m| {
                if m.provenance.opcua_join_status.is_some() {
                    m.provenance.opcua_join_status = Some(JoinStatus::Other("Partial".into()));
                    true
                } else {
                    false
                }
            },
        ),
        (
            "the version of the OPC UA join is read as a number",
            |m| match m.provenance.opcua_join_version.as_mut() {
                Some(version) => {
                    version.push_str(".0");
                    true
                }
                None => false,
            },
        ),
        ("a laser's complete flag is lost", |m| {
            edit_laser(m, 0, |laser| {
                let mut changed = false;
                let ids: Vec<_> = laser.layers.ids().collect();
                for id in ids {
                    let layer = laser.layers.get_mut(id).expect("a layer");
                    changed |= layer.complete.take().is_some();
                }
                changed
            })
        }),
        ("a laser's complete flag is turned over", |m| {
            edit_laser(m, 1, |laser| {
                let mut changed = false;
                let ids: Vec<_> = laser.layers.ids().collect();
                for id in ids {
                    let layer = laser.layers.get_mut(id).expect("a layer");
                    if let Some(flag) = layer.complete {
                        layer.complete = Some(!flag);
                        changed = true;
                    }
                }
                changed
            })
        }),
        ("a complete flag the file does not have is invented", |m| {
            edit_laser(m, 0, |laser| {
                let mut changed = false;
                let ids: Vec<_> = laser.layers.ids().collect();
                for id in ids {
                    let layer = laser.layers.get_mut(id).expect("a layer");
                    if layer.complete.is_none() {
                        layer.complete = Some(true);
                        changed = true;
                    }
                }
                changed
            })
        }),
        ("a laser's parameters are lost", |m| {
            edit_laser(m, 0, |laser| laser.parameters.take().is_some())
        }),
        ("parameters are invented for a laser that has none", |m| {
            edit_laser(m, 0, |laser| {
                let none = laser.parameters.is_none();
                if none {
                    laser.parameters = Some(LaserParameters::new());
                }
                none
            })
        }),
        ("the optical train of a laser is another one", |m| {
            edit_laser(m, 0, |laser| match laser.parameters.as_mut() {
                Some(parameters) => match parameters.optical_train_number {
                    Some(n) => {
                        parameters.optical_train_number = Some(n + 1);
                        true
                    }
                    None => false,
                },
                None => false,
            })
        }),
        (
            "a laser is given an optical train that the file does not have",
            |m| {
                edit_laser(m, 0, |laser| match laser.parameters.as_mut() {
                    Some(parameters) if parameters.optical_train_number.is_none() => {
                        parameters.optical_train_number = Some(1);
                        true
                    }
                    _ => false,
                })
            },
        ),
        ("the sign of a scan-head offset is flipped", |m| {
            edit_laser(m, 0, |laser| {
                let Some(q) = laser
                    .parameters
                    .as_mut()
                    .and_then(|p| p.scanner.offset_x.as_mut())
                else {
                    return false;
                };
                q.value = -q.value;
                q.value != 0.0
            })
        }),
        ("the note on a value is lost", |m| {
            edit_laser(m, 0, |laser| {
                let Some(p) = laser.parameters.as_mut() else {
                    return false;
                };
                if p.origins.get("scanner.offset_x").is_none() {
                    return false;
                }
                let kept: Vec<_> = p
                    .origins
                    .iter()
                    .filter(|(name, _)| *name != "scanner.offset_x")
                    .map(|(name, origin)| (name.to_string(), origin.clone()))
                    .collect();
                p.origins = build_log_model::Origins::new();
                for (name, origin) in kept {
                    p.origins.record(name, origin);
                }
                true
            })
        }),
        ("the note on a value says something else", |m| {
            edit_laser(m, 0, |laser| {
                let Some(p) = laser.parameters.as_mut() else {
                    return false;
                };
                if p.origins.get("scanner.offset_x").is_none() {
                    return false;
                }
                p.origins.record(
                    "scanner.offset_x",
                    Origin::Other("scan-head offset: flipped".into()),
                );
                true
            })
        }),
        ("a note is put on a value that has none", |m| {
            edit_laser(m, 0, |laser| {
                let Some(p) = laser.parameters.as_mut() else {
                    return false;
                };
                if p.origins.get("scanner.offset_x").is_none() {
                    return false;
                }
                p.origins.record(
                    "scanner.field_size_x",
                    Origin::Other("a note that was not expected".into()),
                );
                true
            })
        }),
        ("a galvo's bit resolution is another number", |m| {
            edit_laser(m, 1, |laser| {
                let Some(p) = laser.parameters.as_mut() else {
                    return false;
                };
                match p.galvo_y.actual_bit_resolution {
                    Some(n) => {
                        p.galvo_y.actual_bit_resolution = Some(n + 1);
                        true
                    }
                    None => false,
                }
            })
        }),
        (
            "a galvo's bit resolution is not given to every galvo",
            |m| {
                edit_laser(m, 0, |laser| {
                    laser
                        .parameters
                        .as_mut()
                        .is_some_and(|p| p.galvo_z.commanded_bit_resolution.take().is_some())
                })
            },
        ),
        ("a value of the snapshot changes", |m| {
            edit_laser(m, 1, |laser| {
                let Some(q) = laser
                    .parameters
                    .as_mut()
                    .and_then(|p| p.scanner.field_size_x.as_mut())
                else {
                    return false;
                };
                q.value += 1.0;
                true
            })
        }),
        ("a unit is guessed where the file states none", |m| {
            edit_laser(m, 0, |laser| {
                let Some(q) = laser
                    .parameters
                    .as_mut()
                    .and_then(|p| p.scanner.rotation.as_ref())
                else {
                    return false;
                };
                let guessed = Quantity::from_file(q.value, Some("deg"));
                laser.parameters.as_mut().unwrap().scanner.rotation = Some(guessed);
                true
            })
        }),
        ("a value of the snapshot is dropped", |m| {
            edit_laser(m, 0, |laser| {
                laser
                    .parameters
                    .as_mut()
                    .is_some_and(|p| p.scanner.model.take().is_some())
            })
        }),
        ("a value that the file does not have is invented", |m| {
            edit_laser(m, 0, |laser| {
                let Some(p) = laser.parameters.as_mut() else {
                    return false;
                };
                let none = p.light_source.power_min_actual.is_none();
                if none {
                    p.light_source.power_min_actual = Some(Quantity::from_file(100.0, None));
                }
                none
            })
        }),
        ("a galvo's protocol is read as another one", |m| {
            edit_laser(m, 0, |laser| {
                let Some(p) = laser.parameters.as_mut() else {
                    return false;
                };
                let had = p.galvo_x.protocol.is_some();
                if had {
                    p.galvo_x.protocol = Some(GalvoProtocol::Other("SL2-100".into()));
                }
                had
            })
        }),
        ("the constants of the equation are given other names", |m| {
            edit_laser(m, 0, |laser| {
                let Some(equation) = laser
                    .parameters
                    .as_mut()
                    .and_then(|p| p.light_source.volts_to_watts.as_mut())
                else {
                    return false;
                };
                // the same numbers, the other way round
                for constant in &mut equation.constants {
                    constant.name = format!("{}x", constant.name);
                }
                true
            })
        }),
        ("the equation is read as another kind", |m| {
            edit_laser(m, 0, |laser| {
                let Some(equation) = laser
                    .parameters
                    .as_mut()
                    .and_then(|p| p.light_source.volts_to_watts.as_mut())
                else {
                    return false;
                };
                equation.kind = EquationKind::Other("LINEAR".into());
                true
            })
        }),
        ("the equation is lost", |m| {
            edit_laser(m, 1, |laser| {
                laser
                    .parameters
                    .as_mut()
                    .is_some_and(|p| p.light_source.volts_to_watts.take().is_some())
            })
        }),
        (
            "text that stands for a number is decoded as the number",
            |m| {
                edit_laser(m, 0, |laser| {
                    let Some(p) = laser.parameters.as_mut() else {
                        return false;
                    };
                    let had = p.origins.get("light_source.power_limits").is_some();
                    if had {
                        p.origins = build_log_model::Origins::new();
                        p.light_source.power_min_actual = Some(Quantity::from_file(100.6, None));
                    }
                    had
                })
            },
        ),
        ("the dimensions of an image change", |m| {
            m.sensors
                .iter_mut()
                .any(|channel| match &mut channel.dtype {
                    ChannelDType::Image(format) => {
                        *format = ImageFormat::new(format.rows + 1, format.columns, format.pixel);
                        true
                    }
                    _ => false,
                })
        }),
        ("the pixel type of an image changes", |m| {
            m.sensors
                .iter_mut()
                .any(|channel| match &mut channel.dtype {
                    ChannelDType::Image(format) => {
                        *format = ImageFormat::new(format.rows, format.columns, NumericType::U16);
                        true
                    }
                    _ => false,
                })
        }),
        ("an image channel is read as a plain sensor", |m| {
            m.sensors.iter_mut().any(|channel| {
                let was = matches!(channel.dtype, ChannelDType::Image(_));
                if was {
                    channel.kind = ChannelKind::Sensor;
                    channel.dtype = ChannelDType::Numeric(NumericType::F64);
                }
                was
            })
        }),
        ("an image channel is lost", |m| {
            let before = m.sensors.len();
            m.sensors
                .retain(|c| !matches!(c.dtype, ChannelDType::Image(_)));
            m.sensors.len() != before
        }),
        ("an image is lost in one layer", |m| {
            m.sensors.iter_mut().any(|channel| {
                if !matches!(channel.dtype, ChannelDType::Image(_)) {
                    return false;
                }
                let Some(id) = channel.presence.ids().next() else {
                    return false;
                };
                channel.presence = channel
                    .presence
                    .iter()
                    .filter(|(layer, _)| *layer != id)
                    .map(|(layer, p)| (layer, *p))
                    .collect();
                true
            })
        }),
        ("an event series is lost", |m| {
            if m.events.is_empty() {
                return false;
            }
            m.events.remove(0);
            true
        }),
        ("an event series is invented", |m| {
            m.events.push(EventSeries::new(
                EventKind::Sensor,
                vec!["not".into(), "there".into()],
            ));
            true
        }),
        ("the records of an event in one place change", |m| {
            m.events.iter_mut().any(|series| {
                if let Some(part) = series.no_layer.as_mut() {
                    part.records += 1;
                    return true;
                }
                let first = series.layers.ids().next();
                match first {
                    Some(id) => {
                        series.layers.get_mut(id).expect("a place").records += 1;
                        true
                    }
                    None => false,
                }
            })
        }),
        ("the type an event declares changes", |m| {
            m.events.iter_mut().any(|series| match &series.data_type {
                Some(_) => {
                    series.data_type = Some(EventDataType::Other("Float".into()));
                    true
                }
                None => false,
            })
        }),
        ("a type is declared for an event that declares none", |m| {
            m.events.iter_mut().any(|series| {
                let none = series.data_type.is_none();
                if none {
                    series.data_type = Some(EventDataType::Double);
                }
                none
            })
        }),
        ("an event is read as another kind", |m| {
            m.events.iter_mut().any(|series| {
                let was = series.kind == EventKind::Sensor;
                if was {
                    series.kind = EventKind::Task;
                }
                was
            })
        }),
        ("a task is read as having no interval", |m| {
            m.events.iter_mut().any(|series| {
                let was = series.interval;
                series.interval = false;
                was
            })
        }),
        ("the bucket that no layer owns is read as a layer", |m| {
            m.events
                .iter_mut()
                .any(|series| match series.no_layer.take() {
                    Some(part) => {
                        series.layers.insert(LayerId(0), part);
                        true
                    }
                    None => false,
                })
        }),
        ("the height of a task is lost", |m| {
            m.events.iter_mut().any(|series| {
                let ids: Vec<LayerId> = series.layers.ids().collect();
                let mut lost = false;
                for id in ids {
                    let part = series.layers.get_mut(id).expect("a place");
                    lost |= part.height.take().is_some();
                }
                lost
            })
        }),
        ("an event is lost in one layer", |m| {
            m.events.iter_mut().any(|series| {
                let Some(id) = series.layers.ids().next() else {
                    return false;
                };
                series.layers = series
                    .layers
                    .iter()
                    .filter(|(layer, _)| *layer != id)
                    .map(|(layer, part)| (layer, part.clone()))
                    .collect();
                true
            })
        }),
    ]
}

#[test]
fn the_synthetic_cdm_with_images_passes_and_a_changed_one_fails() {
    let all = load_all().expect("the expectation files can be read");
    let images = all
        .iter()
        .find(|e| e.miniature() == "v17_2026_08_18_images.h5")
        .expect("the images file");
    let meta = synthesise(images).expect("a synthetic CDM");
    // two camera channels, one for each phase, each with the frame the expectation names
    let formats: Vec<_> = meta
        .sensors
        .iter()
        .filter_map(|c| match &c.dtype {
            ChannelDType::Image(format) => Some(format.clone()),
            _ => None,
        })
        .collect();
    let wanted = ImageFormat::new(128, 128, NumericType::U8);
    assert_eq!(formats, [wanted.clone(), wanted]);
    assert!(check(&meta, images).is_empty());
    // the same CDM with a frame one row taller does not pass
    let mut taller = meta.clone();
    taller.sensors[0].dtype = ChannelDType::Image(ImageFormat::new(129, 128, NumericType::U8));
    assert!(!check(&taller, images).is_empty());
    // and the other files hold no image at all
    for expectation in all
        .iter()
        .filter(|e| e.miniature() != "v17_2026_08_18_images.h5")
    {
        let other = synthesise(expectation).expect("a synthetic CDM");
        assert!(
            other
                .sensors
                .iter()
                .all(|c| !matches!(c.dtype, ChannelDType::Image(_))),
            "{}",
            expectation.miniature()
        );
    }
}

#[test]
fn the_synthetic_cdm_with_events_passes_and_a_changed_one_fails() {
    let all = load_all().expect("the expectation files can be read");
    let images = all
        .iter()
        .find(|e| e.miniature() == "v17_2026_08_18_images.h5")
        .expect("the file with events at the top and inside each layer");
    let meta = synthesise(images).expect("a synthetic CDM");
    assert_eq!(meta.events.len(), 40);
    // a series recorded in the bucket that no layer owns and in each layer, a task with an interval and a height, and the run state
    let velocity = meta
        .events
        .iter()
        .find(|s| s.path.last().map(String::as_str) == Some("current_velocity"))
        .expect("a sensor series");
    assert!(velocity.no_layer.is_some() && velocity.layers.len() == 3);
    assert!(meta
        .events
        .iter()
        .any(|s| s.kind == EventKind::Task && s.interval));
    assert!(meta.events.iter().any(|s| s.kind == EventKind::Run));
    assert!(check(&meta, images).is_empty());
    // the same CDM with one record more in one place does not pass
    let mut changed = meta.clone();
    changed.events[0]
        .no_layer
        .get_or_insert_with(|| EventPart::new(0))
        .records += 1;
    assert!(!check(&changed, images).is_empty());
    // the files that have no event say so
    for expectation in all.iter().filter(|e| {
        !["v17_2026_08_18_images.h5", "v17_triggered_2026_10_19.h5"].contains(&e.miniature())
    }) {
        assert!(synthesise(expectation).expect("a CDM").events.is_empty());
    }
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
fn the_parts_together_equal_the_whole() {
    // an adapter built in steps asserts the parts it implements (IP-13, D13-9), so the parts must add up to the whole: on a CDM that is right, and on
    // every mutated CDM of the list above, the lines of `check` are the lines of the parts one after the other, and no line belongs to two parts
    let all = load_all().expect("the expectation files can be read");
    let mut mutated = 0;
    for expectation in &all {
        let right = synthesise(expectation).expect("a synthetic CDM");
        for part in Part::ALL {
            assert!(
                check_parts(&right, expectation, &[part]).is_empty(),
                "{} fails {part:?} on a CDM built from it",
                expectation.miniature()
            );
        }
        assert!(check_parts(&right, expectation, &[]).is_empty());
        for (name, mutate) in mutations() {
            let mut meta = synthesise(expectation).expect("a synthetic CDM");
            if !mutate(&mut meta) {
                continue;
            }
            mutated += 1;
            let whole = check(&meta, expectation);
            let in_parts: Vec<String> = Part::ALL
                .iter()
                .flat_map(|part| check_parts(&meta, expectation, &[*part]))
                .collect();
            assert_eq!(
                whole,
                in_parts,
                "{} after {name}: the parts do not add up to the whole",
                expectation.miniature()
            );
            // the order in which the parts are named does not change the order of the lines
            assert_eq!(
                check_parts(
                    &meta,
                    expectation,
                    &[
                        Part::Events,
                        Part::Sensors,
                        Part::Parameters,
                        Part::Lasers,
                        Part::Layers,
                        Part::Context,
                        Part::Build
                    ]
                ),
                whole
            );
        }
    }
    assert!(mutated > 0, "no mutation applied, so nothing was compared");
}

#[test]
fn a_part_reports_only_its_own_disagreements() {
    let all = load_all().expect("the expectation files can be read");
    let expectation = all
        .iter()
        .find(|e| e.miniature() == "v17_mock.h5")
        .expect("the mock capture");
    let mut meta = synthesise(expectation).unwrap();
    meta.identity.build_id.push('x'); // a fault of the build level
    edit_laser(&mut meta, 0, |laser| {
        laser.channels.remove(0); // and one of the lasers
        true
    });
    let build = check_parts(&meta, expectation, &[Part::Build]);
    let lasers = check_parts(&meta, expectation, &[Part::Lasers]);
    assert!(
        build.iter().any(|l| l.starts_with("the build id")) && build.len() == 1,
        "{build:?}"
    );
    assert!(
        !lasers.is_empty() && lasers.iter().all(|l| !l.starts_with("the build id")),
        "{lasers:?}"
    );
    assert!(check_parts(&meta, expectation, &[Part::Layers]).is_empty());
    assert!(check_parts(&meta, expectation, &[Part::Sensors]).is_empty());
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
