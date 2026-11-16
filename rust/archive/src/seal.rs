//! Sealing a working file into an archive, and unsealing a sealed one to change its HDF5 member.
//!
//! A **working file** is an HDF5 file created at its final path with a userblock of [`USERBLOCK`] bytes (the IO layer's `create_working_file`): the build is written into it, growing it, by HDF5 itself, with no copy. **Sealing** it makes it an
//! archive in place, in this order, so that an interruption leaves a working file and never a damaged archive:
//!
//! 1. find where the HDF5 data ends (the end-of-file address in its superblock), and cut off whatever is after it (the leftovers of a seal that was interrupted);
//! 2. read the HDF5 data once, for its CRC-32;
//! 3. write the ZIP headers of the first two members (`mimetype` and the HDF5 member) into the userblock, which is just big enough for them;
//! 4. append the other members and the manifest;
//! 5. write the central directory and the end records, **last**: the file is an archive from the moment they are complete.
//!
//! **Unsealing** exists because HDF5 writes new data where an archive keeps its other members (at the end of its own data), so an HDF5 library that opens a sealed archive for writing would overwrite them (even to change one attribute). [`unseal`]
//! reads the other members, writes them to a side file next to the archive (`<name>.detached.zip`, atomically), and cuts the file back to the HDF5 data: a working file again. When the HDF5 file has been changed, [`seal`] with the options that
//! [`Detached::to_options`] gives puts them back, and [`Detached::finish`] removes the side file. If the process dies in between, the side file is still there, and [`Detached::load`] reads it.

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use build_log_model::{Error, ErrorKind, Violation};

use crate::archive::{
    Archive, DEFAULT_PRIMARY, HDF5_SIGNATURE, MACHINE_CONFIG_MEMBER, MIMETYPE, MIMETYPE_MEMBER,
    USERBLOCK,
};
use crate::manifest::{Manifest, MANIFEST_MEMBER};
use crate::member::Member;
use crate::read_at::ReadAt;
use crate::time::{dos, iso8601, now_unix};
use crate::zip::{
    central_entry, corrupt, end_records, io_error, local_header, read_members, HeaderSpec,
};

/// Where the data of every member after the first starts, a multiple of: it makes reads of a member line up with the disk's blocks.
const ALIGN: u64 = 4096;
const COPY_BUFFER: usize = 8 << 20;

/// Where the bytes of a member to be sealed in come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberSource {
    /// Bytes in memory.
    Bytes(Vec<u8>),
    /// A file, copied in as it is when the archive is sealed.
    File(PathBuf),
}

/// A member to be sealed in, other than the HDF5 member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealMember {
    /// Its name in the archive (`logs/capture.log`): no leading slash and no backslash.
    pub name: String,
    pub source: MemberSource,
}

/// What [`seal`] puts in the archive besides the HDF5 member.
#[derive(Clone)]
pub struct SealOptions {
    primary_name: String,
    machine_config: Option<MemberSource>,
    members: Vec<SealMember>,
    schema_version: Option<i64>,
    created_by: String,
    zip64: bool,
    progress: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
}

impl std::fmt::Debug for SealOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealOptions")
            .field("primary_name", &self.primary_name)
            .field("machine_config", &self.machine_config.is_some())
            .field(
                "members",
                &self.members.iter().map(|m| &m.name).collect::<Vec<_>>(),
            )
            .field("schema_version", &self.schema_version)
            .finish_non_exhaustive()
    }
}

impl Default for SealOptions {
    fn default() -> Self {
        SealOptions {
            primary_name: DEFAULT_PRIMARY.to_string(),
            machine_config: None,
            members: Vec::new(),
            schema_version: None,
            created_by: format!("build-log-archive/{}", env!("CARGO_PKG_VERSION")),
            zip64: false,
            progress: None,
        }
    }
}

impl SealOptions {
    /// Options that seal the HDF5 file and nothing else.
    pub fn new() -> Self {
        Self::default()
    }

    /// The name of the HDF5 member (default `build.h5`).
    pub fn with_primary_name(mut self, name: impl Into<String>) -> Self {
        self.primary_name = name.into();
        self
    }

    /// The machine configuration file, sealed in as `machine_config.h5` and named in the manifest. It is copied as it is, and never read by this library.
    pub fn with_machine_config(mut self, source: MemberSource) -> Self {
        self.machine_config = Some(source);
        self
    }

