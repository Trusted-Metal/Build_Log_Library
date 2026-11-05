//! The error taxonomy: kinds, classes, locations and violations.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** library (`Error`, `Location`, `Violation`); the enums `ErrorKind` and `ErrorClass`.
//! * **Requirement:** BL-FA-10.
//!
//! # What this module claims, and what it does not
//!
//! * **One taxonomy for the whole library**, from MCF's seven kinds plus `Cancelled`, `OutOfWidth`, `BudgetExceeded` and `Corrupt` (plan Appendix C). Each
//!   kind has a class (who is at fault) and a default for whether trying again can help; an error may override the default.
//! * **An error is a value.** It has no state that outlives it and nothing is sticky: a failure in one item does not hide the next. Using a closed handle
//!   returns `Closed`, and bad input returns an error: the writer does not panic on either.
//! * **`is_valid` is a result, not an error** (BL-ST-05): a file that fails its checks is a successful read with a verdict.
//! * **Every violation is kept.** A `ValidationError` lists all the violations it found, each with where it was found, not the first one.
//! * **Locations are neutral:** a layer, a laser, a channel and the operation, never a group or path of a file.

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

use crate::identity::{ChannelKey, LaserKey, LayerId};

/// Where something happened: the layer, laser and channel it concerns and the operation that was running. Every part is optional.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Location {
    pub layer: Option<LayerId>,
    pub laser: Option<LaserKey>,
    pub channel: Option<ChannelKey>,
    /// What was being done, such as `read_range` or `append`.
    pub operation: Option<String>,
}

impl Location {
    pub fn new() -> Self {
        Location::default()
    }

    pub fn with_layer(mut self, layer: LayerId) -> Self {
        self.layer = Some(layer);
        self
    }

    pub fn with_laser(mut self, laser: LaserKey) -> Self {
        self.laser = Some(laser);
        self
    }

    pub fn with_channel(mut self, channel: ChannelKey) -> Self {
        self.channel = Some(channel);
        self
    }

    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Does it say nothing at all?
    pub fn is_empty(&self) -> bool {
        self.layer.is_none()
            && self.laser.is_none()
            && self.channel.is_none()
            && self.operation.is_none()
    }
}

impl fmt::Display for Location {
    /// The parts that are there, in the order layer, laser, channel, operation: `layer 3, laser 2, channel position/x, operation read_range`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(layer) = &self.layer {
            parts.push(format!("layer {layer}"));
        }
        if let Some(laser) = &self.laser {
            parts.push(format!("laser {laser}"));
        }
        if let Some(channel) = &self.channel {
            parts.push(format!("channel {channel}"));
        }
        if let Some(operation) = &self.operation {
            parts.push(format!("operation {operation}"));
        }
        write!(f, "{}", parts.join(", "))
    }
}

/// One thing that is wrong, with where it was found.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Violation {
    pub location: Location,
    pub message: String,
}

impl Violation {
    pub fn new(message: impl Into<String>) -> Self {
        Violation {
            location: Location::default(),
            message: message.into(),
        }
    }

    pub fn at(mut self, location: Location) -> Self {
        self.location = location;
        self
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.location.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{} ({})", self.message, self.location)
        }
    }
}

/// Who or what is at fault: it decides what a caller can do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// The caller asked for something that cannot be done: a bug on the calling side, or a request the version or the handle does not allow.
    Caller,
    /// The data or the request breaks a rule: something absent, a value that does not fit, a check that failed.
    Validation,
    /// The environment failed (a disk, a device, a memory budget), which is not the caller's bug.
    Io,
    /// The library or the file itself is at fault: a library bug or a damaged file.
    LibraryOrFile,
}

