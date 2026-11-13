//! The metadata of a source, read whole or one layer at a time (IP-26 phase 26K, decision D26-19, IP-24 finding F17).
//!
//! A version says **how it reads** ([`Walk`]): its build's own level with the ids of its layers, one layer by itself, and every layer. This file says **when**:
//!
//! * **Lazy** ([`Meta::lazy`]): the overview is read at once, and so errors in the build's own level or in the list of the layers are reported when the file is opened. **A
//!   layer is read when it is asked for, once, and kept for as long as the source lives** (no budget: about 12 KB for a layer). **The whole build is read the first time
//!   [`Meta::build`] is called, once, and kept.** Nothing else walks every layer.
//! * **Eager** ([`Meta::eager`]): a build that is already whole (a test's, a mock's, a source made by hand). The overview and a layer's detail are cut from it.
//!
//! The source and its locator share one [`Meta`]: a request for a column asks it for the layer that the request names, and so never asks for the whole build.
//!
//! This names no attribute, no group and no version.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use build_log_model::{BuildMeta, BuildOverview, Error, LayerDetail, LayerId};

/// How a version reads its metadata.
pub trait Walk: Send + Sync {
    /// The build's own level and the ids of its layers. It reads no layer.
    fn overview(&self) -> Result<BuildOverview, Error>;
    /// One layer by itself: the same as the whole walk reads for that layer. The layer is one that [`Walk::overview`] lists.
    fn layer(&self, id: LayerId) -> Result<LayerDetail, Error>;
    /// Every layer: the whole build.
    fn full(&self) -> Result<BuildMeta, Error>;
}

/// How much a [`Meta`] has read, for the tests that show a request does not walk more than it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetaStats {
    /// How many times the whole build was walked.
    pub full_walks: usize,
    /// How many layers were read by themselves.
    pub layers_read: usize,
}

enum Mode {
    Eager(Arc<BuildMeta>),
    Lazy {
        walk: Arc<dyn Walk>,
        ids: BTreeSet<LayerId>,
        build: Mutex<Option<Arc<BuildMeta>>>,
    },
}

/// The metadata of a source: the overview, the layers read so far, and the whole build when it has been asked for.
pub struct Meta {
    mode: Mode,
    overview: Arc<BuildOverview>,
    layers: Mutex<HashMap<LayerId, Arc<LayerDetail>>>,
    full_walks: AtomicUsize,
    layers_read: AtomicUsize,
}

fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Meta {
    /// The metadata of a build that is already whole.
    pub fn eager(build: Arc<BuildMeta>) -> Arc<Meta> {
        let overview = Arc::new(BuildOverview::from_build(&build));
        Arc::new(Meta {
            mode: Mode::Eager(build),
            overview,
            layers: Mutex::new(HashMap::new()),
            full_walks: AtomicUsize::new(0),
            layers_read: AtomicUsize::new(0),
        })
    }

    /// The metadata of a build that is read as it is asked for. The overview is read now, so that what is wrong with the build's own level is found when the file is
    /// opened.
    pub fn lazy(walk: Arc<dyn Walk>) -> Result<Arc<Meta>, Error> {
        let overview = Arc::new(walk.overview()?);
        let ids = overview.layers.iter().copied().collect();
        Ok(Arc::new(Meta {
            mode: Mode::Lazy {
                walk,
                ids,
                build: Mutex::new(None),
            },
            overview,
            layers: Mutex::new(HashMap::new()),
            full_walks: AtomicUsize::new(0),
            layers_read: AtomicUsize::new(0),
        }))
    }

    /// The build's own level and the ids of its layers.
    pub fn overview(&self) -> Arc<BuildOverview> {
        Arc::clone(&self.overview)
    }

    /// The whole build. For a lazy source the first call walks every layer, and the result is kept.
    pub fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        match &self.mode {
            Mode::Eager(build) => Ok(Arc::clone(build)),
            Mode::Lazy { walk, build, .. } => {
                let mut kept = held(build);
                if let Some(build) = kept.as_ref() {
                    return Ok(Arc::clone(build));
                }
                let walked = Arc::new(walk.full()?);
                self.full_walks.fetch_add(1, Relaxed);
                *kept = Some(Arc::clone(&walked));
                Ok(walked)
            }
        }
    }

    /// One layer's detail, or `None` if the build does not hold the layer. A lazy source reads the layer by itself the first time and keeps it.
    pub fn layer(&self, id: LayerId) -> Result<Option<Arc<LayerDetail>>, Error> {
        let mut kept = held(&self.layers);
        if let Some(detail) = kept.get(&id) {
            return Ok(Some(Arc::clone(detail)));
        }
        let detail = match &self.mode {
            Mode::Eager(build) => LayerDetail::from_build(build, id),
            Mode::Lazy { walk, ids, .. } => {
                if !ids.contains(&id) {
                    return Ok(None);
                }
                let detail = walk.layer(id)?;
                self.layers_read.fetch_add(1, Relaxed);
                Some(detail)
            }
        };
        Ok(detail.map(|detail| {
            let detail = Arc::new(detail);
            kept.insert(id, Arc::clone(&detail));
            detail
        }))
    }

    /// What has been read so far.
    pub fn stats(&self) -> MetaStats {
        MetaStats {
            full_walks: self.full_walks.load(Relaxed),
            layers_read: self.layers_read.load(Relaxed),
        }
    }
}