    /// Another member: a log, an image.
    pub fn with_member(mut self, name: impl Into<String>, source: MemberSource) -> Self {
        self.members.push(SealMember {
            name: name.into(),
            source,
        });
        self
    }

    /// The version of the build's file, for the manifest, as a hint.
    pub fn with_schema_version(mut self, version: i64) -> Self {
        self.schema_version = Some(version);
        self
    }

    /// What the manifest says wrote the archive (default `build-log-archive/<version>`).
    pub fn with_created_by(mut self, created_by: impl Into<String>) -> Self {
        self.created_by = created_by.into();
        self
    }

    /// Write the ZIP64 fields for every member, as a member of 4 GiB or more needs. For testing that readers handle them with small files.
    pub fn with_zip64(mut self, always: bool) -> Self {
        self.zip64 = always;
        self
    }

    /// Called with `(bytes done, bytes in all)` while the HDF5 data is read for its CRC and while members are copied in; a seal of a terabyte takes minutes, and an application shows it.
    pub fn with_progress(mut self, progress: impl Fn(u64, u64) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(progress));
        self
    }

    fn validate(&self) -> Result<(), Error> {
        let mut violations = Vec::new();
        let mut seen: Vec<&str> = vec![MIMETYPE_MEMBER, MANIFEST_MEMBER, &self.primary_name];
        if self.machine_config.is_some() {
            seen.push(MACHINE_CONFIG_MEMBER);
        }
        let names = std::iter::once(self.primary_name.as_str())
            .chain(self.members.iter().map(|m| m.name.as_str()));
        for name in names {
            if name.is_empty()
                || name.starts_with('/')
                || name.contains('\\')
                || name.contains("..")
                || name.ends_with('/')
            {
                violations.push(Violation::new(format!("{name:?} is not a name for a member: it needs text, with no leading slash, no backslash, no `..` and no trailing slash")));
            }
        }
        for member in &self.members {
            if seen.contains(&member.name.as_str()) {
                violations.push(Violation::new(format!(
                    "the member name {:?} is used twice, or is one that the archive reserves",
                    member.name
                )));
            }
            seen.push(&member.name);
        }
        if violations.is_empty() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::ValidationError { violations }))
        }
    }
}

/// What a seal did.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SealReport {
    /// The members of the sealed archive, as a reader sees them.
    pub members: Vec<Member>,
    /// The size of the archive file.
    pub archive_len: u64,
    /// The size of the HDF5 member.
    pub hdf5_len: u64,
    /// The CRC-32 of the HDF5 member.
    pub hdf5_crc32: u32,
    /// The bytes that were after the HDF5 data and were cut off: zero for a working file that was closed cleanly; more after an interrupted seal.
    pub discarded_bytes: u64,
}

/// A member that has been written: what the central directory needs.
struct Written {
    name: String,
    offset: u64,
    size: u64,
    crc32: u32,
    zip64: bool,
}

fn validation(message: String) -> Error {
    Error::new(ErrorKind::ValidationError {
        violations: vec![Violation::new(message)],
    })
}

fn crc_of(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

/// Where the HDF5 data starts and ends, from its superblock: the base address and the end-of-file address, both absolute offsets in the file (HDF5 writes them so, and the base address is the userblock size).
fn hdf5_extent(file: &File) -> Result<(u64, u64), Error> {
    let mut sb = [0u8; 64];
    let mut n = 0;
    while n < sb.len() {
        let got = file
            .read_at(USERBLOCK + n as u64, &mut sb[n..])
            .map_err(|e| io_error("reading the HDF5 superblock", &e))?;
        if got == 0 {
            break;
        }
        n += got;
    }
    if n < 40 || sb[..8] != HDF5_SIGNATURE {
        return Err(corrupt(format!(
            "there is no HDF5 file at byte {USERBLOCK}: this is not the working file of a build (it was not created with a userblock of {USERBLOCK} bytes)"
        )));
    }
    let word = |at: usize, size: usize| -> u64 {
        sb[at..at + size]
            .iter()
            .rev()
            .fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
    };
    let (offset_size, base_at, eoa_at) = match sb[8] {
        0 | 1 => {
            let so = usize::from(sb[13]);
            let start = if sb[8] == 1 { 28 } else { 24 };
            (so, start, start + 2 * so)
        }
        2 | 3 => {
            let so = usize::from(sb[9]);
            (so, 12, 12 + 2 * so)
        }
        other => {
            return Err(corrupt(format!(
                "the HDF5 superblock is of version {other}, which this library does not know"
            )))
        }
    };
    if ![2, 4, 8].contains(&offset_size) || eoa_at + offset_size > n {
        return Err(corrupt(format!(
            "the HDF5 superblock has an offset size of {offset_size} bytes"
        )));
    }
    Ok((word(base_at, offset_size), word(eoa_at, offset_size)))
}

struct Progress<'a> {
    callback: Option<&'a (dyn Fn(u64, u64) + Send + Sync)>,
    done: u64,
    total: u64,
}

