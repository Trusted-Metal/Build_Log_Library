//! The source of a build that was read from an archive, guarded against a change of the file under it.
//!
//! HDF5 caches what it has read (the chunk index, the chunks), and a source caches what it has parsed. If the file is modified in place while they are in use, what they hold may mix two versions of the file. On Windows the open archive
//! keeps writers out (see the archive layer's `GuardedFile`) and this can not happen; on other systems it can, and the guard is what notices. **Before every call** it asks the archive whether the bytes under the open file changed; if they did,
//! this call and every later one fail with `FileChanged`: the source must be closed and the file opened again. A file that was only *replaced* by a new version (renamed over) or *removed* is not an error: the open handle still reads the
//! consistent file that was opened. A consumer asks `Opened::check_file` to learn that.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use build_log_archive::FileArchive;
use build_log_model::{
    AvailabilityReport, BuildMeta, BuildOverview, BuildSource, CacheStats, ChannelDescriptor,
    ColumnData, ColumnRequest, ColumnSource, Error, ErrorKind, EventColumns, EventRequest,
    GridData, GridRequest, ImageData, ImageRequest, LaserKey, LayerDetail, LayerId, ReadContext,
    Revision, ValidationReport,
};

/// A source that fails once the file under it has changed. It delegates **every** method of `ColumnSource` and `BuildSource`, the defaulted ones too (a default would answer from the wrapper and not from the source it wraps, and be slower or
/// wrong): `the_guard_delegates_every_method` in the tests of this crate calls each one through it.
pub struct GuardedSource {
    inner: Box<dyn BuildSource>,
    archive: Arc<FileArchive>,
    stale: AtomicBool,
}

impl GuardedSource {
    /// Guards `inner`, which reads from the open `archive`.
    pub fn new(inner: Box<dyn BuildSource>, archive: Arc<FileArchive>) -> Self {
        GuardedSource {
            inner,
            archive,
            stale: AtomicBool::new(false),
        }
    }

    fn guard(&self) -> Result<(), Error> {
        if !self.stale.load(Ordering::Relaxed) && self.archive.is_modified() {
            self.stale.store(true, Ordering::Relaxed);
        }
        if self.stale.load(Ordering::Relaxed) {
            return Err(Error::new(ErrorKind::FileChanged {
                detail: format!(
                    "`{}` was modified in place after it was opened; close the source and open the file again",
                    self.archive.path().display()
                ),
            }));
        }
        Ok(())
    }
}

impl std::fmt::Debug for GuardedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedSource")
            .field("path", &self.archive.path())
            .field("stale", &self.stale.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl ColumnSource for GuardedSource {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        self.guard()?;
        self.inner.read(request, context)
    }

    fn read_events(
        &self,
        request: &EventRequest,
        context: &ReadContext,
    ) -> Result<EventColumns, Error> {
        self.guard()?;
        self.inner.read_events(request, context)
    }

    fn read_image(
        &self,
        request: &ImageRequest,
        context: &ReadContext,
    ) -> Result<ImageData, Error> {
        self.guard()?;
        self.inner.read_image(request, context)
    }

    fn read_grid(&self, request: &GridRequest, context: &ReadContext) -> Result<GridData, Error> {
        self.guard()?;
        self.inner.read_grid(request, context)
    }

    fn cache_stats(&self) -> Option<CacheStats> {
        self.inner.cache_stats()
    }
}

impl BuildSource for GuardedSource {
    fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        self.guard()?;
        self.inner.build()
    }

    fn revision(&self) -> Result<Revision, Error> {
        self.guard()?;
        self.inner.revision()
    }

    fn layers(&self) -> Result<Vec<LayerId>, Error> {
        self.guard()?;
        self.inner.layers()
    }

    fn lasers_in_layer(&self, layer: LayerId) -> Result<Vec<LaserKey>, Error> {
        self.guard()?;
        self.inner.lasers_in_layer(layer)
    }

    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        self.guard()?;
        self.inner.validate()
    }

    fn availability(&self) -> Result<Arc<AvailabilityReport>, Error> {
        self.guard()?;
        self.inner.availability()
    }

    fn channels(&self, laser: &LaserKey) -> Result<Vec<ChannelDescriptor>, Error> {
        self.guard()?;
        self.inner.channels(laser)
    }

    fn overview(&self) -> Result<Arc<BuildOverview>, Error> {
        self.guard()?;
        self.inner.overview()
    }

    fn layer_meta(&self, layer: LayerId) -> Result<Arc<LayerDetail>, Error> {
        self.guard()?;
        self.inner.layer_meta(layer)
    }
}
