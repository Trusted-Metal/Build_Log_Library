//! An archive on disk: the file is kept open so that nothing changes under it, and can be asked whether the path still names it.
//!
//! **Why.** The offsets of an archive and everything HDF5 has cached about its member are only true of the bytes that were there when the file was opened. A file that is changed in place under an open reader would give
//! results that mix two versions. So:
//!
//! * **Windows:** the file is opened with a sharing mode that **excludes writers** (readers and deleters are allowed). A writer that already has the file makes the open fail (a transient I/O error), and a writer that comes later
//!   fails instead of changing the file. Replacing a file by renaming a new one over it is how a new version is published; the open handle keeps reading the old file.
//! * **Every system:** [`GuardedFile::check`] compares the open handle with the path and with what it was when opened, and says `Unchanged`, `Superseded` (the path now names another file: the open handle is still consistent), `Removed`, or `Modified`
//!   (the bytes under the handle changed: whatever was read from it is unreliable). The check is cheap: two `stat`-like calls.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use build_log_model::Error;

use crate::archive::Archive;
use crate::read_at::ReadAt;
use crate::zip::io_error;

/// Whether the file that was opened is still the file at its path, unchanged.
// closed: what can be true of an open file's path and bytes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// The path names the file that was opened, and it is as it was.
    Unchanged,
    /// The path now names another file (a new version was put in its place). The open handle still reads a consistent, older file.
    Superseded,
    /// There is no file at the path any more. The open handle still reads what it read.
    Removed,
    /// The bytes under the open handle changed. What was read from it is not reliable: open the file again.
    Modified,
}

/// A file opened for reading, kept as it was opened.
#[derive(Debug)]
pub struct GuardedFile {
    file: File,
    path: PathBuf,
    identity: same_file::Handle,
    len: u64,
    modified: Option<SystemTime>,
}

impl GuardedFile {
    /// Opens `path` for reading. On Windows other writers are excluded for as long as it is open; a writer that has it already makes this fail with a transient I/O error.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let what = format!("opening `{}`", path.display());
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_SHARE_READ | FILE_SHARE_DELETE: readers and a delete or an atomic replace are allowed, writers are not
            options.share_mode(0x1 | 0x4);
        }
        let file = options.open(path).map_err(|e| io_error(&what, &e))?;
        let metadata = file.metadata().map_err(|e| io_error(&what, &e))?;
        let identity =
            same_file::Handle::from_file(file.try_clone().map_err(|e| io_error(&what, &e))?)
                .map_err(|e| io_error(&what, &e))?;
        Ok(GuardedFile {
            file,
            path: path.to_path_buf(),
            identity,
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }

    /// The path it was opened by.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the bytes under the open handle changed (its size or modification time differ from when it was opened). One call on the handle and nothing else, so it is cheap enough to ask before every query; it does not look at the path.
    pub fn is_modified(&self) -> bool {
        match self.file.metadata() {
            Ok(m) => m.len() != self.len || m.modified().ok() != self.modified,
            Err(_) => true,
        }
    }

    /// Compares the open file with what it was and with the path.
    pub fn check(&self) -> FileState {
        if self.is_modified() {
            return FileState::Modified;
        }
        match same_file::Handle::from_path(&self.path) {
            Ok(now) if now == self.identity => FileState::Unchanged,
            Ok(_) => FileState::Superseded,
            Err(e) if e.kind() == io::ErrorKind::NotFound => FileState::Removed,
            // the path cannot be looked at (permissions, a share): it is not the file that was opened, as far as anyone can tell
            Err(_) => FileState::Superseded,
        }
    }
}

impl ReadAt for GuardedFile {
    fn len(&self) -> io::Result<u64> {
        Ok(self.len)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read_at(offset, buf)
    }
}

/// An archive on disk.
pub type FileArchive = Archive<GuardedFile>;

impl Archive<GuardedFile> {
    /// Opens the archive at `path`, keeping the file as described for [`GuardedFile`].
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        Archive::open(GuardedFile::open(path)?)
    }

    /// The path the archive was opened by.
    pub fn path(&self) -> &Path {
        self.reader().path()
    }

    /// Whether the file is still the one that was opened, unchanged.
    pub fn check(&self) -> FileState {
        self.reader().check()
    }

    /// Whether the bytes under the open file changed: the cheap form of [`Self::check`], for before every query.
    pub fn is_modified(&self) -> bool {
        self.reader().is_modified()
    }
}
