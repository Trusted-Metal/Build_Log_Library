//! An archive opened for reading: its members, its manifest, and range reads of a stored member.

use std::io;

use build_log_model::{Error, ErrorKind, Violation};

use crate::manifest::{Manifest, CONTAINER_VERSION, MANIFEST_MEMBER};
use crate::member::Member;
use crate::read_at::ReadAt;
use crate::zip::{corrupt, io_error, read_members, SIG_LOCAL};

/// The size of the userblock of a working file, and so where the HDF5 data starts in an archive: 4096. HDF5 looks for its superblock at 0 and then at every power of two from 512, so a stock HDF5 opens the archive at this offset by itself.
pub const USERBLOCK: u64 = 4096;
/// The default name of the HDF5 member.
pub const DEFAULT_PRIMARY: &str = "build.h5";
/// The name that the machine configuration member is given.
pub const MACHINE_CONFIG_MEMBER: &str = "machine_config.h5";

/// The content of the `mimetype` member: the first member of an archive, so that a tool can tell the type of the file from its first bytes.
pub const MIMETYPE: &str = "application/vnd.lbl+zip";
/// The name of the `mimetype` member.
pub const MIMETYPE_MEMBER: &str = "mimetype";

/// The signature of an HDF5 file.
pub(crate) const HDF5_SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n'];

/// The largest manifest that is believed.
const MANIFEST_LIMIT: u64 = 1 << 20;

/// What a file is, from its first bytes (and, for the working file of a build, the bytes at [`USERBLOCK`]).
// closed: the four things a file of this library can be, and what is left over
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniff {
    /// An HDF5 file: its signature is at byte 0.
    Hdf5,
    /// A ZIP file: a sealed archive, or one whose seal was interrupted (opening it tells which).
    Archive,
    /// A working file: zeros where an archive's headers go, and an HDF5 file at byte [`USERBLOCK`].
    WorkingFile,
    /// None of these.
    Unknown,
}

impl Sniff {
    /// Looks at the first bytes of `reader`.
    pub fn of<R: ReadAt + ?Sized>(reader: &R) -> io::Result<Sniff> {
        let mut head = [0u8; 8];
        let n = reader.read_at(0, &mut head)?;
        if n >= 8 && head == HDF5_SIGNATURE {
            return Ok(Sniff::Hdf5);
        }
        if n >= 4 && head[..4] == SIG_LOCAL.to_le_bytes() {
            return Ok(Sniff::Archive);
        }
        let mut at_userblock = [0u8; 8];
        let m = reader.read_at(USERBLOCK, &mut at_userblock)?;
        if m >= 8 && at_userblock == HDF5_SIGNATURE {
            return Ok(Sniff::WorkingFile);
        }
        Ok(Sniff::Unknown)
    }
}

/// A sealed archive: the directory of its members, and the reader that reads their bytes. Opening reads the central directory, the `mimetype` and the manifest, and nothing else, so it costs the same for an archive of a megabyte and of a terabyte.
pub struct Archive<R: ReadAt> {
    reader: R,
    members: Vec<Member>,
    manifest: Manifest,
    primary: usize,
}

impl<R: ReadAt> std::fmt::Debug for Archive<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("members", &self.members.len())
            .field("primary", &self.members[self.primary].name)
            .finish_non_exhaustive()
    }
}

impl<R: ReadAt> Archive<R> {
    /// Opens an archive. `Unsealed` if the file is the working file of a build, or a seal that was interrupted; `Corrupt` if it is damaged or is a ZIP file that is not an `.lbl`; `UnsupportedInVersion` for an archive of a newer
    /// format, or whose HDF5 member is compressed.
    pub fn open(reader: R) -> Result<Self, Error> {
        let Some(members) = read_members(&reader)? else {
            return Err(match Sniff::of(&reader).map_err(|e| io_error("reading the start of the file", &e))? {
                Sniff::WorkingFile => Error::new(ErrorKind::Unsealed {
                    detail: format!("the file holds an HDF5 file at byte {USERBLOCK} and no archive directory: it is the working file of a build"),
                }),
                Sniff::Archive => Error::new(ErrorKind::Unsealed {
                    detail: "the file starts like an archive and has no end records: sealing it was interrupted".to_string(),
                }),
                Sniff::Hdf5 => corrupt("the file is an HDF5 file, not an archive"),
                Sniff::Unknown => corrupt("the file is not an archive: it has no ZIP end records"),
            });
        };
        let mut archive = Archive {
            reader,
            members,
            manifest: Manifest::default(),
            primary: 0,
        };
        if let Some(mime) = archive.find(MIMETYPE_MEMBER).cloned() {
            let text = archive.read_member(&mime, 256)?;
            if text != MIMETYPE.as_bytes() {
                return Err(corrupt(
                    "the file is a ZIP archive but not an .lbl archive: its `mimetype` is another",
                ));
            }
        }
        if let Some(entry) = archive.find(MANIFEST_MEMBER).cloned() {
            let bytes = archive.read_member(&entry, MANIFEST_LIMIT)?;
            archive.manifest = Manifest::from_json(&bytes)?;
        }
        if archive.manifest.container_version > CONTAINER_VERSION {
            return Err(Error::new(ErrorKind::UnsupportedInVersion {
                version: format!("archive format {}", archive.manifest.container_version),
                feature: format!("reading an archive newer than the format {CONTAINER_VERSION} this library reads"),
            }));
        }
        archive.primary = archive.choose_primary()?;
        let primary = &archive.members[archive.primary];
        if !primary.stored {
            return Err(Error::new(ErrorKind::UnsupportedInVersion {
                version: format!("archive format {}", archive.manifest.container_version),
                feature: format!("reading `{}`, which is compressed or encrypted: the HDF5 member of an archive is stored", primary.name),
            }));
        }
        Ok(archive)
    }

