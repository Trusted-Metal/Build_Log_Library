//! The lasers of a V16 build (IP-12 phase 12D): what V16 says about where its lasers and their channels are. **The reader is the toolkit's** ([`crate::toolkit::lasers`], decision D12-1), which holds the rules
//! that every version shares (a laser's stream is as long as its longest channel, a null dataspace is an empty channel, a laser that cannot be read is reported on itself); this file passes it
//! V16's names and facts.
//!
//! * **A laser is a group named for it in a layer** (`Laser_` and a number), and **a channel is a dataset the adapter knows**: the power, the state and the six positions, in the two groups of
//!   channels of a laser, as in V17. The unit of the power (volts) and of the positions (bits) is declared by the adapter (J7); the state is an enum whose labels nobody has declared.
//! * **A laser that did not fire keeps its group** (decision D12-4): it has its two groups of channels and no channel in them, so it is recorded and empty in every layer, with no channel described.
//! * **V16 has no window and no completeness flag**: a laser's group has no attribute, so the time axis has no window and the laser's `complete` is not recorded. Nothing is invented.
//! * **The sample period is the build's** (decision D12-5): one attribute of the layers' group, written as text that has its unit in it (`10us`), read as one quantity and given to every laser. V16
//!   has none for each laser, and none is made up from the one it has.

use build_log_model::{parse_embedded, Axis, ChannelKind, Error, ErrorKind, Quantity};
use hdf5::{Group, Location};

use super::layout;
use crate::toolkit::lasers::{ChannelSpec, LaserLayout, Lasers};
use crate::toolkit::values::{optional_written, Written};

fn corrupt(detail: String) -> Error {
    Error::new(ErrorKind::Corrupt { detail })
}

/// The eight channels, in the order that a laser's channels are listed.
pub(super) fn specs() -> Vec<ChannelSpec> {
    let position = |dataset, kind| ChannelSpec {
        group: layout::POSITION,
        dataset,
        kind,
        unit: Some("bits"),
    };
    vec![
        ChannelSpec {
            group: layout::LASER_DATA,
            dataset: layout::LASER_POWER,
            kind: ChannelKind::LaserPower,
            unit: Some("V"),
        },
        ChannelSpec {
            group: layout::LASER_DATA,
            dataset: layout::LASER_STATE,
            kind: ChannelKind::LaserState,
            unit: None,
        },
        position(
            layout::ACTUAL_X,
            ChannelKind::ActualPosition { axis: Axis::X },
        ),
        position(
            layout::ACTUAL_Y,
            ChannelKind::ActualPosition { axis: Axis::Y },
        ),
        position(
            layout::ACTUAL_Z,
            ChannelKind::ActualPosition { axis: Axis::Z },
        ),
        position(
            layout::COMMANDED_X,
            ChannelKind::CommandedPosition { axis: Axis::X },
        ),
        position(
            layout::COMMANDED_Y,
            ChannelKind::CommandedPosition { axis: Axis::Y },
        ),
        position(
            layout::COMMANDED_Z,
            ChannelKind::CommandedPosition { axis: Axis::Z },
        ),
    ]
}

/// A quantity that V16 writes as a number with no unit (`10.0`) or as text that has its unit in it (`10us`), as **one quantity**: the value as written, and a unit only where the file states one
/// (decision D13-25; IP-12 phase 12D). Text that is neither is `Corrupt`, and says which attribute. It is built from the toolkit's reader of a value that may be a number or text, and it stays here
/// until a second version needs it (decision D12-12: the toolkit holds what two versions use).
pub fn optional_embedded_quantity(
    location: &Location,
    name: &str,
) -> Result<Option<Quantity>, Error> {
    match optional_written(location, name)? {
        None => Ok(None),
        Some(Written::Number(value)) => Ok(Some(Quantity::from_file(value, None))),
        Some(Written::Text(text)) => parse_embedded(&text).map(Some).ok_or_else(|| {
            corrupt(format!(
                "the attribute `{name}` holds `{text}`, which is neither a number nor a number with a unit"
            ))
        }),
    }
}

/// The build's sample period, the same for every laser: the layers' group's attribute as one quantity. `None` if the file has none (the accuracy of the time axis is then unknown).
fn build_period(mount: &Group) -> Result<Option<Quantity>, Error> {
    if !mount.link_exists(layout::MONITORING_DATA) {
        return Ok(None);
    }
    let monitoring = mount
        .group(layout::MONITORING_DATA)
        .map_err(|e| corrupt(format!("`{}` is not a group: {e}", layout::MONITORING_DATA)))?;
    optional_embedded_quantity(&monitoring, layout::SAMPLE_PERIOD)
}

/// The sample period of laser `number`, which is the build's: V16 has one for the whole build (decision D12-5).
fn read_period(mount: &Group, _number: u32) -> Result<Option<Quantity>, Error> {
    build_period(mount)
}

/// What V16 says about its lasers, for the toolkit's reader and its locator.
pub(super) fn laser_layout() -> LaserLayout {
    LaserLayout {
        label: "V16",
        layers_group: layout::MONITORING_DATA,
        layer_prefix: layout::LAYER_PREFIX,
        laser_prefix: layout::LASER_PREFIX,
        channels: specs(),
        // V16 has no window and no flag: a laser's group has no attribute
        window: None,
        completeness: None,
        typed_attributes: &[],
        period: read_period,
    }
}

/// The lasers of the build whose data is under `mount`, collected layer by layer.
pub fn collector(mount: &Group) -> Lasers {
    Lasers::new(mount, laser_layout())
}
