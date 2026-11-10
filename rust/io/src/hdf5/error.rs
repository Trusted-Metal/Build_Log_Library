//! The mapping of the binding's errors into the library's error taxonomy, at this one place (IP-03 phase 3G, decision D03-8, Appendix A of the plan).
//!
//! | The binding reports | The library says | Class | Can trying again help |
//! |---|---|---|---|
//! | an object (group, dataset, attribute) that is not there, or is not of the kind asked for | `NotPresent` | Validation | no |
//! | an operating-system error (the file does not exist, cannot be opened or read), with its `errno` | `IoError { os_code }` | I/O | by cause: an interrupted or busy device or too many open files, yes; a missing file, a refused access or a full disk, no |
//! | a file that is not HDF5, is truncated, or fails a check | `Corrupt` | Library or file | no |
//! | a request that the binding cannot honour (a conversion it does not offer, a datatype class it does not know) | `UnsupportedInVersion`, naming it | Caller | no |
//! | anything else from libhdf5 | `Corrupt`, with what libhdf5 said | Library or file | no |
//!
//! libhdf5 describes a failure as a stack of frames, each with a function, a message and a major and minor error, and the cause is in the innermost frames. The
//! outer frames of a failed open all say "unable to open file", so the mapping reads the text of the whole stack for the cause and not the top of it, and it
//! tests for a damaged file before a failed read: the stack of a file that is not HDF5 also says "read failed". The wording is libhdf5's, so the tests run on
//! every libhdf5 of the CI legs (1.10.8, 1.10.10 and 2.2.0). Nothing in the primitives panics on the content of a file.

use build_log_model::{Error, ErrorKind};

/// An object that the file does not have (or that is not of the kind that was asked for).
pub(crate) fn not_present(what: impl Into<String>) -> Error {
    Error::new(ErrorKind::NotPresent { what: what.into() })
}

/// A kind of value that the primitives do not read, named. The primitives know no version, so the version says so; an adapter that knows its version
/// can wrap the error.
pub(crate) fn unsupported(feature: impl Into<String>) -> Error {
    Error::new(ErrorKind::UnsupportedInVersion {
        version: "(any: the primitives know no version)".to_string(),
        feature: feature.into(),
    })
}

/// What libhdf5 says about each frame of a failure, as one text: the function, the message and the major and minor error of every frame.
fn stack_text(source: &hdf5::Error) -> String {
    match source {
        hdf5::Error::Internal(text) => text.clone(),
        hdf5::Error::HDF5(stack) => match stack.clone().expand() {
            Ok(frames) => frames
                .iter()
                .filter_map(|frame| frame.detail())
                .collect::<Vec<_>>()
                .join("\n"),
            Err(_) => source.to_string(),
        },
    }
}

/// The `errno` that libhdf5 quotes when an operating-system call failed (`errno = 2, error message = 'No such file or directory'`).
fn errno_of(text: &str) -> Option<i32> {
    let marker = "errno = ";
    let at = text.find(marker)? + marker.len();
    let digits: String = text[at..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Can the cause of an operating-system error pass? An interrupted call, a try-again, a busy device and too many open files can; the rest (a missing file, a
/// refused access, a full disk, an input/output error) cannot.
fn errno_is_transient(errno: i32) -> bool {
    matches!(errno, 4 | 11 | 16 | 24)
}

/// Turns a failure of the binding, while the primitives were doing `what`, into the library's error. The binding's error is kept as its source.
pub fn from_binding(what: &str, source: hdf5::Error) -> Error {
    let kind = classify(what, &source);
    Error::new(kind).with_source(source)
}

fn classify(what: &str, source: &hdf5::Error) -> ErrorKind {
    let text = stack_text(source);
    let said = source.to_string();
    if let hdf5::Error::Internal(_) = source {
        // an error of the binding's own Rust layer: a conversion it does not offer, a datatype class it does not know
        return ErrorKind::UnsupportedInVersion {
            version: "(any: the primitives know no version)".to_string(),
            feature: format!("{what}: {said}"),
        };
    }
    // a damaged file first: the stack of a file that is not HDF5 also says that a read failed
    const DAMAGED: [&str; 5] = [
        "Not an HDF5 file",
        "File has been truncated",
        "file signature not found",
        "truncated file",
        "checksum",
    ];
    if DAMAGED.iter().any(|marker| text.contains(marker)) {
        return ErrorKind::Corrupt {
            detail: format!("{what}: {said}"),
        };
    }
    const ABSENT: [&str; 4] = [
        "Object not found",
        "component not found",
        "Inappropriate type",
        "not a dataset",
    ];
    if ABSENT.iter().any(|marker| text.contains(marker)) {
        return ErrorKind::NotPresent {
            what: format!("{what}: {said}"),
        };
    }
    if let Some(os_code) = errno_of(&text) {
        return ErrorKind::IoError {
            os_code: Some(os_code),
            detail: format!("{what}: {said}"),
            transient: errno_is_transient(os_code),
        };
    }
    ErrorKind::Corrupt {
        detail: format!("{what}: {said}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_errno_that_libhdf5_quotes_is_read_from_its_text() {
        let text = "Error in H5FD__sec2_open(): unable to open file: name = 'x.h5', errno = 2, error message = 'No such file or directory', flags = 0";
        assert_eq!(errno_of(text), Some(2));
        assert_eq!(
            errno_of("errno = 28, error message = 'No space left'"),
            Some(28)
        );
        assert_eq!(errno_of("file signature not found"), None);
        assert_eq!(errno_of("errno = , nothing"), None);
    }

    #[test]
    fn only_a_cause_that_can_pass_is_transient() {
        // interrupted, try again, busy, too many open files
        for errno in [4, 11, 16, 24] {
            assert!(errno_is_transient(errno), "{errno}");
        }
        // no such file, input/output error, refused, full disk
        for errno in [2, 5, 13, 28] {
            assert!(!errno_is_transient(errno), "{errno}");
        }
    }
}
