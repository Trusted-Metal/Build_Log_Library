//! The working file of a build that will be an archive.

use std::path::Path;

use build_log_archive::USERBLOCK;
use build_log_model::Error;
use hdf5::File;

use crate::hdf5::from_binding;
use crate::version::{check_libhdf5, libhdf5_version, Libhdf5Version, LIBHDF5_MINIMUM};

/// Creates the working file of a build whose version lives in an `.lbl` archive: an HDF5 file at `path`, **the final path of the archive**, with a userblock of [`USERBLOCK`] bytes in which the archive's headers will go when it is sealed.
/// The writer writes the build into the returned file, growing it as the build runs, and closes it (drops it) when the build is done; [`super::seal`] then makes the file an archive in place, with no copy.
///
/// The file is created **exclusively**: a path that exists is an error and nothing is overwritten. The libhdf5 minimum is checked first, as for any file.
pub fn create_working_file(path: impl AsRef<Path>) -> Result<File, Error> {
    create_working_file_with_minimum(path, LIBHDF5_MINIMUM)
}

/// [`create_working_file`] with the minimum given, so that a test can ask for one that the linked libhdf5 does not meet.
pub fn create_working_file_with_minimum(
    path: impl AsRef<Path>,
    minimum: Libhdf5Version,
) -> Result<File, Error> {
    check_libhdf5(libhdf5_version(), minimum)?;
    let mut builder = File::with_options();
    builder.create_plist().userblock(USERBLOCK);
    builder
        .create_excl(path.as_ref())
        .map_err(|e| from_binding("creating a working file", e))
}
