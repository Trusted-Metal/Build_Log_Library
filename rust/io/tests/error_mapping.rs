//! The mapping of the binding's errors into the library's (IP-03 phase 3G, task T03.9, decision D03-8, Appendix A of the plan), on real failures of libhdf5.
//!
//! Each test makes the failure itself: a file that is missing, one that is not HDF5, truncated copies of a miniature (made in the system's temporary folder
//! and removed afterwards, never edited in place), an object that is not there. The tests run on every libhdf5 of the CI legs, because the wording of the
//! failures is libhdf5's.

use std::error::Error as StdError;
use std::path::PathBuf;

use build_log_io::hdf5::{from_binding, open_file};
use build_log_model::{Error, ErrorClass, ErrorKind};
use build_log_testkit::{miniature, miniature_path};
use hdf5::File;

/// A file in the system's temporary folder that is removed when the value is dropped, even if the test fails.
struct TempFile(PathBuf);

impl TempFile {
    fn with(label: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "build-log-io-error-mapping-{}-{label}.h5",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("a writable temporary folder");
        TempFile(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn v16_bytes() -> Vec<u8> {
    std::fs::read(miniature_path(miniature::V16)).expect("the V16 miniature")
}

#[test]
fn a_truncated_file_is_corrupt_not_a_panic() {
    let bytes = v16_bytes();
    // cut before the signature, inside the superblock, in the middle, and one byte short
    for cut in [0, 100, 2_000, bytes.len() / 2, bytes.len() - 1] {
        let copy = TempFile::with(&format!("cut-{cut}"), &bytes[..cut]);
        let err = open_file(&copy.0).expect_err("a truncated file does not open");
        assert!(
            matches!(err.kind(), ErrorKind::Corrupt { .. }),
            "cut at {cut}: {err}"
        );
        assert_eq!(err.class(), ErrorClass::LibraryOrFile, "cut at {cut}");
        assert!(!err.is_retryable(), "cut at {cut}");
        assert!(
            err.source().is_some(),
            "the binding's error is kept as the source"
        );
    }
}

#[test]
fn a_file_that_is_not_hdf5_is_corrupt() {
    let text = TempFile::with(
        "text",
        &b"this is not an hdf5 file, just some text\n".repeat(50),
    );
    let err = open_file(&text.0).expect_err("text does not open as HDF5");
    assert!(matches!(err.kind(), ErrorKind::Corrupt { .. }), "{err}");
    assert_eq!(err.class(), ErrorClass::LibraryOrFile);
}

#[test]
fn a_missing_file_is_an_io_error_with_its_os_code() {
    let path = std::env::temp_dir().join(format!(
        "build-log-io-no-such-file-{}.h5",
        std::process::id()
    ));
    let err = open_file(&path).expect_err("there is no such file");
    // errno 2 is "no such file or directory" on every operating system of the CI legs; the cause cannot pass, so trying again does not help
    match err.kind() {
        ErrorKind::IoError {
            os_code, transient, ..
        } => {
            assert_eq!(*os_code, Some(2), "{err}");
            assert!(!transient);
        }
        other => panic!("a missing file is an IoError, found {other:?}: {err}"),
    }
    assert_eq!(err.class(), ErrorClass::Io);
    assert!(!err.is_retryable());
}

#[test]
fn every_binding_error_maps_to_a_kind_with_a_class() {
    let missing = std::env::temp_dir().join(format!(
        "build-log-io-no-such-file-{}.h5",
        std::process::id()
    ));
    let text = TempFile::with("mapping-text", &b"not hdf5\n".repeat(100));
    let bytes = v16_bytes();
    let cut = TempFile::with("mapping-cut", &bytes[..2_000]);
    let file = File::open(miniature_path(miniature::V16)).expect("the V16 miniature");
    let group = file.group("/Built_Part").expect("a group");
    let a_string = group
        .attr_names()
        .expect("attribute names")
        .into_iter()
        .map(|n| group.attr(&n).expect("an attribute"))
        .find(|a| {
            matches!(
                a.dtype().and_then(|d| d.to_descriptor()),
                Ok(hdf5::types::TypeDescriptor::FixedAscii(_))
            )
        })
        .expect("a string attribute");

    // (what went wrong, the binding's error, the kind, the class, whether trying again can help)
    let cases: Vec<(&str, hdf5::Error, &str, ErrorClass, bool)> = vec![
        (
            "a missing file",
            File::open(&missing).expect_err("missing"),
            "IoError",
            ErrorClass::Io,
            false,
        ),
        (
            "a file that is not HDF5",
            File::open(&text.0).expect_err("text"),
            "Corrupt",
            ErrorClass::LibraryOrFile,
            false,
        ),
        (
            "a truncated file",
            File::open(&cut.0).expect_err("cut"),
            "Corrupt",
            ErrorClass::LibraryOrFile,
            false,
        ),
        (
            "a group that is not there",
            file.group("/no/such/group").expect_err("missing"),
            "NotPresent",
            ErrorClass::Validation,
            false,
        ),
        (
            "a group asked for as a dataset",
            file.dataset("/Built_Part").expect_err("a group"),
            "NotPresent",
            ErrorClass::Validation,
            false,
        ),
        (
            "a conversion the binding does not offer",
            a_string.read_raw::<f64>().expect_err("no conversion"),
            "UnsupportedInVersion",
            ErrorClass::Caller,
            false,
        ),
        (
            "an error of the binding's own layer",
            hdf5::Error::from("something the binding refused"),
            "UnsupportedInVersion",
            ErrorClass::Caller,
            false,
        ),
    ];
    for (what, binding_error, kind_name, class, retryable) in cases {
        let mapped: Error = from_binding("a test", binding_error);
        let kind = match mapped.kind() {
            ErrorKind::IoError { .. } => "IoError",
            ErrorKind::Corrupt { .. } => "Corrupt",
            ErrorKind::NotPresent { .. } => "NotPresent",
            ErrorKind::UnsupportedInVersion { .. } => "UnsupportedInVersion",
            other => panic!("{what}: a kind that the mapping must not give: {other:?}"),
        };
        assert_eq!(kind, kind_name, "{what}: {mapped}");
        assert_eq!(mapped.class(), class, "{what}");
        assert_eq!(mapped.is_retryable(), retryable, "{what}");
        assert!(
            mapped.source().is_some(),
            "{what}: the binding's error is kept as the source"
        );
    }
}
