//! Which kind of file a path is, from its first bytes and its extension.

use std::path::Path;
use std::sync::Arc;

use build_log_archive::{FileArchive, Sniff};
use build_log_model::{ContainerKind, Error, ErrorKind};

/// What was found: the kind of file and, for an archive, the archive, opened and kept (see `GuardedFile` of the archive layer) so that nothing changes under the source that is read from it.
#[derive(Debug, Clone)]
pub struct Detected {
    pub kind: ContainerKind,
    /// The open archive: `Some` for `ContainerKind::Archive`.
    pub archive: Option<Arc<FileArchive>>,
}

/// The kind of file that the extension of `path` names: `.lbl` is an archive, `.h5` and `.hdf5` (in any case) are bare HDF5 files, and any other extension, or none, names nothing.
pub fn extension_kind(path: &Path) -> Option<ContainerKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "lbl" => Some(ContainerKind::Archive),
        "h5" | "hdf5" => Some(ContainerKind::BareHdf5),
        _ => None,
    }
}

fn wrong(found: ContainerKind, expected: ContainerKind, detail: String) -> Error {
    Error::new(ErrorKind::WrongContainer {
        found,
        expected,
        detail,
    })
}

/// Looks at the file at `path`. `None` when it cannot say (the file cannot be opened or read, or it is neither HDF5 nor a ZIP file or a working file): the caller then opens it as HDF5, which reports what is wrong in the library's usual terms.
///
/// For an archive the file is opened and its directory read, which is how an unsealed file (`Unsealed`) and a damaged one (`Corrupt`) are told. The HDF5 member must start where HDF5 itself looks for a superblock (an archive that this library
/// sealed does), because a stock HDF5 opens the file; otherwise it is `UnsupportedInVersion`.
pub fn detect(path: &Path) -> Result<Option<Detected>, Error> {
    let Ok(file) = std::fs::File::open(path) else {
        return Ok(None);
    };
    let Ok(sniff) = Sniff::of(&file) else {
        return Ok(None);
    };
    drop(file);
    let by_extension = extension_kind(path);
    match sniff {
        Sniff::Hdf5 => {
            if by_extension == Some(ContainerKind::Archive) {
                return Err(wrong(
                    ContainerKind::BareHdf5,
                    ContainerKind::Archive,
                    format!(
                        "`{}` is named like an .lbl archive and holds a bare HDF5 file; an .lbl is an archive, and a bare HDF5 file is .h5 or .hdf5",
                        path.display()
                    ),
                ));
            }
            Ok(Some(Detected {
                kind: ContainerKind::BareHdf5,
                archive: None,
            }))
        }
        Sniff::Archive | Sniff::WorkingFile => {
            if by_extension == Some(ContainerKind::BareHdf5) {
                return Err(wrong(
                    ContainerKind::Archive,
                    ContainerKind::BareHdf5,
                    format!(
                        "`{}` is named like a bare HDF5 file and holds an .lbl archive; an archive is .lbl",
                        path.display()
                    ),
                ));
            }
            let archive = FileArchive::open_path(path)?;
            if archive.primary_userblock().is_none() {
                return Err(Error::new(ErrorKind::UnsupportedInVersion {
                    version: "archive".to_string(),
                    feature: format!(
                        "reading the HDF5 member `{}`, which starts at byte {}: a stock HDF5 library finds an HDF5 file only at byte 0 or at a power of two from 512",
                        archive.primary().name,
                        archive.primary().data_offset
                    ),
                }));
            }
            Ok(Some(Detected {
                kind: ContainerKind::Archive,
                archive: Some(Arc::new(archive)),
            }))
        }
        Sniff::Unknown => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_names_a_kind_in_any_case_and_other_extensions_name_none() {
        let kind = |name: &str| extension_kind(Path::new(name));
        assert_eq!(kind("build.lbl"), Some(ContainerKind::Archive));
        assert_eq!(kind("BUILD.LBL"), Some(ContainerKind::Archive));
        assert_eq!(kind("old.h5"), Some(ContainerKind::BareHdf5));
        assert_eq!(kind("old.HDF5"), Some(ContainerKind::BareHdf5));
        assert_eq!(kind("download.part"), None);
        assert_eq!(kind("no_extension"), None);
        assert_eq!(kind("dir.lbl/file"), None);
    }
}
