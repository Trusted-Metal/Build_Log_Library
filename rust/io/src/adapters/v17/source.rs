//! The source of a V17 build: its metadata, and its samples (IP-13 decision D13-8, and phase 13L).
//!
//! **The metadata and the samples are the toolkit's source** ([`crate::toolkit::source::SampleSource`], IP-12 phase 12A), which holds the metadata that the reader built, the locator that
//! knows where the samples are and the two reports. **V17 adds what only it has:** the OPC UA events and the camera images (phases 6F and 6G), which are answered here through the same
//! reader of columns.

use std::sync::Arc;

use build_log_model::{
    AvailabilityReport, BuildMeta, BuildSource, CacheStats, ColumnData, ColumnRequest,
    ColumnSource, Error, EventColumns, EventRequest, GridData, GridRequest, ImageData,
    ImageRequest, ReadContext, Revision, ValidationReport,
};
use hdf5::Group;

use super::columns::V17Locator;
use super::event_columns;
use super::image_regions;
use crate::toolkit::source::SampleSource;

/// A V17 build: the toolkit's source, with V17's events and images.
pub struct V17Source {
    inner: SampleSource<V17Locator>,
}

impl V17Source {
    pub fn new(build: BuildMeta, mount: Group) -> Self {
        let build = Arc::new(build);
        let locator = V17Locator::new(Arc::clone(&build), mount);
        V17Source {
            inner: SampleSource::new(build, locator),
        }
    }
}

impl ColumnSource for V17Source {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        self.inner.read(request, context)
    }

    fn read_events(
        &self,
        request: &EventRequest,
        context: &ReadContext,
    ) -> Result<EventColumns, Error> {
        event_columns::read_events(self.inner.columns(), request, context)
            .map_err(|error| error.at(request.location()))
    }

    fn read_image(
        &self,
        request: &ImageRequest,
        context: &ReadContext,
    ) -> Result<ImageData, Error> {
        image_regions::read_image(self.inner.columns().locator(), request, context)
            .map_err(|error| error.at(request.location()))
    }

    fn read_grid(&self, request: &GridRequest, context: &ReadContext) -> Result<GridData, Error> {
        self.inner.read_grid(request, context)
    }

    fn cache_stats(&self) -> Option<CacheStats> {
        self.inner.cache_stats()
    }
}

impl BuildSource for V17Source {
    fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        self.inner.build()
    }

    fn revision(&self) -> Result<Revision, Error> {
        self.inner.revision()
    }

    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        self.inner.validate()
    }

    fn availability(&self) -> Result<Arc<AvailabilityReport>, Error> {
        self.inner.availability()
    }
}
