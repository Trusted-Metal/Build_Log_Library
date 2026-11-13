//! One member of an archive.

/// A member of an archive, as its central directory describes it, with the place of its bytes in the file.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Member {
    /// The name, as the archive spells it (`build.h5`, `machine_config.h5`, `logs/capture.log`).
    pub name: String,
    /// Where the member's local header starts.
    pub header_offset: u64,
    /// Where the member's bytes start: **an offset in the archive file**, so byte `n` of the member is byte `data_offset + n` of the file.
    pub data_offset: u64,
    /// The size of the member's bytes.
    pub size: u64,
    /// The size of the bytes as they are in the file; equal to `size` for a stored member.
    pub compressed_size: u64,
    /// The CRC-32 the archive records for the member.
    pub crc32: u32,
    /// Stored (not compressed, not encrypted): the member can be read by range. A member that is not is listed, and cannot be read.
    pub stored: bool,
}

impl Member {
    /// The member's place in the file, one past its last byte.
    pub fn data_end(&self) -> u64 {
        self.data_offset + self.compressed_size
    }
}
