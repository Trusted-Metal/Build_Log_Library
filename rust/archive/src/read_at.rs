//! Reading bytes at a given offset, from anything that can: a file, or a slice of memory in a test or on `wasm32`.

use std::io;

/// A source of bytes that can be read at any offset, from several threads at once: **no cursor**. A file implements it with the system's positional read, so reads from different threads never disturb each other.
pub trait ReadAt: Send + Sync {
    /// The length in bytes.
    fn len(&self) -> io::Result<u64>;

    /// Reads up to `buf.len()` bytes at `offset`; fewer only at the end. Zero means the end.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;

    /// Is it empty?
    fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Reads exactly `buf.len()` bytes at `offset`, or fails with `UnexpectedEof`.
    fn read_exact_at(&self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        while !buf.is_empty() {
            match self.read_at(offset, buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!(
                            "the data ends at {offset}, before the {} bytes asked for",
                            buf.len()
                        ),
                    ))
                }
                Ok(n) => {
                    offset += n as u64;
                    buf = &mut buf[n..];
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl ReadAt for [u8] {
    fn len(&self) -> io::Result<u64> {
        Ok(<[u8]>::len(self) as u64)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let Ok(start) = usize::try_from(offset) else {
            return Ok(0);
        };
        if start >= <[u8]>::len(self) {
            return Ok(0);
        }
        let n = buf.len().min(<[u8]>::len(self) - start);
        buf[..n].copy_from_slice(&self[start..start + n]);
        Ok(n)
    }
}

impl ReadAt for Vec<u8> {
    fn len(&self) -> io::Result<u64> {
        Ok(Vec::len(self) as u64)
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.as_slice().read_at(offset, buf)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ReadAt for std::fs::File {
    fn len(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    #[cfg(unix)]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        std::os::unix::fs::FileExt::read_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        // moves the handle's cursor, which nothing here uses
        std::os::windows::fs::FileExt::seek_read(self, buf, offset)
    }
}
