//! Where a laser's correction grid is stored (IP-24 phase 24B, task T24.2, decision D24-11; in the toolkit because two versions keep their grids this way, decision D12-12): the part of the **locator** that
//! is the same for every such version. The engine reads the dataset (`crate::columns::read_grid`): the rules for the type, the extents and the order of the values are the engine's and the same for every
//! version.
//!
//! * **A grid is a dataset of a group that belongs to the laser**: the adapter says which ([`GridHome`]), by the group that holds every laser's calibration and the prefix that, with the laser's number,
//!   names the laser's own group. The dataset is there by the name the grid's descriptor gives.
//! * **The metadata answers first** (D06-9): a laser that the build does not have, or that has no snapshot, or a grid that its snapshot does not list, is `NotPresent` and the file is not touched. A grid that
//!   the metadata lists and the file does not hold is `Corrupt`.
//! * **Nothing is interpreted.** Which grid it is, its domain and the order of its axes are not stated by any file, and nothing here decides them.

use build_log_model::{BuildMeta, Error, ErrorKind, GridRequest};
use hdf5::Group;

use crate::columns::Located;

/// Where a version keeps the grids of a laser: in the group named `home_prefix` and the laser's number, inside the group `calibration` of the mount. An adapter passes its own names.
pub struct GridHome {
    pub calibration: &'static str,
    pub home_prefix: &'static str,
}

fn not_present(what: &str) -> Error {
    Error::new(ErrorKind::NotPresent {
        what: what.to_string(),
    })
}

/// The dataset that holds the grid `request` names, in the build whose metadata is `build` and whose data is under `mount`, for a version that keeps its grids as `home` says.
pub fn locate_grid(
    build: &BuildMeta,
    mount: &Group,
    home: &GridHome,
    request: &GridRequest,
) -> Result<Located, Error> {
    let laser = build
        .laser(&request.laser)
        .ok_or_else(|| not_present("the laser"))?;
    let listed = laser
        .parameters
        .as_ref()
        .is_some_and(|p| p.grids.iter().any(|g| g.name == request.grid));
    if !listed {
        return Err(not_present("the grid of this laser"));
    }
    let missing = || {
        Error::new(ErrorKind::Corrupt {
            detail: format!(
                "the metadata says the laser has the grid `{}`, and the file has no dataset for it",
                request.grid
            ),
        })
    };
    mount
        .group(home.calibration)
        .and_then(|calibration| {
            calibration.group(&format!("{}{}", home.home_prefix, request.laser.number))
        })
        .and_then(|group| group.dataset(&request.grid))
        .map(Located::new)
        .map_err(|_| missing())
}