impl Progress<'_> {
    fn add(&mut self, bytes: u64) {
        self.done += bytes;
        if let Some(callback) = self.callback {
            callback(self.done, self.total);
        }
    }
}

/// The CRC-32 of `len` bytes of the file from `start`.
fn crc_range(file: &File, start: u64, len: u64, progress: &mut Progress<'_>) -> Result<u32, Error> {
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut done = 0u64;
    while done < len {
        let want = (len - done).min(buffer.len() as u64) as usize;
        file.read_exact_at(start + done, &mut buffer[..want])
            .map_err(|e| io_error("reading the HDF5 data for its CRC-32", &e))?;
        hasher.update(&buffer[..want]);
        done += want as u64;
        progress.add(want as u64);
    }
    Ok(hasher.finalize())
}

fn spec<'a>(w: &'a Written, dos_time: u16, dos_date: u16) -> HeaderSpec<'a> {
    HeaderSpec {
        name: &w.name,
        offset: w.offset,
        size: w.size,
        crc32: w.crc32,
        force_zip64: w.zip64,
        align: 1,
        dos_time,
        dos_date,
    }
}

/// Writes a member after `pos`, with its data aligned, and returns what the directory needs. `pos` is where the file ends; it is moved to where it ends after.
fn append_member(
    file: &mut File,
    pos: &mut u64,
    name: &str,
    source: &MemberSource,
    zip64: bool,
    stamp: (u16, u16),
    progress: &mut Progress<'_>,
) -> Result<Written, Error> {
    let what = format!("writing the member `{name}`");
    let (size, known_crc) = match source {
        MemberSource::Bytes(bytes) => (bytes.len() as u64, Some(crc_of(bytes))),
        MemberSource::File(path) => (
            fs::metadata(path)
                .map_err(|e| io_error(&format!("reading the size of `{}`", path.display()), &e))?
                .len(),
            None,
        ),
    };
    let offset = *pos;
    let (header, data_offset) = local_header(&HeaderSpec {
        name,
        offset,
        size,
        crc32: known_crc.unwrap_or(0),
        force_zip64: zip64,
        align: ALIGN,
        dos_time: stamp.0,
        dos_date: stamp.1,
    });
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| io_error(&what, &e))?;
    file.write_all(&header).map_err(|e| io_error(&what, &e))?;
    let crc32 = match source {
        MemberSource::Bytes(bytes) => {
            file.write_all(bytes).map_err(|e| io_error(&what, &e))?;
            progress.add(size);
            known_crc.unwrap_or(0)
        }
        MemberSource::File(path) => {
            use std::io::Read;
            let mut input = File::open(path)
                .map_err(|e| io_error(&format!("opening `{}`", path.display()), &e))?;
            let mut hasher = crc32fast::Hasher::new();
            let mut buffer = vec![0u8; COPY_BUFFER];
            let mut copied = 0u64;
            loop {
                let n = input
                    .read(&mut buffer)
                    .map_err(|e| io_error(&format!("reading `{}`", path.display()), &e))?;
                if n == 0 {
                    break;
                }
                copied += n as u64;
                if copied > size {
                    break;
                }
                hasher.update(&buffer[..n]);
                file.write_all(&buffer[..n])
                    .map_err(|e| io_error(&what, &e))?;
                progress.add(n as u64);
            }
            if copied != size {
                return Err(validation(format!("`{}` changed size while it was copied in: {size} bytes at the start and {copied} at the end", path.display())));
            }
            let crc = hasher.finalize();
            // the CRC is only known now: it is at byte 14 of the local header
            file.seek(SeekFrom::Start(offset + 14))
                .map_err(|e| io_error(&what, &e))?;
            file.write_all(&crc.to_le_bytes())
                .map_err(|e| io_error(&what, &e))?;
            crc
        }
    };
    *pos = data_offset + size;
    Ok(Written {
        name: name.to_string(),
        offset,
        size,
        crc32,
        zip64: zip64 || size >= 0xFFFF_FFFF,
    })
}