/// What went wrong. The class and the default retry flag of each kind are in plan Appendix C.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The file's version has no adapter. Versions are spelled as the adapter registry spells them (the integer as text).
    UnsupportedVersion {
        found: String,
        supported: Vec<String>,
    },
    /// A feature or direction the version does not offer (for example creating a file of a version that is read only); the feature is named.
    UnsupportedInVersion { version: String, feature: String },
    /// The file carries no version marker, so no adapter can be chosen for it (BL-VA-01). It is a typed outcome of its own so that a caller can tell it from
    /// every other failure and, if it knows what the file is, ask for a probe explicitly; `detail` says where the marker was looked for.
    Unversioned { detail: String },
    /// An optional group or field is absent in the file.
    NotPresent { what: String },
    /// One or more violations, every one of them listed.
    ValidationError { violations: Vec<Violation> },
    /// A read or write failed. `transient` says whether the cause can pass (a busy device) or cannot (a full disk).
    IoError {
        os_code: Option<i32>,
        detail: String,
        transient: bool,
    },
    /// A layer, laser, channel or range outside what exists.
    InvalidIndex { what: String },
    /// A handle used after it was closed.
    Closed,
    /// The caller's token fired.
    Cancelled,
    /// A value does not fit the width it would be stored in, and would have been narrowed.
    OutOfWidth { value: String, stored_as: String },
    /// The memory or in-flight budget that the application set was exceeded.
    BudgetExceeded { limit_bytes: u64, needed_bytes: u64 },
    /// The file is damaged or breaks the rules of its own version.
    Corrupt { detail: String },
}

impl ErrorKind {
    pub fn class(&self) -> ErrorClass {
        match self {
            ErrorKind::UnsupportedVersion { .. }
            | ErrorKind::UnsupportedInVersion { .. }
            | ErrorKind::InvalidIndex { .. }
            | ErrorKind::Closed
            | ErrorKind::Cancelled => ErrorClass::Caller,
            ErrorKind::NotPresent { .. }
            | ErrorKind::Unversioned { .. }
            | ErrorKind::ValidationError { .. }
            | ErrorKind::OutOfWidth { .. } => ErrorClass::Validation,
            ErrorKind::IoError { .. } | ErrorKind::BudgetExceeded { .. } => ErrorClass::Io,
            ErrorKind::Corrupt { .. } => ErrorClass::LibraryOrFile,
        }
    }

    /// Can trying again help? Cancelling and a budget can be retried, an I/O failure when its cause is transient, and nothing else.
    pub fn default_retryable(&self) -> bool {
        match self {
            ErrorKind::Cancelled | ErrorKind::BudgetExceeded { .. } => true,
            ErrorKind::IoError { transient, .. } => *transient,
            _ => false,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::UnsupportedVersion { found, supported } => {
                let list = if supported.is_empty() {
                    "none".to_string()
                } else {
                    supported.join(", ")
                };
                write!(f, "unsupported version {found} (supported: {list})")
            }
            ErrorKind::UnsupportedInVersion { version, feature } => {
                write!(f, "{feature} is not supported in version {version}")
            }
            ErrorKind::Unversioned { detail } => {
                write!(f, "the file has no version marker: {detail}")
            }
            ErrorKind::NotPresent { what } => write!(f, "{what} is not present in the file"),
            ErrorKind::ValidationError { violations } => {
                let plural = if violations.len() == 1 { "" } else { "s" };
                write!(
                    f,
                    "validation failed with {} violation{plural}",
                    violations.len()
                )?;
                for (i, violation) in violations.iter().enumerate() {
                    write!(f, "{} {violation}", if i == 0 { ":" } else { ";" })?;
                }
                Ok(())
            }
            ErrorKind::IoError {
                os_code, detail, ..
            } => match os_code {
                Some(code) => write!(f, "I/O error (os error {code}): {detail}"),
                None => write!(f, "I/O error: {detail}"),
            },
            ErrorKind::InvalidIndex { what } => write!(f, "invalid index: {what}"),
            ErrorKind::Closed => write!(f, "the handle is closed"),
            ErrorKind::Cancelled => write!(f, "cancelled"),
            ErrorKind::OutOfWidth { value, stored_as } => {
                write!(f, "the value {value} does not fit {stored_as}")
            }
            ErrorKind::BudgetExceeded {
                limit_bytes,
                needed_bytes,
            } => write!(
                f,
                "budget exceeded: {needed_bytes} bytes needed, {limit_bytes} allowed"
            ),
            ErrorKind::Corrupt { detail } => write!(f, "corrupt file: {detail}"),
        }
    }
}

