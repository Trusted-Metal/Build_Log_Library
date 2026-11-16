//! The manifest member of an archive, `lbl-manifest.json`: what the archive says about itself. A reader trusts the archive's own directory and the HDF5 file for what they say; the manifest only names which member is which.

use build_log_model::{Error, ErrorKind};
use serde_json::{json, Map, Value};

/// The name of the manifest member.
pub const MANIFEST_MEMBER: &str = "lbl-manifest.json";

/// The version of the archive format that this crate reads and writes. A reader refuses an archive of a newer one.
pub const CONTAINER_VERSION: u64 = 1;

/// What the manifest says. Every key is optional but the version; keys that are not known are ignored by a reader and are not kept.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Manifest {
    /// The version of the archive format (`lbl_container_version`).
    pub container_version: u64,
    /// The member that holds the build's HDF5 file (`primary`). Absent: the first `*.h5` member that is not the machine configuration.
    pub primary: Option<String>,
    /// The member that holds the machine configuration file (`machine_config`). Absent: a member named `machine_config.h5`. This library never reads it.
    pub machine_config: Option<String>,
    /// The version of the build's file, as a hint (`schema_version`): the authority is the HDF5 file's own marker.
    pub schema_version: Option<i64>,
    /// The names of the other members (`attachments`).
    pub attachments: Vec<String>,
    /// What wrote the archive (`created_by`).
    pub created_by: Option<String>,
    /// When it was written, as text (`created_utc`).
    pub created_utc: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            container_version: CONTAINER_VERSION,
            primary: None,
            machine_config: None,
            schema_version: None,
            attachments: Vec::new(),
            created_by: None,
            created_utc: None,
        }
    }
}

fn corrupt(detail: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt {
        detail: detail.into(),
    })
}

impl Manifest {
    /// Reads a manifest from its JSON. A manifest that is not an object, or whose known keys have the wrong type, is `Corrupt`.
    pub fn from_json(bytes: &[u8]) -> Result<Manifest, Error> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| corrupt(format!("the manifest is not JSON: {e}")))?;
        let Value::Object(map) = value else {
            return Err(corrupt("the manifest is not a JSON object"));
        };
        let text = |key: &str| -> Result<Option<String>, Error> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(corrupt(format!("the manifest's `{key}` is not text"))),
            }
        };
        let container_version = match map.get("lbl_container_version") {
            None => CONTAINER_VERSION,
            Some(v) => v.as_u64().filter(|n| *n > 0).ok_or_else(|| {
                corrupt("the manifest's `lbl_container_version` is not a whole number above zero")
            })?,
        };
        let schema_version =
            match map.get("schema_version") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_i64().ok_or_else(|| {
                    corrupt("the manifest's `schema_version` is not a whole number")
                })?),
            };
        let attachments = match map.get("attachments") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_string).ok_or_else(|| {
                        corrupt("the manifest's `attachments` holds something that is not text")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(corrupt("the manifest's `attachments` is not a list")),
        };
        Ok(Manifest {
            container_version,
            primary: text("primary")?,
            machine_config: text("machine_config")?,
            schema_version,
            attachments,
            created_by: text("created_by")?,
            created_utc: text("created_utc")?,
        })
    }

    /// The manifest as JSON, as a writer stores it.
    pub fn to_json(&self) -> Vec<u8> {
        let mut map = Map::new();
        map.insert(
            "lbl_container_version".into(),
            json!(self.container_version),
        );
        map.insert("schema".into(), json!("lbl.buildlog"));
        if let Some(v) = &self.primary {
            map.insert("primary".into(), json!(v));
        }
        if let Some(v) = &self.machine_config {
            map.insert("machine_config".into(), json!(v));
        }
        if let Some(v) = self.schema_version {
            map.insert("schema_version".into(), json!(v));
        }
        map.insert("attachments".into(), json!(self.attachments));
        if let Some(v) = &self.created_by {
            map.insert("created_by".into(), json!(v));
        }
        if let Some(v) = &self.created_utc {
            map.insert("created_utc".into(), json!(v));
        }
        serde_json::to_vec_pretty(&Value::Object(map)).expect("a JSON value always serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_reads_back_what_it_wrote() {
        let manifest = Manifest {
            primary: Some("build.h5".into()),
            machine_config: Some("machine_config.h5".into()),
            schema_version: Some(18),
            attachments: vec!["logs/capture.log".into()],
            created_by: Some("test".into()),
            created_utc: Some("2026-11-13T15:28:13Z".into()),
            ..Manifest::default()
        };
        assert_eq!(Manifest::from_json(&manifest.to_json()).unwrap(), manifest);
    }

    #[test]
    fn unknown_keys_are_ignored_and_a_missing_version_is_the_first() {
        let m = Manifest::from_json(br#"{"primary":"b.h5","something_new":{"a":1}}"#).unwrap();
        assert_eq!(m.container_version, 1);
        assert_eq!(m.primary.as_deref(), Some("b.h5"));
    }

    #[test]
    fn a_manifest_with_a_key_of_the_wrong_type_is_corrupt() {
        for bad in [
            &br#"[1]"#[..],
            br#"{"primary": 3}"#,
            br#"{"lbl_container_version": 0}"#,
            br#"{"attachments": "x"}"#,
            br#"{"attachments": [1]}"#,
            b"not json",
        ] {
            let error = Manifest::from_json(bad).unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Corrupt { .. }),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
