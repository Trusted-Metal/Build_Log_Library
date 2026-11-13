//! The parts of the ZIP format that an archive uses: reading the central directory (ZIP64 included), and the bytes of the headers a writer produces.
//!
//! Only what an `.lbl` needs is here: no compression, no encryption, no multi-disk archives. A member that is compressed or encrypted is listed (so that a tool can say so) and is not readable by range.

use std::io;

use build_log_model::{Error, ErrorKind};

use crate::member::Member;
use crate::read_at::ReadAt;

pub(crate) const SIG_LOCAL: u32 = 0x0403_4B50;
pub(crate) const SIG_CENTRAL: u32 = 0x0201_4B50;
pub(crate) const SIG_EOCD: u32 = 0x0605_4B50;
pub(crate) const SIG_EOCD64: u32 = 0x0606_4B50;
pub(crate) const SIG_EOCD64_LOCATOR: u32 = 0x0706_4B50;
/// The id of the ZIP64 extra field.
pub(crate) const EXTRA_ZIP64: u16 = 0x0001;
/// The id that Android's `zipalign` uses for the padding that puts a member's data at a multiple of an alignment: `u16 alignment`, then zero bytes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const EXTRA_ALIGN: u16 = 0xD935;
pub(crate) const FLAG_ENCRYPTED: u16 = 0x0001;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const FLAG_UTF8: u16 = 0x0800;
pub(crate) const U32_MAX: u64 = 0xFFFF_FFFF;
const U16_MAX: u64 = 0xFFFF;
/// The longest central directory that is believed: a directory of a few members is a few hundred bytes.
const DIRECTORY_LIMIT: u64 = 64 << 20;

pub(crate) fn corrupt(detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt {
        detail: detail.into(),
    })
}

/// An I/O failure as the library reports it. A sharing violation (Windows: another process has the file open in a way that excludes this) and a lock held elsewhere can pass, so they are transient.
pub(crate) fn io_error(what: &str, e: &io::Error) -> Error {
    let os_code = e.raw_os_error();
    let sharing_violation = cfg!(windows) && os_code == Some(32);
    let transient = sharing_violation
        || matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted | io::ErrorKind::TimedOut
        );
    let detail = if sharing_violation {
        format!("{what}: another process has the file open for writing ({e})")
    } else {
        format!("{what}: {e}")
    };
    Error::new(ErrorKind::IoError {
        os_code,
        detail,
        transient,
    })
    .with_source(io::Error::new(e.kind(), e.to_string()))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// Where the central directory is, from the end records.
struct EndRecords {
    directory_offset: u64,
    directory_size: u64,
    entries: u64,
}

/// Reads the end records. `None` when the file has none: it is not an archive, or it is one that was never sealed.
fn read_end_records<R: ReadAt + ?Sized>(reader: &R) -> Result<Option<EndRecords>, Error> {
    let len = reader
        .len()
        .map_err(|e| io_error("reading the size of the archive", &e))?;
    if len < 22 {
        return Ok(None);
    }
    let tail_len = len.min(U16_MAX + 22) as usize;
    let tail_start = len - tail_len as u64;
    let mut tail = vec![0u8; tail_len];
    reader
        .read_exact_at(tail_start, &mut tail)
        .map_err(|e| io_error("reading the end of the archive", &e))?;
    // the last position at which an end record starts and whose comment reaches exactly to the end of the file
    let position = (0..=tail_len - 22).rev().find(|&at| {
        u32_at(&tail, at) == SIG_EOCD && at + 22 + usize::from(u16_at(&tail, at + 20)) == tail_len
    });
    let Some(at) = position else {
        return Ok(None);
    };
    let eocd_at = tail_start + at as u64;
    let entries16 = u64::from(u16_at(&tail, at + 10));
    let size32 = u64::from(u32_at(&tail, at + 12));
    let offset32 = u64::from(u32_at(&tail, at + 16));
    let saturated = entries16 == U16_MAX || size32 == U32_MAX || offset32 == U32_MAX;

    // a ZIP64 locator sits just before the end record
    if eocd_at >= 20 {
        let mut locator = [0u8; 20];
        reader
            .read_exact_at(eocd_at - 20, &mut locator)
            .map_err(|e| io_error("reading the ZIP64 locator", &e))?;
        if u32_at(&locator, 0) == SIG_EOCD64_LOCATOR {
            let record_at = u64_at(&locator, 8);
            let mut record = [0u8; 56];
            reader.read_exact_at(record_at, &mut record).map_err(|e| {
                corrupt(format!(
                    "the ZIP64 end record cannot be read at {record_at}: {e}"
                ))
            })?;
            if u32_at(&record, 0) != SIG_EOCD64 {
                return Err(corrupt(format!(
                    "there is no ZIP64 end record at {record_at}, where the locator says"
                )));
            }
            return Ok(Some(EndRecords {
                entries: u64_at(&record, 32),
                directory_size: u64_at(&record, 40),
                directory_offset: u64_at(&record, 48),
            }));
        }
    }
    if saturated {
        return Err(corrupt(
            "the end record needs the ZIP64 fields and the archive has none",
        ));
    }
    Ok(Some(EndRecords {
        entries: entries16,
        directory_size: size32,
        directory_offset: offset32,
    }))
}

