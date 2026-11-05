//! Where the correction grids of V17 are stored (IP-24 phase 24B, task T24.2; decision D24-11): the one thing of the **locator** that is V17's, the group that holds a laser's grids. The toolkit finds the
//! dataset ([`crate::toolkit::grids`], which two versions use, decision D12-12) and the engine reads it (`crate::columns::read_grid`).
//!
//! * **A grid belongs to the laser's own group** below the calibration: laser N uses the optical train N (decision D13-18). The grid is a dataset of that group, by the name its descriptor gives.
//!
//! Every name comes from [`super::layout`].

use super::layout;
use crate::toolkit::grids::GridHome;

/// Where V17 keeps the grids of a laser.
pub(super) const GRID_HOME: GridHome = GridHome {
    calibration: layout::CALIBRATION_DATA,
    home_prefix: layout::OPTICAL_TRAIN_PREFIX,
};
