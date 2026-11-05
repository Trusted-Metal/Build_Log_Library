//! The libhdf5 minimum (IP-03 phase 3G, task T03.10, decision D03-5, requirement BL-ST-01): declared once, checked before a file is opened, with no maximum.

use build_log_io::hdf5::{open_file, open_file_with_minimum};
use build_log_io::{
    check_libhdf5, check_linked_libhdf5, libhdf5_version, Libhdf5Version, LIBHDF5_MINIMUM,
};
use build_log_model::{ErrorClass, ErrorKind};
use build_log_testkit::{miniature, miniature_path, repo_root};

#[test]
fn a_libhdf5_below_the_minimum_is_refused() {
    let minimum = LIBHDF5_MINIMUM;
    assert_eq!(minimum, Libhdf5Version::new(1, 10, 8));
    for older in [
        Libhdf5Version::new(1, 10, 7),
        Libhdf5Version::new(1, 8, 22),
        Libhdf5Version::new(0, 0, 0),
    ] {
        let err = check_libhdf5(older, minimum).expect_err("older than the minimum");
        match err.kind() {
            ErrorKind::UnsupportedInVersion { version, feature } => {
                assert!(
                    version.contains(&older.to_string()),
                    "names the library found: {version}"
                );
                assert!(
                    feature.contains(&minimum.to_string()),
                    "names the minimum: {feature}"
                );
            }
            other => panic!("expected UnsupportedInVersion, found {other:?}"),
        }
        assert_eq!(err.class(), ErrorClass::Caller);
        assert!(!err.is_retryable());
    }
    // the minimum and everything newer is accepted: there is no maximum (a new major version is reported by CI, not refused)
    for newer in [
        Libhdf5Version::new(1, 10, 8),
        Libhdf5Version::new(1, 10, 10),
        Libhdf5Version::new(1, 14, 6),
        Libhdf5Version::new(2, 2, 0),
        Libhdf5Version::new(3, 0, 0),
        Libhdf5Version::new(255, 255, 255),
    ] {
        assert!(check_libhdf5(newer, minimum).is_ok(), "{newer} is accepted");
    }
}

#[test]
fn opening_a_file_checks_the_minimum_before_it_touches_the_file() {
    // a minimum that no libhdf5 meets: the file is fine, and so is the path, and the refusal is about the library
    let too_new = Libhdf5Version::new(255, 0, 0);
    let err = open_file_with_minimum(miniature_path(miniature::V16), too_new)
        .expect_err("the library is too old for this minimum");
    assert!(
        matches!(err.kind(), ErrorKind::UnsupportedInVersion { .. }),
        "{err}"
    );
    // and it comes before the open: a path that does not exist gives the same refusal, not an I/O error
    let err = open_file_with_minimum("no/such/file.h5", too_new).expect_err("refused first");
    assert!(
        matches!(err.kind(), ErrorKind::UnsupportedInVersion { .. }),
        "{err}"
    );
    // with the declared minimum, the linked libhdf5 opens a miniature
    assert!(open_file(miniature_path(miniature::V16)).is_ok());
}

#[test]
fn the_linked_version_is_available_to_a_writer() {
    // a writer records the libhdf5 it wrote with: the version is a value that prints and reads back, and it meets the minimum on every leg
    let linked = libhdf5_version();
    assert!(
        linked >= LIBHDF5_MINIMUM,
        "the libhdf5 linked here is {linked}"
    );
    assert_eq!(linked.to_string().parse::<Libhdf5Version>(), Ok(linked));
    assert!(check_linked_libhdf5().is_ok());
}

#[test]
fn the_minimum_declared_in_ci_is_the_one_the_library_declares() {
    // the workflow names the minimum for its version report; the library declares it once, and this keeps the two equal
    let workflow = std::fs::read_to_string(repo_root().join(".github/workflows/rust.yml"))
        .expect("the workflow");
    let line = workflow
        .lines()
        .find(|l| l.trim_start().starts_with("LIBHDF5_MINIMUM:"))
        .expect("the workflow declares LIBHDF5_MINIMUM");
    let text = line
        .split(':')
        .nth(1)
        .expect("a value")
        .trim()
        .trim_matches('"');
    assert_eq!(text.parse::<Libhdf5Version>(), Ok(LIBHDF5_MINIMUM));
}
