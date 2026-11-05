//! Opening a file (IP-03 phase 3G, task T03.10): the libhdf5 minimum is checked **first**, so that an old library gives a typed error that names both
//! versions and not a puzzling failure later, and a failure to open is mapped into the library's errors ([`super::error::from_binding`]).

use std::path::Path;

use build_log_model::Error;
use hdf5::File;

use super::error::from_binding;
use crate::version::{check_libhdf5, libhdf5_version, Libhdf5Version, LIBHDF5_MINIMUM};

/// Opens a file for reading, after checking that the linked libhdf5 is at least [`LIBHDF5_MINIMUM`].
pub fn open_file(path: impl AsRef<Path>) -> Result<File, Error> {
    open_file_with_minimum(path, LIBHDF5_MINIMUM)
}

/// [`open_file`] with the minimum given, so that a test can ask for one that the linked libhdf5 does not meet. The check comes before the file is touched: a
/// libhdf5 that is too old is reported even if the path is wrong.
pub fn open_file_with_minimum(
    path: impl AsRef<Path>,
    minimum: Libhdf5Version,
) -> Result<File, Error> {
    check_libhdf5(libhdf5_version(), minimum)?;
    File::open(path.as_ref()).map_err(|e| from_binding("opening a file", e))
}