/// Writes the central directory and the end records at `pos`.
fn write_directory(
    file: &mut File,
    pos: u64,
    written: &[Written],
    stamp: (u16, u16),
) -> Result<(), Error> {
    let mut directory = Vec::new();
    for w in written {
        directory.extend_from_slice(&central_entry(&spec(w, stamp.0, stamp.1)));
    }
    let mut out = directory.clone();
    out.extend_from_slice(&end_records(
        pos,
        directory.len() as u64,
        written.len() as u64,
    ));
    file.seek(SeekFrom::Start(pos))
        .map_err(|e| io_error("writing the central directory", &e))?;
    file.write_all(&out)
        .map_err(|e| io_error("writing the central directory", &e))?;
    Ok(())
}

fn source_len(source: &MemberSource) -> Result<u64, Error> {
    match source {
        MemberSource::Bytes(b) => Ok(b.len() as u64),
        MemberSource::File(p) => fs::metadata(p)
            .map(|m| m.len())
            .map_err(|e| io_error(&format!("reading the size of `{}`", p.display()), &e)),
    }
}

/// Seals the working file at `path` into an archive, in place. See the module's description of the order, which makes an interruption harmless.
///
/// The file must be a working file (an HDF5 file with its superblock at byte [`USERBLOCK`]) that no HDF5 handle has open, and that no other process has open: on Windows a reader that has it open makes this fail with a transient I/O error. A file
/// that is already sealed is refused; [`unseal`] it first.
pub fn seal(path: &Path, options: &SealOptions) -> Result<SealReport, Error> {
    options.validate()?;
    let what = format!("sealing `{}`", path.display());
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| io_error(&what, &e))?;
    if read_members(&file)?.is_some() {
        return Err(validation(format!(
            "`{}` is already sealed: unseal it to change it",
            path.display()
        )));
    }
    let len = file.metadata().map_err(|e| io_error(&what, &e))?.len();
    // the HDF5 data is as long as its own superblock says (end of file minus base address), and starts where its superblock is. A file created with a userblock says `base = 4096`; a bare HDF5 file that was
    // copied behind a userblock (see `pack`) says `base = 0`; both give the same length, which is what matters
    let (base, eoa) = hdf5_extent(&file)?;
    if eoa < base {
        return Err(corrupt(format!(
            "the HDF5 superblock puts the end of the data (byte {eoa}) before its start (byte {base})"
        )));
    }
    let hdf5_len = eoa - base;
    let hdf5_end = USERBLOCK + hdf5_len;
    if hdf5_end > len {
        return Err(corrupt(format!(
            "the HDF5 data is {hdf5_len} bytes by its superblock, and the file has only {} after the userblock: it is truncated",
            len.saturating_sub(USERBLOCK)
        )));
    }
    let discarded_bytes = len - hdf5_end;
    if discarded_bytes > 0 {
        file.set_len(hdf5_end).map_err(|e| io_error(&what, &e))?;
    }

    let extra: u64 = options
        .machine_config
        .iter()
        .map(source_len)
        .chain(options.members.iter().map(|m| source_len(&m.source)))
        .sum::<Result<u64, Error>>()?;
    let mut progress = Progress {
        callback: options.progress.as_deref(),
        done: 0,
        total: hdf5_len + extra,
    };
    let hdf5_crc32 = crc_range(&file, USERBLOCK, hdf5_len, &mut progress)?;

    // the userblock: the headers of `mimetype` and of the HDF5 member, padded to end exactly where the HDF5 data starts
    let stamp = dos(now_unix());
    let mimetype = Written {
        name: MIMETYPE_MEMBER.to_string(),
        offset: 0,
        size: MIMETYPE.len() as u64,
        crc32: crc_of(MIMETYPE.as_bytes()),
        zip64: false,
    };
    let (mut userblock, mime_end) = local_header(&HeaderSpec {
        name: MIMETYPE_MEMBER,
        offset: 0,
        size: mimetype.size,
        crc32: mimetype.crc32,
        force_zip64: false,
        align: 1,
        dos_time: stamp.0,
        dos_date: stamp.1,
    });
    userblock.extend_from_slice(MIMETYPE.as_bytes());
    debug_assert_eq!(userblock.len() as u64, mime_end + mimetype.size);
    let primary = Written {
        name: options.primary_name.clone(),
        offset: userblock.len() as u64,
        size: hdf5_len,
        crc32: hdf5_crc32,
        zip64: options.zip64 || hdf5_len >= 0xFFFF_FFFF,
    };
    let (header, data_offset) = local_header(&HeaderSpec {
        name: &primary.name,
        offset: primary.offset,
        size: primary.size,
        crc32: primary.crc32,
        force_zip64: primary.zip64,
        align: USERBLOCK,
        dos_time: stamp.0,
        dos_date: stamp.1,
    });
    if data_offset != USERBLOCK {
        return Err(corrupt(format!("the headers of the first two members do not fit the {USERBLOCK}-byte userblock (the HDF5 member would start at {data_offset})")));
    }
    userblock.extend_from_slice(&header);
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_error(&what, &e))?;
    file.write_all(&userblock)
        .map_err(|e| io_error(&what, &e))?;

    // the other members, the manifest, and last the directory
    let mut written = vec![mimetype, primary];
    let mut pos = hdf5_end;
    if let Some(source) = &options.machine_config {
        written.push(append_member(
            &mut file,
            &mut pos,
            MACHINE_CONFIG_MEMBER,
            source,
            options.zip64,
            stamp,
            &mut progress,
        )?);
    }
    for member in &options.members {
        written.push(append_member(
            &mut file,
            &mut pos,
            &member.name,
            &member.source,
            options.zip64,
            stamp,
            &mut progress,
        )?);
    }
    let manifest = Manifest {
        primary: Some(options.primary_name.clone()),
        machine_config: options
            .machine_config
            .as_ref()
            .map(|_| MACHINE_CONFIG_MEMBER.to_string()),
        schema_version: options.schema_version,
        attachments: options.members.iter().map(|m| m.name.clone()).collect(),
        created_by: Some(options.created_by.clone()),
        created_utc: Some(iso8601(now_unix())),
        ..Manifest::default()
    };
    written.push(append_member(
        &mut file,
        &mut pos,
        MANIFEST_MEMBER,
        &MemberSource::Bytes(manifest.to_json()),
        options.zip64,
        stamp,
        &mut progress,
    )?);
    write_directory(&mut file, pos, &written, stamp)?;
    file.sync_all().map_err(|e| io_error(&what, &e))?;

    let members = read_members(&file)?
        .ok_or_else(|| corrupt("the archive that was just sealed has no end records"))?;
    let archive_len = file.metadata().map_err(|e| io_error(&what, &e))?.len();
    Ok(SealReport {
        members,
        archive_len,
        hdf5_len,
        hdf5_crc32,
        discarded_bytes,
    })
}

