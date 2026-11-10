//! The fingerprints of stored columns and the check of a column source against them (IP-06 phase 6A, task T06.1, decision D06-16).
//!
//! The expectation files of the six V17 miniatures carry a fingerprint for every laser channel recorded with samples. Until an adapter serves those columns (IP-13 phase
//! 13L) the check is proved on a source made of synthetic columns: a source that serves the columns as stored passes, and each way of serving them wrongly is reported,
//! on that column and on no other.

use build_log_conformance::{
    check_columns, column_expectations, load_all, ColumnExpectation, Expectation, Fingerprint,
};
use build_log_model::{
    ChannelKey, ColumnData, ColumnSource, InMemorySource, LaserKey, LayerId, Shared,
};
use serde_json::{json, Value};

// ---------------------------------------------------------------- a synthetic build: two lasers, two layers, three channels

const LAYERS: [u32; 2] = [1, 2];
const LASERS: [u32; 2] = [1, 2];
const CHANNELS: [&str; 3] = ["Laser_Power", "Laser_State", "Actual_X"];

fn laser(number: u32) -> LaserKey {
    LaserKey::new(format!("Laser_{number}"), number)
}

/// The column of a laser, a layer and a channel: different for each, 1,000 values and more, and with a spread that a changed value can hide in.
fn column(laser: u32, layer: u32, channel: &str) -> ColumnData {
    let length = 1_000 + 37 * layer as usize + 11 * laser as usize;
    let seed = (laser * 131 + layer * 17) as usize;
    match channel {
        "Laser_Power" => ColumnData::F32(Shared::new(
            (0..length)
                .map(|i| 0.25 * (((i * 7 + seed) % 400) as f32))
                .collect(),
        )),
        "Laser_State" => ColumnData::U8(Shared::new(
            (0..length).map(|i| ((i / 5 + seed) % 2) as u8).collect(),
        )),
        _ => ColumnData::I32(Shared::new(
            (0..length)
                .map(|i| ((i * 7919 + seed * 31) % 100_000) as i32 - 50_000)
                .collect(),
        )),
    }
}

/// The source that serves every column as stored; `change` may serve one differently, or not at all (`None`).
fn source_with(
    change: impl Fn(u32, u32, &str, ColumnData) -> Option<ColumnData>,
) -> InMemorySource {
    let mut source = InMemorySource::new();
    for number in LASERS {
        for layer in LAYERS {
            for channel in CHANNELS {
                if let Some(data) = change(number, layer, channel, column(number, layer, channel)) {
                    source.insert(
                        LayerId(layer),
                        Some(laser(number)),
                        ChannelKey::new([channel]),
                        data,
                    );
                }
            }
        }
    }
    source
}

/// An expectation that describes the columns as stored, with the fingerprints of the synthetic columns; the version says whether each must be described.
fn expectation(version: i64, described: bool) -> Expectation {
    let lasers: Vec<Value> = LASERS
        .iter()
        .map(|&number| {
            let channels: Vec<Value> = CHANNELS
                .iter()
                .map(|&name| {
                    let layers: Vec<Value> = LAYERS
                        .iter()
                        .map(|&layer| {
                            let data = column(number, layer, name);
                            let mut entry = json!({
                                "layer": layer,
                                "state": "recorded",
                                "samples": data.len(),
                            });
                            if described {
                                entry["column"] =
                                    Fingerprint::of(&data).expect("numbers").to_json();
                            }
                            entry
                        })
                        .collect();
                    json!({ "source_name": name, "layers": layers })
                })
                .collect();
            json!({ "key": format!("Laser_{number}"), "number": number, "channels": channels })
        })
        .collect();
    let text = json!({
        "expectations_version": 1,
        "miniature": { "file": "synthetic.h5" },
        "file_version": version,
        "lasers": lasers,
    })
    .to_string();
    Expectation::parse(&text).expect("an expectation")
}

fn found(source: &dyn ColumnSource) -> Vec<String> {
    check_columns(source, &expectation(17, true))
}

#[test]
fn a_source_that_serves_the_columns_as_stored_passes() {
    let source = source_with(|_, _, _, data| Some(data));
    let lines = found(&source);
    assert!(lines.is_empty(), "{}", lines.join("\n"));
    // and the check had something to check: two lasers, two layers, three channels
    let described = column_expectations(&expectation(17, true)).expect("the columns");
    assert_eq!(described.len(), 12);
    assert!(described
        .iter()
        .any(|c: &ColumnExpectation| c.laser.key == "Laser_2"
            && c.layer == LayerId(2)
            && c.channel.name() == Some("Actual_X")));
}

/// A way of serving the column of laser 2, layer 2 and `Actual_X` wrongly, and a word that the line about it must contain.
type Serve = fn(&ColumnData) -> Option<ColumnData>;

fn i32s(data: &ColumnData) -> Vec<i32> {
    data.as_i32().expect("an i32 column").to_vec()
}

