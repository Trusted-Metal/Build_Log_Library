//! Where the samples of V16 are stored: the **locator** that the column engine asks (IP-12 phase 12F, task T12.6; IP-06 decisions D06-1 (c) and D06-9; decision D12-9).
//!
//! The engine (`crate::columns`) knows the rules for every version, and the toolkit ([`crate::toolkit::columns`]) finds a laser's channel for a version laid out as a [`LaserLayout`] says; this
//! file only holds what is V16's: **its layout** ([`super::lasers`]) **and the mount and the metadata that the locator asks.**
//!
//! * **A column is a laser's channel in a layer**: the power (`float32`), the state (`uint8`) and the six positions (`int32`), each stored as **one gzip chunk for all its values** and not resizable
//!   (G17). The engine reads each as stored: nothing is narrowed, widened or converted.
//! * **The metadata answers first** (D06-9): a laser that fired but has no channel recorded in a layer, and a laser that did not fire (`Laser_2`, with no channel described), are `NotPresent`, and
//!   the file is not touched.
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

/// Finds the datasets of the columns of a V16 build: its metadata says what is recorded, and its mount says where.
pub struct V16Locator {
    meta: Arc<Meta>,
    mount: Group,
    layout: LaserLayout,
}

impl V16Locator {
    /// A locator for a build that is already whole.
    pub fn new(build: Arc<BuildMeta>, mount: Group) -> Self {
        V16Locator::with_meta(Meta::eager(build), mount)
    }

    /// A locator that shares the metadata of its source (IP-26 phase 26K): a request for a column asks it for the layer it names and no other.
    pub fn with_meta(meta: Arc<Meta>, mount: Group) -> Self {
        V16Locator {
            meta,
            mount,
            layout: laser_layout(),
        }
    }
}

impl Locator for V16Locator {
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
