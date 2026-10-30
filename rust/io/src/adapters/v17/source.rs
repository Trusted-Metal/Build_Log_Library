//! The source of a V17 build: its metadata, and its samples (IP-13 decision D13-8, and phase 13L).
//!
//! It holds the metadata that the reader built and the mount's group handle, so that the file stays open for the life of the source (PS-10). **A request for a column is
//! answered by the column engine** (`crate::columns`), through the locator of this folder ([`super::columns`]), which says which dataset holds the column. Until phase 13L the
//! answer was a typed error that said the samples were not read yet (decision D13-8).

use std::sync::{Arc, OnceLock};

use build_log_model::{
    availability_of, validate_build, AvailabilityReport, BuildMeta, BuildSource, CacheStats,
    ChannelDType, ChannelDescriptor, ColumnData, ColumnRequest, ColumnSource, Error, EventColumns,
    EventRequest, ImageData, ImageRequest, ReadContext, Revision, ValidationReport,
};
use hdf5::Group;

use super::columns::V17Locator;
use super::event_columns;
use super::image_regions;
use crate::columns::{can_read, ColumnReader};

/// A V17 build: its metadata, and the reader of its samples: the locator that knows where they are (which holds the mount the build was read from, so the file stays open)
/// and the cache of the blocks it has decoded.
pub struct V17Source {
    build: Arc<BuildMeta>,
    columns: ColumnReader<V17Locator>,
    // the two reports of IP-07 are a function of the metadata, which does not change while the file is open (D06-7), so each is made once and shared (D07-8, BL-FA-11)
    validation: OnceLock<Arc<ValidationReport>>,
    availability: OnceLock<Arc<AvailabilityReport>>,
}

/// What this source cannot read of a channel, **from the one list of what the column engine reads** (`columns::can_read`, IP-07 phase 6I, decision D07-3): the feature that is missing, or `None`.
/// A channel whose type is a number is asked; the laser state is an enum, which does not say the width it is stored in, and is not (the files that we hold store it as `uint8`).
fn cannot_read(channel: &ChannelDescriptor) -> Option<String> {
    match &channel.dtype {
        ChannelDType::Numeric(numeric) if !can_read(*numeric) => {
            Some(format!("a column stored as {numeric:?}"))
        }
        _ => None,
    }
}

impl V17Source {
    pub fn new(build: BuildMeta, mount: Group) -> Self {
        let build = Arc::new(build);
        V17Source {
            columns: ColumnReader::new(V17Locator::new(Arc::clone(&build), mount)),
            build,
            validation: OnceLock::new(),
            availability: OnceLock::new(),
        }
    }
}

impl ColumnSource for V17Source {
    fn read(&self, request: &ColumnRequest, context: &ReadContext) -> Result<ColumnData, Error> {
        self.columns.read(request, context)
    }

    fn read_events(
        &self,
        request: &EventRequest,
        context: &ReadContext,
    ) -> Result<EventColumns, Error> {
        event_columns::read_events(&self.columns, request, context)
            .map_err(|error| error.at(request.location()))
    }

    fn read_image(
        &self,
        request: &ImageRequest,
        context: &ReadContext,
    ) -> Result<ImageData, Error> {
        image_regions::read_image(self.columns.locator(), request, context)
            .map_err(|error| error.at(request.location()))
    }

    fn cache_stats(&self) -> Option<CacheStats> {
        Some(self.columns.stats())
    }
}

impl BuildSource for V17Source {
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
