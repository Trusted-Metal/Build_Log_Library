//! The HDF5 binding links and works (IP-01 task T01.7, phase 1D; requirement BL-ST-01).
//!
//! These tests need a libhdf5: the one installed on the machine, or the one `--features static-hdf5` compiles in.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use build_log_io::{libhdf5_version, Libhdf5Version};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A file path in the system temporary folder that is removed when the value is dropped, even if the test fails.
struct TempFile(PathBuf);

impl TempFile {
    fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        TempFile(std::env::temp_dir().join(format!(
            "build-log-io-{}-{}-{label}.h5",
            std::process::id(),
            n
        )))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn libhdf5_links_and_reports_its_version() {
    let version = libhdf5_version();
    // the number is printed so a CI log shows which libhdf5 each leg linked (IP-01 phase 1I reports it properly)
    eprintln!("linked libhdf5 {version}");
    assert!(
        version.major >= 1,
        "an HDF5 library reports a major version of at least 1, got {version}"
    );
    assert!(
        version >= Libhdf5Version::new(1, 8, 4),
        "hdf5-metno needs libhdf5 1.8.4 or later, got {version}"
    );
    assert_eq!(
        version.to_string(),
        format!("{}.{}.{}", version.major, version.minor, version.micro)
    );
}

#[test]
fn hdf5_roundtrips_one_attribute_in_a_temp_file() {
    let temp = TempFile::new("attribute");
    {
        let file = hdf5::File::create(&temp.0).expect("a new HDF5 file can be created");
        let attribute = file
            .new_attr::<i32>()
            .create("File_Version")
            .expect("an attribute can be created");
        attribute
            .write_scalar(&18_i32)
            .expect("the attribute can be written");
    }
    let file = hdf5::File::open(&temp.0).expect("the file can be opened again");
    let value: i32 = file
        .attr("File_Version")
        .expect("the attribute is there")
        .read_scalar()
        .expect("it can be read");
    assert_eq!(value, 18);
}

#[test]
fn a_file_that_is_not_hdf5_is_an_error_not_a_crash() {
    let temp = TempFile::new("not-hdf5");
    std::fs::write(&temp.0, b"this is not an HDF5 file").expect("a plain file");
    assert!(hdf5::File::open(&temp.0).is_err());
}
