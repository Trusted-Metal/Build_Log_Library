//! The source of a build: its metadata, and its samples (IP-13 decision D13-8 and phase 13L; moved to the toolkit by IP-12 phase 12A, decision D12-1).
//!
//! It holds the metadata that the reader built and a locator that knows where the samples are (which holds the mount the build was read from, so the file stays open for the life of the
//! source: PS-10). **A request for a column is answered by the column engine** (`crate::columns`), through the version's locator, which says which dataset holds the column. A version that
//! also has events or images wraps this source and answers those requests itself; every other method is as the traits say.

use std::sync::{Arc, OnceLock};

use build_log_model::{
    availability_of, validate_build, AvailabilityReport, BuildMeta, BuildOverview, BuildSource,
    CacheStats, ChannelDType, ChannelDescriptor, ColumnData, ColumnRequest, ColumnSource, Error,
    ErrorKind, GridData, GridRequest, LaserKey, LayerDetail, LayerId, Location, ReadContext,
    Revision, ValidationReport,
};

use super::meta::Meta;
use crate::columns::{can_read, ColumnReader, Locator};

/// What a source cannot read of a channel, **from the one list of what the column engine reads** (`columns::can_read`, IP-07 phase 6I, decision D07-3): the feature that is missing, or `None`.
/// A channel whose type is a number is asked; the laser state is an enum, which does not say the width it is stored in, and is not (the files that we hold store it as `uint8`).
fn cannot_read(channel: &ChannelDescriptor) -> Option<String> {
    match &channel.dtype {
        ChannelDType::Numeric(numeric) if !can_read(*numeric) => {
            Some(format!("a column stored as {numeric:?}"))
        }
        _ => None,
    }
}

/// A build: its metadata, and the reader of its samples: the locator that knows where they are and the cache of the blocks it has decoded.
pub struct SampleSource<L: Locator> {
    meta: Arc<Meta>,
    columns: ColumnReader<L>,
    // the two reports of IP-07 are a function of the metadata, which does not change while the file is open (D06-7), so each is made once and shared (D07-8, BL-FA-11)
    validation: OnceLock<Arc<ValidationReport>>,
    availability: OnceLock<Arc<AvailabilityReport>>,
}

impl<L: Locator> SampleSource<L> {
    /// A source of `build`, whose samples `locator` finds.
    pub fn new(build: Arc<BuildMeta>, locator: L) -> Self {
        SampleSource::with_meta(Meta::eager(build), locator)
    }

    /// A source whose metadata is `meta`, which its locator shares (IP-26 phase 26K): a lazy source is made this way.
    pub fn with_meta(meta: Arc<Meta>, locator: L) -> Self {
        SampleSource {
            meta,
            columns: ColumnReader::new(locator),
            validation: OnceLock::new(),
            availability: OnceLock::new(),
        }
    }

    /// The metadata of the source: what has been read of it so far is in [`Meta::stats`].
    pub fn meta(&self) -> &Arc<Meta> {
        &self.meta
    }

    /// The reader of the samples, for a version that answers more requests than `read`.
    pub fn columns(&self) -> &ColumnReader<L> {
        &self.columns
    }
}

impl<L: Locator> ColumnSource for SampleSource<L> {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        self.columns.read(request, context)
    }

    fn read_grid(&self, request: &GridRequest, context: &ReadContext) -> Result<GridData, Error> {
        self.columns.read_grid(request, context)
    }

    fn cache_stats(&self) -> Option<CacheStats> {
        Some(self.columns.stats())
    }
}

impl<L: Locator> BuildSource for SampleSource<L> {
    fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        self.meta.build()
    }

    fn overview(&self) -> Result<Arc<BuildOverview>, Error> {
        Ok(self.meta.overview())
    }

    fn layer_meta(&self, layer: LayerId) -> Result<Arc<LayerDetail>, Error> {
        match self.meta.layer(layer)? {
            Some(detail) => Ok(detail),
            None => Err(Error::new(ErrorKind::InvalidIndex {
                what: "a layer that the build does not hold".to_string(),
            })
            .at(Location::new().with_layer(layer).with_operation("discover"))),
        }
    }

    /// From the overview, which lists the layers: **no layer is read.** (The default asks for the whole build.)
    fn layers(&self) -> Result<Vec<LayerId>, Error> {
        Ok(self.meta.overview().layers.clone())
    }

    /// From the one layer that is asked about: **no other layer is read.** (The default asks for the whole build.)
    fn lasers_in_layer(&self, layer: LayerId) -> Result<Vec<LaserKey>, Error> {
        Ok(self
            .layer_meta(layer)?
            .lasers
            .iter()
            .map(|laser| laser.key.clone())
            .collect())
    }

    fn revision(&self) -> Result<Revision, Error> {
        // a file does not grow while it is open for reading
        Ok(Revision::default())
    }

    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        if let Some(report) = self.validation.get() {
            return Ok(Arc::clone(report));
        }
        let build = self.meta.build()?;
        Ok(Arc::clone(
            self.validation
                .get_or_init(|| Arc::new(validate_build(&build))),
        ))
    }

    fn availability(&self) -> Result<Arc<AvailabilityReport>, Error> {
        if let Some(report) = self.availability.get() {
            return Ok(Arc::clone(report));
        }
        let validation = self.validate()?;
        let build = self.meta.build()?;
        Ok(Arc::clone(self.availability.get_or_init(|| {
            Arc::new(availability_of(&build, &validation, &cannot_read))
        })))
    }
}
