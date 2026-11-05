//! The lasers of a V17 build (IP-13 phase 13D): what V17 says about where its lasers and their channels are. **The reader is the toolkit's** ([`crate::toolkit::lasers`], IP-12 phase 12A,
//! decision D12-1), which holds the rules that every version shares (a laser's stream is as long as its longest channel, a null dataspace is an empty channel, a laser that cannot
//! be read is reported on itself); this file passes it V17's names and facts.
//!
//! * **A laser is a group named for it in a layer** (`Laser_` and a number), and **a channel is a dataset the adapter knows**: the power, the state and the six positions, in the two
//!   groups of channels of a laser. The unit of the power (volts) and of the positions (bits) is declared by the adapter (J7); the state is an enum whose labels nobody has declared.
//! * **The window is the laser's own start and end times as written** (absent from the March files, so there is no window), and **the flag is the laser's `complete`**, kept as written.
//! * **The sample period is the scanner card's of the optical train of the same number**, with no unit (decisions D13-5, D13-18 and D13-25).

use build_log_model::{Axis, ChannelKind, Error, ErrorKind, Quantity};
use hdf5::Group;

use super::layout;
use crate::toolkit::lasers::{ChannelSpec, LaserLayout, Lasers};

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

/// The sample period of laser `number`: the scanner card's of the optical train of the same number, with no unit. `None` if the file has none.
fn read_period(mount: &Group, number: u32) -> Result<Option<Quantity>, Error> {
    let train = format!("{}{number}", layout::OPTICAL_TRAIN_PREFIX);
    let mut group = mount.clone();
    for part in [
        layout::CALIBRATION_DATA,
        train.as_str(),
        layout::SCANNER_CARD,
    ] {
        if !group.link_exists(part) {
            return Ok(None);
        }
        group = group
            .group(part)
            .map_err(|e| corrupt(format!("`{part}` is not a group: {e}")))?;
    }
    // a number with no unit, or text that has its unit in it: one quantity (decision D13-25)
    super::parameters::sample_period(&group)
}

/// What V17 says about its lasers, for the toolkit's reader and its locator.
pub(super) fn laser_layout() -> LaserLayout {
    LaserLayout {
        label: "V17",
        layers_group: layout::MONITORING_DATA,
        layer_prefix: layout::LAYER_PREFIX,
        laser_prefix: layout::LASER_PREFIX,
        channels: specs(),
        window: Some((layout::START_LASER_TIME, layout::END_LASER_TIME)),
        completeness: Some(layout::COMPLETE),
        typed_attributes: &layout::TYPED_ON_A_LASER,
        period: read_period,
    }
}

/// The lasers of the build whose data is under `mount`, collected layer by layer.
pub fn collector(mount: &Group) -> Lasers {
    Lasers::new(mount, laser_layout())
}