/// One entry of the central directory, before the local header says where its data starts.
struct Entry {
    name: String,
    header_offset: u64,
    size: u64,
    compressed_size: u64,
    crc32: u32,
    stored: bool,
}

fn parse_entries(directory: &[u8], expected: u64) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    let mut at = 0usize;
    while at < directory.len() {
        if directory.len() - at < 46 || u32_at(directory, at) != SIG_CENTRAL {
            return Err(corrupt(format!(
                "the central directory has no entry at byte {at} of it"
            )));
        }
        let flags = u16_at(directory, at + 8);
        let method = u16_at(directory, at + 10);
        let crc32 = u32_at(directory, at + 16);
        let mut compressed = u64::from(u32_at(directory, at + 20));
        let mut size = u64::from(u32_at(directory, at + 24));
        let name_len = usize::from(u16_at(directory, at + 28));
        let extra_len = usize::from(u16_at(directory, at + 30));
        let comment_len = usize::from(u16_at(directory, at + 32));
        let mut header_offset = u64::from(u32_at(directory, at + 42));
        let end = at + 46 + name_len + extra_len + comment_len;
        if end > directory.len() {
            return Err(corrupt(
                "an entry of the central directory runs past its end",
            ));
        }
        let name = String::from_utf8_lossy(&directory[at + 46..at + 46 + name_len]).into_owned();
        // the ZIP64 extra field holds, in this order, those of the three values that were saturated
        let mut extra = &directory[at + 46 + name_len..at + 46 + name_len + extra_len];
        while extra.len() >= 4 {
            let id = u16_at(extra, 0);
            let field_len = usize::from(u16_at(extra, 2));
            if 4 + field_len > extra.len() {
                return Err(corrupt(format!(
                    "an extra field of `{name}` runs past its entry"
                )));
            }
            if id == EXTRA_ZIP64 {
                let mut field = &extra[4..4 + field_len];
                let mut take = |want: bool, value: &mut u64| -> Result<(), Error> {
                    if want {
                        if field.len() < 8 {
                            return Err(corrupt(format!(
                                "the ZIP64 field of `{name}` is too short"
                            )));
                        }
                        *value = u64_at(field, 0);
                        field = &field[8..];
                    }
                    Ok(())
                };
                take(size == U32_MAX, &mut size)?;
                take(compressed == U32_MAX, &mut compressed)?;
                take(header_offset == U32_MAX, &mut header_offset)?;
            }
            extra = &extra[4 + field_len..];
        }
        if !name.ends_with('/') {
            entries.push(Entry {
                name,
                header_offset,
                size,
                compressed_size: compressed,
                crc32,
                stored: method == 0 && flags & FLAG_ENCRYPTED == 0,
            });
        }
        at = end;
    }
    let listed = entries.len() as u64;
    // directories are skipped above, so the count can only be checked as an upper bound
    if listed > expected {
        return Err(corrupt(format!(
            "the central directory holds {listed} members and the end record says {expected}"
        )));
    }
    Ok(entries)
}

