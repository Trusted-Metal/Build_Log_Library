//! Where the samples of V17 are stored (IP-13 phase 13L, task T13.14; IP-06 decisions D06-1 (c) and D06-9): the **locator** that the column engine asks.
//!
//! The engine (`crate::columns`) knows the rules for every version, and the toolkit ([`crate::toolkit::columns`], IP-12 phase 12A) finds a laser's channel for a version laid out as a
//! [`LaserLayout`] says; this file only holds what is V17's: **its layout** ([`super::lasers`]) **and the mount and the metadata that the locator asks** (the camera images and the OPC UA
//! events have accessors of their own, which use them too).
//!
//! Every name comes from [`super::layout`] and [`super::lasers`], which is where the channels are listed.

use std::sync::Arc;

use build_log_model::{BuildMeta, ColumnRequest, Error, GridRequest};
use hdf5::Group;

use super::grids::GRID_HOME;
use super::lasers::laser_layout;
use super::KEY;
use crate::columns::{Located, Locator};
use crate::toolkit::columns::locate;
use crate::toolkit::grids::locate_grid;
use crate::toolkit::lasers::LaserLayout;

/// Finds the datasets of the columns of a V17 build: its metadata says what is recorded, and its mount says where.
pub struct V17Locator {
    pub(super) build: Arc<BuildMeta>,
    pub(super) mount: Group,
    layout: LaserLayout,
}

impl V17Locator {
    pub fn new(build: Arc<BuildMeta>, mount: Group) -> Self {
        V17Locator {
            build,
            mount,
            layout: laser_layout(),
        }
    }
}

impl Locator for V17Locator {
    fn version(&self) -> &str {
        KEY
    }

    fn locate(&self, request: &ColumnRequest) -> Result<Located, Error> {
        locate(&self.build, &self.mount, &self.layout, request)
    }

    fn locate_grid(&self, request: &GridRequest) -> Result<Located, Error> {
        locate_grid(&self.build, &self.mount, &GRID_HOME, request)
    }
}
