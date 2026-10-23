//! The source of a V17 build before its samples can be read (IP-13 decision D13-8).
//!
//! It holds the metadata that the reader built and a clone of the mount's group handle, so that the file stays open and IP-06 can read from it. A request for
//! samples answers with a typed error that says what is missing and why, **not** `NotPresent`: the data is in the file, and the library does not read it yet.
//! Phase 13L replaces the answer.

use std::sync::Arc;

use build_log_model::{
    BuildMeta, BuildSource, ColumnData, ColumnRequest, ColumnSource, Error, ErrorKind, Location,
    ReadContext, Revision,
};
use hdf5::Group;

use super::KEY;

/// A V17 build: its metadata, and the mount it was read from.
pub struct V17Source {
    build: Arc<BuildMeta>,
    // held so that the file stays open until the source is dropped; the columns of IP-06 will read through it
    _mount: Group,
}

impl V17Source {
    pub fn new(build: BuildMeta, mount: Group) -> Self {
        V17Source {
            build: Arc::new(build),
            _mount: mount,
        }
    }
}

impl ColumnSource for V17Source {
    fn read(&self, request: &ColumnRequest, _context: &ReadContext) -> Result<ColumnData, Error> {
        let mut location = Location::new()
            .with_layer(request.layer)
            .with_channel(request.channel.clone())
            .with_operation("read");
        if let Some(laser) = &request.laser {
            location = location.with_laser(laser.clone());
        }
        Err(Error::new(ErrorKind::UnsupportedInVersion {
            version: KEY.to_string(),
            feature: "reading samples (arrives with IP-06)".to_string(),
        })
        .at(location))
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
}
