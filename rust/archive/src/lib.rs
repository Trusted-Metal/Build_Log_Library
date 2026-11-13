//! The **archive layer**: the container of a Build Log from V18 on. An `.lbl` file is a ZIP archive whose members are **stored, never compressed**, so that every byte of every member is at a known place in the file
//! and can be read without reading anything else. One member is the HDF5 file with the build (`build.h5`); the others (the machine configuration, logs, images) are opaque to this library.
//!
//! * **Layer:** the container, between the Model and the IO layer: the IO layer opens an archive through it, and it knows nothing of HDF5 or of any version's layout. It builds for `wasm32` without its file-system part.
//! * **May depend on:** `build-log-model`, for the library's errors, and nothing else of this workspace. **No HDF5**: the HDF5 member is found, sized and sealed here as bytes.
//! * **Who depends on it:** the IO layer (opening and sealing archives), and through it the `build-log` facade.
//!
//! # Why this works without a custom reader of the HDF5 file
//!
//! HDF5 looks for its superblock at byte 0 and then at 512, 1024, 2048, 4096 and so on, and treats the bytes before it as a *userblock* that it ignores. The archive puts the HDF5 member's data at byte **4096** ([`USERBLOCK`]),
//! so a stock HDF5 library (1.10.8 to 2.x) opens the `.lbl` file itself, at its usual speed, and reads the build from the first member. The ZIP headers of the first two members live in those 4096 bytes.
//!
//! # The life of an archive
//!
//! 1. **Working file.** A writer creates the HDF5 file *at its final path* with a userblock of [`USERBLOCK`] bytes (the IO layer's `create_working_file`) and writes the build into it, growing it as the build runs. It is an HDF5 file and not yet an archive.
//!    [`open`](FileArchive::open) says so with `Unsealed`.
//! 2. **Seal.** [`seal`] reads the HDF5 member once (for its CRC-32), writes the ZIP headers into the userblock, and appends the other members, the manifest and the central directory. The directory is written **last**: a seal that is
//!    interrupted leaves a working file, never a damaged archive. A sealed archive is immutable.
//! 3. **Read.** [`FileArchive::open`] reads the central directory only. Members are read by range ([`Archive::read_member_at`]), and [`Archive::verify`] checks a member's CRC-32 when asked (it reads the whole member, so it is not done on every query).
//! 4. **Unseal and seal again**, to change the HDF5 member of a sealed archive: HDF5 writes at the end of its own data, which is where an archive keeps its other members, so [`unseal`] first saves them (in a side file, so that an
//!    interruption loses nothing) and cuts the file back to the HDF5 data, and [`seal`] puts them back.
//!
//! **Modification while open.** A [`FileArchive`] keeps its file open so that nothing changes under it: on Windows it denies other writers (a writer that already has the file makes the open fail with a transient I/O error, and so does a seal or an unseal while a
//! reader has it), and on every system [`FileArchive::check`] says whether the path still names the same, unmodified file: `Unchanged`, `Superseded` (replaced by a new version; what is open stays consistent), `Removed` or `Modified`.
//!
//! The format is documented in `docs/archive.md`.

#![forbid(unsafe_code)]

mod archive;
mod manifest;
mod member;
mod read_at;
#[cfg(not(target_arch = "wasm32"))]
mod time;
mod zip;

#[cfg(not(target_arch = "wasm32"))]
mod file;
#[cfg(not(target_arch = "wasm32"))]
mod seal;

pub use archive::{
    Archive, Sniff, DEFAULT_PRIMARY, MACHINE_CONFIG_MEMBER, MIMETYPE, MIMETYPE_MEMBER, USERBLOCK,
};
#[cfg(not(target_arch = "wasm32"))]
pub use file::{FileArchive, FileState, GuardedFile};
pub use manifest::{Manifest, CONTAINER_VERSION, MANIFEST_MEMBER};
pub use member::Member;
pub use read_at::ReadAt;
#[cfg(not(target_arch = "wasm32"))]
pub use seal::{pack, seal, unseal, Detached, MemberSource, SealMember, SealOptions, SealReport};