// ---------------------------------------------------------------- packing an existing HDF5 file

/// Makes an archive at `dest` from an existing bare HDF5 file: a userblock of [`USERBLOCK`] zero bytes, then the HDF5 file copied as it is, then [`seal`]. For a build that was written as a bare file (a V18 file from before archives were
/// written directly, or one received), and for tests. HDF5 reads the copy at once: it finds the superblock behind the userblock, whatever base address the superblock names. `dest` is created exclusively, and removed again if the seal fails.
///
/// The source is read once for the copy and once, in the copy, for its CRC-32, so a terabyte file takes two passes; a writer that creates its file with `create_working_file` pays one.
pub fn pack(source: &Path, dest: &Path, options: &SealOptions) -> Result<SealReport, Error> {
    use std::io::Read;
    options.validate()?;
    let mut input =
        File::open(source).map_err(|e| io_error(&format!("opening `{}`", source.display()), &e))?;
    let mut head = [0u8; 8];
    input
        .read_exact(&mut head)
        .map_err(|e| io_error(&format!("reading `{}`", source.display()), &e))?;
    if head != HDF5_SIGNATURE {
        return Err(validation(format!(
            "`{}` is not an HDF5 file with its superblock at byte 0, so it cannot be packed",
            source.display()
        )));
    }
    input
        .seek(SeekFrom::Start(0))
        .map_err(|e| io_error(&format!("reading `{}`", source.display()), &e))?;
    let what = format!("packing `{}` into `{}`", source.display(), dest.display());
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|e| io_error(&what, &e))?;
    let copied = (|| -> Result<(), Error> {
        output
            .write_all(&vec![0u8; USERBLOCK as usize])
            .map_err(|e| io_error(&what, &e))?;
        let mut buffer = vec![0u8; COPY_BUFFER];
        loop {
            let n = input.read(&mut buffer).map_err(|e| io_error(&what, &e))?;
            if n == 0 {
                break;
            }
            output
                .write_all(&buffer[..n])
                .map_err(|e| io_error(&what, &e))?;
        }
        output.sync_all().map_err(|e| io_error(&what, &e))
    })();
    drop(output);
    let result = copied.and_then(|()| seal(dest, options));
    if result.is_err() {
        let _ = fs::remove_file(dest);
    }
    result
}