/// The members of the archive, with where their bytes start. `None` when the file has no end records. The start of a member's data is in its local header, which is read here and checked against the central directory.
pub(crate) fn read_members<R: ReadAt + ?Sized>(reader: &R) -> Result<Option<Vec<Member>>, Error> {
    let Some(end) = read_end_records(reader)? else {
        return Ok(None);
    };
    if end.directory_size > DIRECTORY_LIMIT {
        return Err(corrupt(format!(
            "the central directory claims {} bytes",
            end.directory_size
        )));
    }
    let mut directory = vec![0u8; end.directory_size as usize];
    reader
        .read_exact_at(end.directory_offset, &mut directory)
        .map_err(|e| {
            corrupt(format!(
                "the central directory cannot be read at {}: {e}",
                end.directory_offset
            ))
        })?;
    let entries = parse_entries(&directory, end.entries)?;
    let mut members: Vec<Member> = Vec::with_capacity(entries.len());
    for entry in entries {
        if members.iter().any(|m| m.name == entry.name) {
            return Err(corrupt(format!(
                "the archive holds two members named `{}`",
                entry.name
            )));
        }
        let mut local = [0u8; 30];
        reader
            .read_exact_at(entry.header_offset, &mut local)
            .map_err(|e| {
                corrupt(format!(
                    "the local header of `{}` cannot be read at {}: {e}",
                    entry.name, entry.header_offset
                ))
            })?;
        if u32_at(&local, 0) != SIG_LOCAL {
            return Err(corrupt(format!(
                "there is no local header for `{}` at {}, where the central directory puts it",
                entry.name, entry.header_offset
            )));
        }
        let name_len = u64::from(u16_at(&local, 26));
        let extra_len = u64::from(u16_at(&local, 28));
        let mut local_name = vec![0u8; name_len as usize];
        reader
            .read_exact_at(entry.header_offset + 30, &mut local_name)
            .map_err(|e| {
                corrupt(format!(
                    "the local header of `{}` is cut short: {e}",
                    entry.name
                ))
            })?;
        if local_name != entry.name.as_bytes() {
            return Err(corrupt(format!(
                "the local header at {} names another member than the central directory's `{}`",
                entry.header_offset, entry.name
            )));
        }
        members.push(Member {
            data_offset: entry.header_offset + 30 + name_len + extra_len,
            name: entry.name,
            header_offset: entry.header_offset,
            size: entry.size,
            compressed_size: entry.compressed_size,
            crc32: entry.crc32,
            stored: entry.stored,
        });
    }
    Ok(Some(members))
}

// ---------------------------------------------------------------- what a writer produces

/// What the header of one member of a writer's archive needs to say.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct HeaderSpec<'a> {
    pub name: &'a str,
    /// Where the local header starts in the file.
    pub offset: u64,
    pub size: u64,
    pub crc32: u32,
    /// Write the ZIP64 fields even for a member that does not need them.
    pub force_zip64: bool,
    /// The member's data starts at a multiple of this (1: anywhere).
    pub align: u64,
    pub dos_time: u16,
    pub dos_date: u16,
}

#[cfg(not(target_arch = "wasm32"))]
impl HeaderSpec<'_> {
    pub(crate) fn needs_zip64(&self) -> bool {
        self.force_zip64 || self.size >= U32_MAX
    }
}

/// The local header of a member, with the padding that puts its data at a multiple of `align`, and where the data then starts. The CRC is at byte 14 of the header, for a writer that learns it while it copies.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn local_header(spec: &HeaderSpec<'_>) -> (Vec<u8>, u64) {
    let name = spec.name.as_bytes();
    let zip64 = spec.needs_zip64();
    let mut extra = Vec::new();
    if zip64 {
        extra.extend_from_slice(&EXTRA_ZIP64.to_le_bytes());
        extra.extend_from_slice(&16u16.to_le_bytes());
        extra.extend_from_slice(&spec.size.to_le_bytes());
        extra.extend_from_slice(&spec.size.to_le_bytes());
    }
    if spec.align > 1 {
        // 4 bytes of extra-field header and 2 of alignment value, then zeros up to the multiple
        let base = spec.offset + 30 + name.len() as u64 + extra.len() as u64 + 6;
        let pad = (spec.align - base % spec.align) % spec.align;
        extra.extend_from_slice(&EXTRA_ALIGN.to_le_bytes());
        extra.extend_from_slice(&((2 + pad) as u16).to_le_bytes());
        extra.extend_from_slice(&(spec.align.min(U16_MAX) as u16).to_le_bytes());
        extra.resize(extra.len() + pad as usize, 0);
    }
    let mut header = Vec::with_capacity(30 + name.len() + extra.len());
    header.extend_from_slice(&SIG_LOCAL.to_le_bytes());
    header.extend_from_slice(&(if zip64 { 45u16 } else { 20 }).to_le_bytes());
    header.extend_from_slice(&FLAG_UTF8.to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes()); // stored
    header.extend_from_slice(&spec.dos_time.to_le_bytes());
    header.extend_from_slice(&spec.dos_date.to_le_bytes());
    header.extend_from_slice(&spec.crc32.to_le_bytes());
    let small = if zip64 {
        U32_MAX as u32
    } else {
        spec.size as u32
    };
    header.extend_from_slice(&small.to_le_bytes());
    header.extend_from_slice(&small.to_le_bytes());
    header.extend_from_slice(&(name.len() as u16).to_le_bytes());
    header.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    header.extend_from_slice(name);
    header.extend_from_slice(&extra);
    let data_offset = spec.offset + header.len() as u64;
    (header, data_offset)
}

