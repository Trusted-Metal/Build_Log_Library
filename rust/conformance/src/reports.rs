//! The two reports of a source, held to the expectation of a miniature (IP-07 phases 7C and 7D, tasks T07.3 and T07.4; decisions D07-4 and D07-5).
//!
//! An expectation file carries, besides what the CDM must show, the **findings** that the metadata of the miniature must give (`validation`: the code, the severity and the layer, laser and
//! channel of each, and the counts of each severity) and the **availability** of its items (`availability`: for layers, lasers in layers and channels in layers, how many are in each
//! state, and the items that are degraded, unreadable or unsupported). Both are written by the independent script from the facts it read with `h5py`, by rules written there and nowhere else.
//!
//! * [`check_validation`] asks a source for its validation report and says every finding that differs, by what it is and where.
//! * [`check_availability`] asks a source for its availability report and says every count and every exception that differs.
//!
//! Both are cheap (neither reads a sample), and each is itself tested by mutation (`tests/reports.rs`). **An expectation that carries neither is an error**, so that a check cannot pass by
//! having nothing to check.

use std::collections::BTreeMap;

use build_log_model::{Availability, BuildSource, ItemKind};

use crate::expectation::{array, field, int, opt_int, opt_text, text};
use crate::Expectation;

/// One finding or exception as a line: the words and the place.
fn place(layer: Option<i64>, laser: Option<&str>, channel: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(layer) = layer {
        parts.push(format!("layer {layer}"));
    }
    if let Some(laser) = laser {
        parts.push(format!("laser {laser}"));
    }
    if let Some(channel) = channel {
        parts.push(format!("channel {channel}"));
    }
    if parts.is_empty() {
        "the build".to_string()
    } else {
        parts.join(", ")
    }
}

/// What the expectation says of each finding, as lines that can be compared and counted.
fn expected_findings(expectation: &Expectation) -> Result<(Vec<String>, [u64; 3]), String> {
    let validation = field(expectation.root(), "validation")?;
    let mut lines = Vec::new();
    for finding in array(validation, "findings")? {
        lines.push(format!(
            "{} ({}) at {}",
            text(finding, "code")?,
            text(finding, "severity")?,
            place(
                opt_int(finding, "layer")?,
                opt_text(finding, "laser")?.as_deref(),
                opt_text(finding, "channel")?.as_deref()
            )
        ));
    }
    let counts = field(validation, "counts")?;
    Ok((
        lines,
        [
            int(counts, "notes")? as u64,
            int(counts, "warnings")? as u64,
            int(counts, "faults")? as u64,
        ],
    ))
}

/// Compares two lists as multisets and says each difference once, in order.
fn differences(what: &str, found: &mut Vec<String>, actual: Vec<String>, wanted: Vec<String>) {
    let mut left: BTreeMap<String, i64> = BTreeMap::new();
    for line in &actual {
        *left.entry(line.clone()).or_default() += 1;
    }
    for line in &wanted {
        *left.entry(line.clone()).or_default() -= 1;
    }
    for (line, balance) in left {
        match balance {
            0 => {}
            n if n > 0 => found.push(format!(
                "{what}: not expected: {line} (reported {n} too many)"
            )),
            n => found.push(format!("{what}: missing: {line} (expected {} more)", -n)),
        }
    }
}