// ---------------------------------------------------------------- unsealing

/// The members that [`unseal`] took out of an archive, and where it kept them: a side file next to the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detached {
    primary_name: String,
    machine_config: bool,
    schema_version: Option<i64>,
    members: Vec<SealMember>,
    sidecar: PathBuf,
}

/// The side file of an archive: `<name>.detached.zip` next to it.
fn sidecar_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".detached.zip");
    PathBuf::from(name)
}

impl Detached {
    /// The side file that holds the members.
    pub fn sidecar(&self) -> &Path {
        &self.sidecar
    }

    /// The names of the members that were taken out, in the order they will be sealed back (the machine configuration, if there was one, is `machine_config.h5`).
    pub fn member_names(&self) -> Vec<&str> {
        self.members.iter().map(|m| m.name.as_str()).collect()
    }

    /// The options that seal them back in, as they were (the name of the HDF5 member, the version hint, the machine configuration and the other members).
    pub fn to_options(&self) -> SealOptions {
        let mut options = SealOptions::new().with_primary_name(self.primary_name.clone());
        options.schema_version = self.schema_version;
        for member in &self.members {
            if self.machine_config && member.name == MACHINE_CONFIG_MEMBER {
                options.machine_config = Some(member.source.clone());
            } else {
                options.members.push(member.clone());
            }
        }
        options
    }

    /// Removes the side file, once the archive is sealed again.
    pub fn finish(self) -> Result<(), Error> {
        fs::remove_file(&self.sidecar)
            .map_err(|e| io_error(&format!("removing `{}`", self.sidecar.display()), &e))
    }

    /// Reads the side file of the archive at `path`, if an [`unseal`] left one (the process died before the archive was sealed again). `None` if there is none.
    pub fn load(path: &Path) -> Result<Option<Detached>, Error> {
        let sidecar = sidecar_of(path);
        if !sidecar.is_file() {
            return Ok(None);
        }
        let file = File::open(&sidecar)
            .map_err(|e| io_error(&format!("opening `{}`", sidecar.display()), &e))?;
        let listed = read_members(&file)?
            .ok_or_else(|| corrupt(format!("`{}` is not an archive", sidecar.display())))?;
        let mut detached = Detached {
            primary_name: DEFAULT_PRIMARY.to_string(),
            machine_config: false,
            schema_version: None,
            members: Vec::new(),
            sidecar: sidecar.clone(),
        };
        let read = |m: &Member| -> Result<Vec<u8>, Error> {
            let mut bytes = vec![0u8; m.size as usize];
            file.read_exact_at(m.data_offset, &mut bytes)
                .map_err(|e| io_error(&format!("reading `{}` from the side file", m.name), &e))?;
            Ok(bytes)
        };
        let manifest = match listed.iter().find(|m| m.name == MANIFEST_MEMBER) {
            Some(m) => Manifest::from_json(&read(m)?)?,
            None => Manifest::default(),
        };
        if let Some(primary) = &manifest.primary {
            detached.primary_name = primary.clone();
        }
        detached.schema_version = manifest.schema_version;
        for m in &listed {
            if m.name == MIMETYPE_MEMBER || m.name == MANIFEST_MEMBER {
                continue;
            }
            if m.name == MACHINE_CONFIG_MEMBER {
                detached.machine_config = true;
            }
            detached.members.push(SealMember {
                name: m.name.clone(),
                source: MemberSource::Bytes(read(m)?),
            });
        }
        Ok(Some(detached))
    }
}

