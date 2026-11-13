//! The container of a build log file (V18 on): what kind of file it is, the guard that keeps an archive from changing under an open source, and the working file that a writer creates.
//!
//! * **[`detect`]** looks at a file's first bytes and its extension and says which kind it is: **a bare HDF5 file** (`.h5`, `.hdf5`; V17 and earlier) or **an `.lbl` archive** (V18 and later). An extension that says one and a content that
//!   is the other is `WrongContainer`; an extension that says neither (a download in progress, a temporary name) lets the content decide. The *version's* container is checked later, when the marker has named the version: that is data of the
//!   version's adapter (`AdapterSet::container`), not code here.
//! * **[`GuardedSource`]** wraps the source of a build that was read from an archive. Before every call it asks the open archive whether the file changed in place, and a changed file is `FileChanged` from then on.
//! * **[`create_working_file`]** is what a writer calls to start a build of a version that lives in an archive: an HDF5 file at its final path, with the userblock that the archive's headers will fill. **[`seal`]** turns it into the archive when the
//!   build is done, and **[`unseal`]** turns an archive back into a working file for a pass that writes to the HDF5 data.
//!
//! Reading needs no help from here: the HDF5 member starts at the userblock, so the stock HDF5 library opens the archive's file itself.

mod detect;
mod guarded;
mod working;

pub use detect::{detect, extension_kind, Detected};
pub use guarded::GuardedSource;
pub use working::{create_working_file, create_working_file_with_minimum};

pub use build_log_archive::{
    pack, seal, unseal, Detached, FileArchive, FileState, Manifest, Member, MemberSource,
    SealMember, SealOptions, SealReport, Sniff, DEFAULT_PRIMARY, MACHINE_CONFIG_MEMBER, USERBLOCK,
};
