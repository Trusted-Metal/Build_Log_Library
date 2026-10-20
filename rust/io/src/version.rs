//! Which libhdf5 this build is linked against.
//!
//! The library has to work on every libhdf5 its consumers use (1.10.8 in tauri's container, 1.14 on its Windows installer, and whatever a build with
//! `static-hdf5` compiles in), so each CI leg reports the version it ran with (IP-01 phase 1I) and a minimum is declared (IP-03).

use std::fmt;
use std::str::FromStr;

/// A libhdf5 version, as the library itself reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Libhdf5Version {
    pub major: u8,
    pub minor: u8,
    pub micro: u8,
}

impl Libhdf5Version {
    pub const fn new(major: u8, minor: u8, micro: u8) -> Self {
        Self {
            major,
            minor,
            micro,
        }
    }
}

impl fmt::Display for Libhdf5Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.micro)
    }
}

/// Text that is not a libhdf5 version of the form `major.minor.micro`, such as `1.10.8`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseLibhdf5VersionError {
    text: String,
}

impl fmt::Display for ParseLibhdf5VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "not a libhdf5 version (expected major.minor.micro, such as 1.10.8): {:?}",
            self.text
        )
    }
}

impl std::error::Error for ParseLibhdf5VersionError {}

impl FromStr for Libhdf5Version {
    type Err = ParseLibhdf5VersionError;

    /// Reads `major.minor.micro`: three plain decimal numbers, each 0 to 255, and nothing else (CI passes the minimum it checks against this way).
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ParseLibhdf5VersionError {
            text: text.to_string(),
        };
        let number = |part: &str| -> Result<u8, ParseLibhdf5VersionError> {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(error());
            }
            part.parse::<u8>().map_err(|_| error())
        };
        let mut parts = text.split('.');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(major), Some(minor), Some(micro), None) => {
                Ok(Self::new(number(major)?, number(minor)?, number(micro)?))
            }
            _ => Err(error()),
        }
    }
}

/// The version of the libhdf5 linked into this build. Calling it loads and initialises the library.
pub fn libhdf5_version() -> Libhdf5Version {
    let (major, minor, micro) = hdf5::library_version();
    Libhdf5Version::new(major, minor, micro)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_print_as_major_minor_micro_and_order_by_those_numbers() {
        assert_eq!(Libhdf5Version::new(1, 10, 8).to_string(), "1.10.8");
        assert!(Libhdf5Version::new(1, 10, 8) < Libhdf5Version::new(1, 14, 0));
        assert!(Libhdf5Version::new(1, 14, 9) < Libhdf5Version::new(2, 0, 0));
        assert!(Libhdf5Version::new(1, 10, 9) > Libhdf5Version::new(1, 10, 8));
    }

    #[test]
    fn a_version_is_read_back_from_the_text_it_prints() {
        for version in [
            Libhdf5Version::new(1, 10, 8),
            Libhdf5Version::new(1, 14, 6),
            Libhdf5Version::new(2, 2, 0),
            Libhdf5Version::new(0, 0, 0),
            Libhdf5Version::new(255, 255, 255),
        ] {
            assert_eq!(version.to_string().parse::<Libhdf5Version>(), Ok(version));
        }
    }

    #[test]
    fn text_that_is_not_major_minor_micro_is_rejected() {
        for text in [
            "", "1", "1.10", "1.10.8.1", "1..8", "a.b.c", "1.10.x", "1.10.-8", "+1.10.8",
            " 1.10.8", "1.10.8 ", "1.10.8\n", "v1.10.8", "256.0.0", "1.256.0", "1.0.256", "1,10,8",
        ] {
            assert!(
                text.parse::<Libhdf5Version>().is_err(),
                "{text:?} should not parse"
            );
        }
        let message = "1.10".parse::<Libhdf5Version>().unwrap_err().to_string();
        assert!(
            message.contains("1.10"),
            "the message names the text: {message}"
        );
    }
}