fn mutations() -> Vec<(&'static str, &'static str, Serve)> {
    vec![
        // the values of the column, in the same count and the same stored type
        (
            "a value in the middle is changed and no extreme or end moves",
            "SHA-256",
            |d| {
                let mut v = i32s(d);
                let middle = v.len() / 2;
                // a value that is neither the minimum nor the maximum goes to another value that is between them
                v[middle] = if v[middle] == 7 { 8 } else { 7 };
                Some(ColumnData::I32(Shared::new(v)))
            },
        ),
        ("the last value is changed", "last values", |d| {
            let mut v = i32s(d);
            *v.last_mut().expect("a value") += 1;
            Some(ColumnData::I32(Shared::new(v)))
        }),
        ("the largest value is raised", "maximum", |d| {
            let mut v = i32s(d);
            let at = v
                .iter()
                .position(|x| x == v.iter().max().expect("a value"))
                .expect("a position");
            v[at] += 1;
            Some(ColumnData::I32(Shared::new(v)))
        }),
        ("the smallest value is lowered", "minimum", |d| {
            let mut v = i32s(d);
            let at = v
                .iter()
                .position(|x| x == v.iter().min().expect("a value"))
                .expect("a position");
            v[at] -= 1;
            Some(ColumnData::I32(Shared::new(v)))
        }),
        ("a value is dropped", "count of values", |d| {
            let mut v = i32s(d);
            v.remove(v.len() / 3);
            Some(ColumnData::I32(Shared::new(v)))
        }),
        ("the range is shifted by one", "first values", |d| {
            let mut v = i32s(d);
            v.rotate_left(1);
            Some(ColumnData::I32(Shared::new(v)))
        }),
        (
            "the same numbers are served as another type",
            "stored type",
            |d| {
                Some(ColumnData::F32(Shared::new(
                    i32s(d).iter().map(|&x| x as f32).collect(),
                )))
            },
        ),
        ("the column is empty", "count of values", |_| {
            Some(ColumnData::I32(Shared::new(Vec::new())))
        }),
        ("the column is not served", "was not read", |_| None),
    ]
}

#[test]
fn a_changed_value_a_dropped_value_a_shifted_range_and_a_wrong_type_each_fail() {
    for (what, word, serve) in mutations() {
        let source = source_with(|number, layer, channel, data| {
            if (number, layer, channel) == (2, 2, "Actual_X") {
                serve(&data)
            } else {
                Some(data)
            }
        });
        let lines = found(&source);
        assert!(!lines.is_empty(), "{what}: the check reported nothing");
        // every line is about that column and no other, and one of them says what is different
        assert!(
            lines
                .iter()
                .all(|l| l.starts_with("layer 2, Laser_2, Actual_X: ")),
            "{what}: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains(word)),
            "{what}: no line says `{word}`: {lines:?}"
        );
    }
}

#[test]
fn a_source_that_serves_nothing_is_reported_for_every_column() {
    let lines = found(&InMemorySource::new());
    assert_eq!(lines.len(), 12);
    assert!(lines.iter().all(|l| l.contains("the column was not read")));
}

#[test]
fn a_column_of_the_wrong_kind_is_not_a_column_of_numbers() {
    let source = source_with(|number, layer, channel, data| {
        if (number, layer, channel) == (1, 1, "Laser_State") {
            Some(ColumnData::Text(Shared::new(vec!["on".to_string()])))
        } else {
            Some(data)
        }
    });
    let lines = found(&source);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("layer 1, Laser_1, Laser_State: ")
            && lines[0].contains("not a column of numbers")
    );
}

// ---------------------------------------------------------------- the fingerprint itself