/// Takes the members other than the HDF5 member out of the sealed archive at `path` and cuts the file back to the HDF5 data, which makes it a working file again: HDF5 can open it for writing.
///
/// The members are read into memory first (`BudgetExceeded` if they are more than `limit` bytes together, so that a large attachment is never copied by mistake), checked against their CRC-32, and saved to the side file. Nothing is cut until
/// the side file is safely there. The other members must have been written by [`seal`]: the HDF5 member must be the first after `mimetype`, at [`USERBLOCK`].
pub fn unseal(path: &Path, limit: u64) -> Result<Detached, Error> {
    let sidecar = sidecar_of(path);
    if sidecar.exists() {
        return Err(validation(format!(
            "`{}` exists: a previous unseal of this archive was not finished. Read it with `Detached::load`, or remove it if the archive is as it should be",
            sidecar.display()
        )));
    }
    let archive = Archive::open_path(path)?;
    let primary = archive.primary().clone();
    if primary.data_offset != USERBLOCK {
        return Err(validation(format!("the HDF5 member starts at byte {}, not at {USERBLOCK}: this archive was not sealed by this library", primary.data_offset)));
    }
    let mut detached = Detached {
        primary_name: primary.name.clone(),
        machine_config: false,
        schema_version: archive.manifest().schema_version,
        members: Vec::new(),
        sidecar: sidecar.clone(),
    };
    let others: Vec<Member> = archive
        .members()
        .iter()
        .filter(|m| {
            m.name != primary.name && m.name != MIMETYPE_MEMBER && m.name != MANIFEST_MEMBER
        })
        .cloned()
        .collect();
    let needed: u64 = others.iter().map(|m| m.size).sum();
    if needed > limit {
        return Err(Error::new(ErrorKind::BudgetExceeded {
            limit_bytes: limit,
            needed_bytes: needed,
        }));
    }
    for member in &others {
        archive.verify(member)?;
        detached.machine_config |= member.name == MACHINE_CONFIG_MEMBER;
        detached.members.push(SealMember {
            name: member.name.clone(),
            source: MemberSource::Bytes(archive.read_member(member, limit)?),
        });
    }
    let hdf5_end = primary.data_end();
    drop(archive);

    // the side file, atomically: it either exists whole or not at all
    let partial = {
        let mut name = sidecar.as_os_str().to_os_string();
        name.push(".part");
        PathBuf::from(name)
    };
    let mut side = File::create(&partial)
        .map_err(|e| io_error(&format!("creating `{}`", partial.display()), &e))?;
    let stamp = dos(now_unix());
    let mut progress = Progress {
        callback: None,
        done: 0,
        total: needed,
    };
    let mut pos = 0u64;
    let mime = MemberSource::Bytes(MIMETYPE.as_bytes().to_vec());
    let mut written = vec![append_member(
        &mut side,
        &mut pos,
        MIMETYPE_MEMBER,
        &mime,
        false,
        stamp,
        &mut progress,
    )?];
    for member in &detached.members {
        written.push(append_member(
            &mut side,
            &mut pos,
            &member.name,
            &member.source,
            false,
            stamp,
            &mut progress,
        )?);
    }
    let manifest = Manifest {
        primary: Some(detached.primary_name.clone()),
        machine_config: detached
            .machine_config
            .then(|| MACHINE_CONFIG_MEMBER.to_string()),
        schema_version: detached.schema_version,
        attachments: detached.members.iter().map(|m| m.name.clone()).collect(),
        ..Manifest::default()
    };
    written.push(append_member(
        &mut side,
        &mut pos,
        MANIFEST_MEMBER,
        &MemberSource::Bytes(manifest.to_json()),
        false,
        stamp,
        &mut progress,
    )?);
    write_directory(&mut side, pos, &written, stamp)?;
    side.sync_all()
        .map_err(|e| io_error("saving the side file", &e))?;
    drop(side);
    fs::rename(&partial, &sidecar)
        .map_err(|e| io_error(&format!("renaming `{}`", partial.display()), &e))?;

    // only now is the file cut back to the HDF5 data
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| io_error(&format!("opening `{}` for writing", path.display()), &e))?;
    file.set_len(hdf5_end).map_err(|e| {
        io_error(
            &format!("cutting `{}` back to its HDF5 data", path.display()),
            &e,
        )
    })?;
    file.sync_all().map_err(|e| {
        io_error(
            &format!("cutting `{}` back to its HDF5 data", path.display()),
            &e,
        )
    })?;
    Ok(detached)
}