/// An error of the library: what went wrong, where, whether trying again can help, and the cause when there is one.
///
/// It is a plain value that can be cloned, sent and shared. It keeps no state beyond itself. Its parts are in a box, so that an `Error` is one pointer wide
/// and a `Result<T, Error>` stays small on the paths that do not fail.
// constructed-by: library
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Error {
    inner: Box<Inner>,
}

#[derive(Debug, Clone)]
struct Inner {
    kind: ErrorKind,
    location: Location,
    retryable: bool,
    source: Option<Arc<dyn StdError + Send + Sync + 'static>>,
}

impl Error {
    /// An error of this kind, with no location and no cause, and the default retry flag of the kind.
    pub fn new(kind: ErrorKind) -> Self {
        let retryable = kind.default_retryable();
        Error {
            inner: Box::new(Inner {
                kind,
                location: Location::default(),
                retryable,
                source: None,
            }),
        }
    }

    pub fn at(mut self, location: Location) -> Self {
        self.inner.location = location;
        self
    }

    /// Records the error that caused this one, reached through [`std::error::Error::source`].
    pub fn with_source(mut self, source: impl StdError + Send + Sync + 'static) -> Self {
        self.inner.source = Some(Arc::new(source));
        self
    }

    /// Replaces the default retry flag of the kind, when the cause says otherwise.
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.inner.retryable = retryable;
        self
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.inner.kind
    }

    pub fn location(&self) -> &Location {
        &self.inner.location
    }

    pub fn class(&self) -> ErrorClass {
        self.inner.kind.class()
    }

    pub fn is_retryable(&self) -> bool {
        self.inner.retryable
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner.kind)?;
        if !self.inner.location.is_empty() {
            write!(f, " [{}]", self.inner.location)?;
        }
        Ok(())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner
            .source
            .as_deref()
            .map(|e| e as &(dyn StdError + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn violation(message: &str, layer: u32) -> Violation {
        Violation::new(message).at(Location::new().with_layer(LayerId(layer)))
    }

    #[test]
    fn validation_error_lists_every_violation() {
        let error = Error::new(ErrorKind::ValidationError {
            violations: vec![
                violation("start is after end", 3),
                violation("sample count is missing", 4),
                Violation::new("no lasers"),
            ],
        });
        let text = error.to_string();
        assert!(text.contains("3 violations"), "{text}");
        for part in [
            "start is after end",
            "sample count is missing",
            "no lasers",
            "layer 3",
            "layer 4",
        ] {
            assert!(text.contains(part), "{part} is missing from: {text}");
        }
        // one violation is not pluralised
        let one = Error::new(ErrorKind::ValidationError {
            violations: vec![Violation::new("x")],
        });
        assert!(one.to_string().contains("1 violation:"), "{one}");
    }

    #[test]
    fn every_kind_has_a_class_and_a_default_retryable_flag() {
        use ErrorClass::{Caller, Io, LibraryOrFile, Validation};
        // the table of plan Appendix C
        let table = [
            (
                ErrorKind::UnsupportedVersion {
                    found: "19".into(),
                    supported: vec!["17".into()],
                },
                Caller,
                false,
            ),
            (
                ErrorKind::UnsupportedInVersion {
                    version: "16".into(),
                    feature: "create".into(),
                },
                Caller,
                false,
            ),
            (
                ErrorKind::NotPresent { what: "x".into() },
                Validation,
                false,
            ),
            (
                ErrorKind::Unversioned {
                    detail: "no marker on any mount".into(),
                },
                Validation,
                false,
            ),
            (
                ErrorKind::ValidationError {
                    violations: vec![Violation::new("x")],
                },
                Validation,
                false,
            ),
            (
                ErrorKind::IoError {
                    os_code: Some(28),
                    detail: "disk full".into(),
                    transient: false,
                },
                Io,
                false,
            ),
            (
                ErrorKind::IoError {
                    os_code: None,
                    detail: "busy".into(),
                    transient: true,
                },
                Io,
                true,
            ),
            (
                ErrorKind::InvalidIndex {
                    what: "layer 9".into(),
                },
                Caller,
                false,
            ),
            (ErrorKind::Closed, Caller, false),
            (ErrorKind::Cancelled, Caller, true),
            (
                ErrorKind::OutOfWidth {
                    value: "300".into(),
                    stored_as: "u8".into(),
                },
                Validation,
                false,
            ),
            (
                ErrorKind::BudgetExceeded {
                    limit_bytes: 10,
                    needed_bytes: 20,
                },
                Io,
                true,
            ),
            (
                ErrorKind::Corrupt { detail: "x".into() },
                LibraryOrFile,
                false,
            ),
        ];
        for (kind, class, retryable) in table {
            let error = Error::new(kind.clone());
            assert_eq!(error.class(), class, "{kind:?}");
            assert_eq!(error.is_retryable(), retryable, "{kind:?}");
            assert!(!error.to_string().is_empty());
        }
        // the flag can be overridden when the cause says otherwise
        assert!(Error::new(ErrorKind::Closed)
            .with_retryable(true)
            .is_retryable());
    }

    #[test]
    fn unversioned_is_a_validation_error_that_cannot_be_retried() {
        let error = Error::new(ErrorKind::Unversioned {
            detail: "looked for a marker on 2 mounts".into(),
        });
        // a missing marker is a fact about the file, not the caller's mistake and not a failing device: asking again cannot change it
        assert_eq!(error.class(), ErrorClass::Validation);
        assert!(!error.is_retryable());
        // and it reads as what it is, with its detail, so that a caller can tell it from a version nobody registered
        let text = error.to_string();
        assert!(
            text.contains("no version marker") && text.contains("2 mounts"),
            "{text}"
        );
        assert!(!matches!(
            error.kind(),
            ErrorKind::UnsupportedVersion { .. }
        ));
    }

    #[test]
    fn error_location_names_layer_laser_channel_and_operation() {
        let location = Location::new()
            .with_layer(LayerId(3))
            .with_laser(LaserKey::new("b", 2))
            .with_channel(ChannelKey::new(["position", "x"]))
            .with_operation("read_range");
        let error = Error::new(ErrorKind::InvalidIndex {
            what: "samples 10 to 20".into(),
        })
        .at(location.clone());
        assert_eq!(error.location(), &location);
        let text = error.to_string();
        for part in [
            "layer 3",
            "laser 2",
            "channel position/x",
            "operation read_range",
            "samples 10 to 20",
        ] {
            assert!(text.contains(part), "{part} is missing from: {text}");
        }
        // no location, no brackets
        assert_eq!(
            Error::new(ErrorKind::Closed).to_string(),
            "the handle is closed"
        );
        assert!(Location::new().is_empty() && !location.is_empty());
    }

    #[test]
    fn an_error_is_one_pointer_wide_so_a_result_stays_small() {
        assert_eq!(
            std::mem::size_of::<Error>(),
            std::mem::size_of::<usize>(),
            "clippy::result_large_err: every Result<_, Error> of the library pays for this"
        );
    }

    #[test]
    fn errors_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Error>();
        assert_send_sync::<ErrorKind>();
        assert_send_sync::<Location>();
        assert_send_sync::<Violation>();
        assert_send_sync::<crate::identity::Availability>();
        // and the error is a std error, usable in a Box<dyn Error + Send + Sync>
        let boxed: Box<dyn StdError + Send + Sync> = Box::new(Error::new(ErrorKind::Closed));
        assert_eq!(boxed.to_string(), "the handle is closed");
    }

    #[test]
    fn unsupported_in_version_names_the_feature() {
        let error = Error::new(ErrorKind::UnsupportedInVersion {
            version: "16".into(),
            feature: "creating a file".into(),
        });
        let text = error.to_string();
        assert!(
            text.contains("creating a file") && text.contains("16"),
            "{text}"
        );
        assert_eq!(error.class(), ErrorClass::Caller);
    }

    #[test]
    fn an_error_reports_its_cause_and_clones_with_it() {
        let cause = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let error = Error::new(ErrorKind::IoError {
            os_code: Some(13),
            detail: "open".into(),
            transient: false,
        })
        .with_source(cause);
        let source = StdError::source(&error).expect("the cause is kept");
        assert!(source.to_string().contains("denied"));
        assert!(StdError::source(&error.clone()).is_some());
        assert!(StdError::source(&Error::new(ErrorKind::Closed)).is_none());
        assert!(error.to_string().contains("os error 13"));
    }
}
