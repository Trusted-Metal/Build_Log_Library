//! The source of a build: its metadata, and its samples (IP-13 decision D13-8 and phase 13L; moved to the toolkit by IP-12 phase 12A, decision D12-1).
//!
//! It holds the metadata that the reader built and a locator that knows where the samples are (which holds the mount the build was read from, so the file stays open for the life of the
//! source: PS-10). **A request for a column is answered by the column engine** (`crate::columns`), through the version's locator, which says which dataset holds the column. A version that
//! also has events or images wraps this source and answers those requests itself; every other method is as the traits say.

use std::sync::{Arc, OnceLock};

use build_log_model::{
    availability_of, validate_build, AvailabilityReport, BuildMeta, BuildSource, CacheStats,
    ChannelDType, ChannelDescriptor, ColumnData, ColumnRequest, ColumnSource, Error, GridData,
    GridRequest, ReadContext, Revision, ValidationReport,
};

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
    build: Arc<BuildMeta>,
    columns: ColumnReader<L>,
    // the two reports of IP-07 are a function of the metadata, which does not change while the file is open (D06-7), so each is made once and shared (D07-8, BL-FA-11)
    validation: OnceLock<Arc<ValidationReport>>,
    availability: OnceLock<Arc<AvailabilityReport>>,
}

impl<L: Locator> SampleSource<L> {
    /// A source of `build`, whose samples `locator` finds.
    pub fn new(build: Arc<BuildMeta>, locator: L) -> Self {
        SampleSource {
            build,
            columns: ColumnReader::new(locator),
            validation: OnceLock::new(),
            availability: OnceLock::new(),
        }
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
        Ok(Arc::clone(&self.build))
    }

    fn revision(&self) -> Result<Revision, Error> {
        // a file does not grow while it is open for reading
        Ok(Revision::default())
    }

    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        Ok(Arc::clone(
            self.validation
                .get_or_init(|| Arc::new(validate_build(&self.build))),
        ))
    }

    fn availability(&self) -> Result<Arc<AvailabilityReport>, Error> {
        let validation = self.validate()?;
        Ok(Arc::clone(self.availability.get_or_init(|| {
            Arc::new(availability_of(&self.build, &validation, &cannot_read))
        })))
    }
}