/// The answers that the oracle's tests hold too, worked out there with `struct` and `hashlib`: the same literals, so that the two languages agree on the bytes.
#[test]
fn the_fingerprint_of_known_columns_equals_the_literals_python_gives() {
    let sha = |data: ColumnData| Fingerprint::of(&data).expect("numbers").sha256;
    assert_eq!(
        sha(ColumnData::U8(Shared::new(vec![1, 2, 3]))),
        "039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81"
    );
    assert_eq!(
        sha(ColumnData::I32(Shared::new(vec![1, -2, 300_000]))),
        "bc7ee6861c985ac4137f1fee179022c42688e8b66c508976e4432ff4fa4ba03a"
    );
    assert_eq!(
        sha(ColumnData::F32(Shared::new(vec![1.5, -0.25, 0.1]))),
        "bdf10fd8bf8c712b3175245ef56daf8d22b826713e16b84b3b218342c0416faa"
    );
    assert_eq!(
        sha(ColumnData::U64(Shared::new(vec![1, u64::MAX]))),
        "db0550d553e2a146e34164d19cd55f006c38d700d8f9a4e3ba2c889a1d7c26b2"
    );
    assert_eq!(
        sha(ColumnData::F64(Shared::new(vec![0.1, 2.5]))),
        "8474c19e6d61138daf4be067368fa4632d5e0e2955dbe391408b33b729f3bfbe"
    );
    assert_eq!(
        sha(ColumnData::I32(Shared::new(Vec::new()))),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    // the rest of what a fingerprint says, written out
    assert_eq!(
        Fingerprint::of(&ColumnData::U8(Shared::new(vec![1, 2, 3]))),
        Some(Fingerprint {
            count: 3,
            stored_as: "U8".to_string(),
            min: Some(1.0),
            max: Some(3.0),
            first: vec![1.0, 2.0, 3.0],
            last: vec![1.0, 2.0, 3.0],
            sha256: "039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81".to_string(),
        })
    );
    let empty = Fingerprint::of(&ColumnData::I32(Shared::new(Vec::new()))).expect("numbers");
    assert!(
        empty.min.is_none()
            && empty.max.is_none()
            && empty.first.is_empty()
            && empty.last.is_empty()
    );
    // a long column shows its ends only, and a hash that does not depend on how the values are cut into blocks
    let long =
        Fingerprint::of(&ColumnData::I32(Shared::new((0..20_000).collect()))).expect("numbers");
    assert_eq!(
        (long.first.clone(), long.last.clone()),
        (vec![0.0, 1.0, 2.0], vec![19_997.0, 19_998.0, 19_999.0])
    );
    // more values than one hashing block holds, against the hash that `struct` and `hashlib` give for the same 20,000 values
    assert_eq!(
        long.sha256,
        "bc995f75a4732ad808f5e637dda6107583b0303ec454d6f55042f5f69609c659"
    );
    // the fingerprint survives its own JSON
    let text = long.to_json().to_string();
    let again: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(again["count"], 20_000);
    assert_eq!(again["stored_as"], "I32");
}

/// A 16-bit integer, both ends of its range (IP-06 phase 6I: tauri's XY2-100E writes its commanded positions as `int16`): two bytes for each value, little endian. The literal is the
/// oracle's, worked out there with `struct` and `hashlib`, and its test holds the same one.
#[test]
fn the_oracle_and_the_fingerprint_agree_on_int16_bytes() {
    let column = ColumnData::I16(Shared::new(vec![1, -2, 30_000, i16::MIN]));
    let fingerprint = Fingerprint::of(&column).expect("numbers");
    assert_eq!(
        fingerprint.sha256,
        "ebc97ebfe8030757b4270b6bd32a0d344ae75842bb9a4400030c266b3f80c1c9"
    );
    assert_eq!(
        (
            fingerprint.stored_as.as_str(),
            fingerprint.count,
            fingerprint.min,
            fingerprint.max
        ),
        ("I16", 4, Some(-32_768.0), Some(30_000.0))
    );
    // the bytes of one value are its two, low byte first
    assert_eq!(
        Fingerprint::of(&ColumnData::I16(Shared::new(vec![0x0102])))
            .expect("numbers")
            .sha256,
        "25dfd29c09617dcc9852281c030e5b3037a338a4712a42a21c907f259c6412a0",
        "0x0102 is the bytes 02 01"
    );
}

#[test]
fn a_column_that_is_not_numbers_has_no_fingerprint() {
    assert!(Fingerprint::of(&ColumnData::Text(Shared::new(vec!["a".to_string()]))).is_none());
    assert!(Fingerprint::of(&ColumnData::Bool(Shared::new(vec![true]))).is_none());
}

// ---------------------------------------------------------------- the expectations

#[test]
fn an_expectation_that_names_no_fingerprint_for_a_v16_or_v17_channel_is_reported() {
    // a V16 or V17 expectation must describe every channel recorded with samples: a check that has nothing to check cannot pass
    let source = source_with(|_, _, _, data| Some(data));
    for version in [16, 17] {
        let lines = check_columns(&source, &expectation(version, false));
        assert_eq!(lines.len(), 1, "V{version}: {lines:?}");
        assert!(
            lines[0].contains("has no fingerprint"),
            "V{version}: {lines:?}"
        );
    }
    // an expectation of V18 describes none yet, and so has none to check
    assert!(check_columns(&source, &expectation(18, false)).is_empty());
    assert!(column_expectations(&expectation(18, false))
        .expect("none")
        .is_empty());
}

#[test]
fn the_v16_and_the_six_v17_expectations_describe_every_recorded_channel_and_v18_describes_none() {
    let all = load_all().expect("the expectation files");
    assert_eq!(all.len(), 8);
    let (mut described_files, mut others, mut columns) = (0, 0, 0);
    for expectation in &all {
        let described = column_expectations(expectation)
            .unwrap_or_else(|e| panic!("{}: {e}", expectation.miniature()));
        if described.is_empty() {
            others += 1;
        } else {
            described_files += 1;
            columns += described.len();
            // a stored type that a fingerprint names is one that a column can be
            assert!(described
                .iter()
                .all(|c| ["F32", "U8", "I32"].contains(&c.fingerprint.stored_as.as_str())));
        }
    }
    assert_eq!(
        (described_files, others),
        (7, 1),
        "V18 is the one that describes none"
    );
    // the six V17 files describe 208 columns and the V16 file 16
    assert_eq!(columns, 224, "{columns} columns are described");
}
