//! Where the samples of V17 are stored (IP-13 phase 13L, task T13.14; IP-06 decisions D06-1 (c) and D06-9): the **locator** that the column engine asks.
//!
//! The engine (`crate::columns`) knows the rules for every version, and the toolkit ([`crate::toolkit::columns`], IP-12 phase 12A) finds a laser's channel for a version laid out as a
//! [`LaserLayout`] says; this file only holds what is V17's: **its layout** ([`super::lasers`]) **and the mount and the metadata that the locator asks** (the camera images and the OPC UA
//! events have accessors of their own, which use them too).
//!
//! Every name comes from [`super::layout`] and [`super::lasers`], which is where the channels are listed.

use std::sync::Arc;

use build_log_model::{BuildMeta, ColumnRequest, Error, GridRequest};

use crate::toolkit::meta::Meta;
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
    meta: Arc<Meta>,
    pub(super) mount: Group,
    layout: LaserLayout,
}

impl V17Locator {
    /// A locator for a build that is already whole.
    pub fn new(build: Arc<BuildMeta>, mount: Group) -> Self {
        V17Locator::with_meta(Meta::eager(build), mount)
    }

    /// A locator that shares the metadata of its source (IP-26 phase 26K): a request for a column asks it for the layer it names and no other.
    pub fn with_meta(meta: Arc<Meta>, mount: Group) -> Self {
        V17Locator {
            meta,
            mount,
            layout: laser_layout(),
        }
    }

    /// The whole build. **For a lazy source this walks every layer the first time**; the events and the camera images are described by that walk, so a request for
    /// either asks for it.
    pub(super) fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        self.meta.build()
    }
}

impl Locator for V17Locator {
    fn version(&self) -> &str {
        KEY
    }

    fn locate(&self, request: &ColumnRequest) -> Result<Located, Error> {
        let layer = self.meta.layer(request.layer)?;
        locate(layer.as_deref(), &self.mount, &self.layout, request)
    }

    fn locate_grid(&self, request: &GridRequest) -> Result<Located, Error> {
        locate_grid(&self.meta.overview(), &self.mount, &GRID_HOME, request)
    }
}