/// The central directory entry of a member.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn central_entry(spec: &HeaderSpec<'_>) -> Vec<u8> {
    let name = spec.name.as_bytes();
    let zip64 = spec.needs_zip64() || spec.offset >= U32_MAX;
    let size_zip64 = spec.needs_zip64();
    let mut extra = Vec::new();
    if zip64 {
        let mut field = Vec::new();
        if size_zip64 {
            field.extend_from_slice(&spec.size.to_le_bytes());
            field.extend_from_slice(&spec.size.to_le_bytes());
        }
        if spec.force_zip64 || spec.offset >= U32_MAX {
            field.extend_from_slice(&spec.offset.to_le_bytes());
        }
        extra.extend_from_slice(&EXTRA_ZIP64.to_le_bytes());
        extra.extend_from_slice(&(field.len() as u16).to_le_bytes());
        extra.extend_from_slice(&field);
    }
    let small_size = if size_zip64 {
        U32_MAX as u32
    } else {
        spec.size as u32
    };
    let small_offset = if spec.force_zip64 || spec.offset >= U32_MAX {
        U32_MAX as u32
    } else {
        spec.offset as u32
    };
    let mut entry = Vec::with_capacity(46 + name.len() + extra.len());
    entry.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
    entry.extend_from_slice(&45u16.to_le_bytes()); // made by
    entry.extend_from_slice(&(if zip64 { 45u16 } else { 20 }).to_le_bytes());
    entry.extend_from_slice(&FLAG_UTF8.to_le_bytes());
    entry.extend_from_slice(&0u16.to_le_bytes()); // stored
    entry.extend_from_slice(&spec.dos_time.to_le_bytes());
    entry.extend_from_slice(&spec.dos_date.to_le_bytes());
    entry.extend_from_slice(&spec.crc32.to_le_bytes());
    entry.extend_from_slice(&small_size.to_le_bytes());
    entry.extend_from_slice(&small_size.to_le_bytes());
    entry.extend_from_slice(&(name.len() as u16).to_le_bytes());
    entry.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    entry.extend_from_slice(&0u16.to_le_bytes()); // comment
    entry.extend_from_slice(&0u16.to_le_bytes()); // disk
    entry.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    entry.extend_from_slice(&0u32.to_le_bytes()); // external attributes
    entry.extend_from_slice(&small_offset.to_le_bytes());
    entry.extend_from_slice(name);
    entry.extend_from_slice(&extra);
    entry
}

/// The end of an archive: the ZIP64 end record and its locator, always (so that a reader has one path for every size), then the classic end record.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn end_records(directory_offset: u64, directory_size: u64, entries: u64) -> Vec<u8> {
    let record_at = directory_offset + directory_size;
    let mut out = Vec::with_capacity(56 + 20 + 22);
    out.extend_from_slice(&SIG_EOCD64.to_le_bytes());
    out.extend_from_slice(&44u64.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&entries.to_le_bytes());
    out.extend_from_slice(&entries.to_le_bytes());
    out.extend_from_slice(&directory_size.to_le_bytes());
    out.extend_from_slice(&directory_offset.to_le_bytes());
    out.extend_from_slice(&SIG_EOCD64_LOCATOR.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&record_at.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&SIG_EOCD.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.min(U16_MAX) as u16).to_le_bytes());
    out.extend_from_slice(&(entries.min(U16_MAX) as u16).to_le_bytes());
    out.extend_from_slice(&(directory_size.min(U32_MAX) as u32).to_le_bytes());
    out.extend_from_slice(&(directory_offset.min(U32_MAX) as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