/// The validation report of a source against the findings of an expectation. Each finding that is missing, each that is not expected and each count that differs is a line.
pub fn check_validation(source: &dyn BuildSource, expectation: &Expectation) -> Vec<String> {
    let mut found = Vec::new();
    let (wanted, wanted_counts) = match expected_findings(expectation) {
        Ok(wanted) => wanted,
        Err(e) => return vec![format!("the expectation of the findings: {e}")],
    };
    let report = match source.validate() {
        Ok(report) => report,
        Err(e) => return vec![format!("the validation report: {e}")],
    };
    let actual: Vec<String> = report
        .findings
        .iter()
        .map(|f| {
            format!(
                "{} ({:?}) at {}",
                f.code.name(),
                f.severity,
                place(
                    f.location.layer.map(|l| i64::from(l.0)),
                    f.location.laser.as_ref().map(|l| l.key.as_str()),
                    f.location
                        .channel
                        .as_ref()
                        .map(|c| c.segments.join("/"))
                        .as_deref()
                )
            )
        })
        .collect();
    differences("a finding", &mut found, actual, wanted);
    let counts = [
        report.counts.notes,
        report.counts.warnings,
        report.counts.faults,
    ];
    for (name, (got, want)) in ["notes", "warnings", "faults"]
        .iter()
        .zip(counts.iter().zip(wanted_counts.iter()))
    {
        if got != want {
            found.push(format!("the {name} counted: {got}, expected {want}"));
        }
    }
    found
}

fn state_name(state: &Availability) -> &'static str {
    match state {
        Availability::Present => "present",
        Availability::NotRecorded => "not_recorded",
        Availability::Degraded { .. } => "degraded",
        Availability::Unreadable { .. } => "unreadable",
        Availability::Unsupported { .. } => "unsupported",
        _ => "another",
    }
}

fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Layer => "layer",
        ItemKind::Laser => "laser",
        ItemKind::Channel => "channel",
        _ => "another",
    }
}

/// The availability report of a source against the expectation: the counts of each state for each kind of item, and the items that are not simply present or not recorded.
pub fn check_availability(source: &dyn BuildSource, expectation: &Expectation) -> Vec<String> {
    let mut found = Vec::new();
    let expected = match field(expectation.root(), "availability") {
        Ok(expected) => expected,
        Err(e) => return vec![format!("the expectation of the availability: {e}")],
    };
    let report = match source.availability() {
        Ok(report) => report,
        Err(e) => return vec![format!("the availability report: {e}")],
    };
    let counts_of = |kind: ItemKind, key: &str, found: &mut Vec<String>| -> Result<(), String> {
        let wanted = field(expected, key)?;
        let actual = report.counts_for(kind);
        for (state, got) in [
            ("present", actual.present),
            ("not_recorded", actual.not_recorded),
            ("degraded", actual.degraded),
            ("unreadable", actual.unreadable),
            ("unsupported", actual.unsupported),
        ] {
            let want = int(wanted, state)? as u64;
            if got != want {
                found.push(format!("the {state} {key}: {got}, expected {want}"));
            }
        }
        Ok(())
    };
    for (kind, key) in [
        (ItemKind::Layer, "layers"),
        (ItemKind::Laser, "lasers"),
        (ItemKind::Channel, "channels"),
    ] {
        if let Err(e) = counts_of(kind, key, &mut found) {
            found.push(format!("the expectation of the {key}: {e}"));
        }
    }
    let wanted: Result<Vec<String>, String> = array(expected, "exceptions").and_then(|items| {
        items
            .iter()
            .map(|e| {
                Ok(format!(
                    "{} {} at {}",
                    text(e, "item")?,
                    text(e, "state")?,
                    place(
                        opt_int(e, "layer")?,
                        opt_text(e, "laser")?.as_deref(),
                        opt_text(e, "channel")?.as_deref()
                    )
                ))
            })
            .collect()
    });
    let actual: Vec<String> = report
        .exceptions()
        .iter()
        .map(|e| {
            let location = e.item.location();
            format!(
                "{} {} at {}",
                kind_name(e.item.kind()),
                state_name(&e.availability),
                place(
                    location.layer.map(|l| i64::from(l.0)),
                    location.laser.as_ref().map(|l| l.key.as_str()),
                    location
                        .channel
                        .as_ref()
                        .map(|c| c.segments.join("/"))
                        .as_deref()
                )
            )
        })
        .collect();
    match wanted {
        Ok(wanted) => differences("an item", &mut found, actual, wanted),
        Err(e) => found.push(format!("the expectation of the exceptions: {e}")),
    }
    found
}