    fn choose_primary(&self) -> Result<usize, Error> {
        let named = |name: &str| {
            self.members
                .iter()
                .position(|m| m.name == name)
                .ok_or_else(|| {
                    Error::new(ErrorKind::ValidationError {
                        violations: vec![Violation::new(format!("the manifest names `{name}` as the build's HDF5 member and the archive has none"))],
                    })
                })
        };
        match &self.manifest.primary {
            Some(name) => named(name),
            None => self
                .members
                .iter()
                .position(|m| m.name.ends_with(".h5") && m.name != MACHINE_CONFIG_MEMBER)
                .ok_or_else(|| corrupt("the archive has no HDF5 member")),
        }
    }

    /// The reader the archive reads through.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// The members, in the order of the directory (the HDF5 member first in an archive this library wrote).
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// A member by its name.
    pub fn find(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name)
    }

    /// What the manifest says (the defaults, if the archive has none).
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The member that holds the build's HDF5 file.
    pub fn primary(&self) -> &Member {
        &self.members[self.primary]
    }

    /// Where HDF5 finds the superblock of the primary member **by itself**, with no help from this library, if it does: `Some(offset)` when the member starts at byte 0 or at a power of two of 512 or more, the offsets at which
    /// HDF5 looks. The members of an archive this library sealed start at [`USERBLOCK`]. `None`: only a reader that maps the member (a virtual file driver) can open it.
    pub fn primary_userblock(&self) -> Option<u64> {
        let at = self.primary().data_offset;
        (at == 0 || (at >= 512 && at.is_power_of_two())).then_some(at)
    }

    /// The machine configuration file's member, if the archive has one: the manifest's `machine_config`, or else a member named `machine_config.h5`. This library never reads it; it is for the machine configuration library.
    pub fn machine_config(&self) -> Option<&Member> {
        match &self.manifest.machine_config {
            Some(name) => self.find(name),
            None => self.find(MACHINE_CONFIG_MEMBER),
        }
    }

    /// Reads up to `buf.len()` bytes of `member` from `offset` within it; fewer only at its end (zero at the end, an `InvalidIndex` past it). Only a stored member can be read this way.
    pub fn read_member_at(
        &self,
        member: &Member,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, Error> {
        if !member.stored {
            return Err(Error::new(ErrorKind::UnsupportedInVersion {
                version: "archive".to_string(),
                feature: format!(
                    "reading `{}` by range: it is compressed or encrypted",
                    member.name
                ),
            }));
        }
        if offset > member.size {
            return Err(Error::new(ErrorKind::InvalidIndex {
                what: format!(
                    "byte {offset} of `{}`, which has {} bytes",
                    member.name, member.size
                ),
            }));
        }
        let n = (buf.len() as u64).min(member.size - offset) as usize;
        self.reader
            .read_exact_at(member.data_offset + offset, &mut buf[..n])
            .map_err(|e| io_error(&format!("reading `{}`", member.name), &e))?;
        Ok(n)
    }

    /// The whole member, if it is no larger than `limit` bytes (`BudgetExceeded` otherwise, so that a caller never reads a terabyte into memory by mistake).
    pub fn read_member(&self, member: &Member, limit: u64) -> Result<Vec<u8>, Error> {
        if member.size > limit {
            return Err(Error::new(ErrorKind::BudgetExceeded {
                limit_bytes: limit,
                needed_bytes: member.size,
            }));
        }
        let mut bytes = vec![0u8; member.size as usize];
        let n = self.read_member_at(member, 0, &mut bytes)?;
        bytes.truncate(n);
        Ok(bytes)
    }

    /// Checks a member's bytes against the CRC-32 that the archive records. It **reads the whole member**: it is for ingest and archival, not for every query.
    pub fn verify(&self, member: &Member) -> Result<(), Error> {
        let mut hasher = crc32fast::Hasher::new();
        let mut buffer = vec![0u8; 8 << 20];
        let mut done = 0u64;
        while done < member.size {
            let n = self.read_member_at(member, done, &mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            done += n as u64;
        }
        let found = hasher.finalize();
        if found != member.crc32 {
            return Err(corrupt(format!(
                "`{}` does not match its CRC-32: the archive records {:08x} and the bytes give {found:08x}",
                member.name, member.crc32
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_told_from_its_first_bytes() {
        let mut hdf5 = HDF5_SIGNATURE.to_vec();
        hdf5.extend_from_slice(&[0; 32]);
        assert_eq!(Sniff::of(&hdf5).unwrap(), Sniff::Hdf5);
        let mut zip = SIG_LOCAL.to_le_bytes().to_vec();
        zip.extend_from_slice(&[0; 32]);
        assert_eq!(Sniff::of(&zip).unwrap(), Sniff::Archive);
        let mut working = vec![0u8; USERBLOCK as usize];
        working.extend_from_slice(&HDF5_SIGNATURE);
        assert_eq!(Sniff::of(&working).unwrap(), Sniff::WorkingFile);
        assert_eq!(Sniff::of(&b"hello"[..]).unwrap(), Sniff::Unknown);
        assert_eq!(Sniff::of(&Vec::<u8>::new()).unwrap(), Sniff::Unknown);
    }
}
