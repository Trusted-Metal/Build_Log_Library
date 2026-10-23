//! A consumer of the library, written against the facade and nothing else (IP-04 phase 4F, task T04.12; requirements BL-AR-03 and BL-VA-06).
//!
//! This is the shape of the integration of an application (tauri, Vision): one call to open a file, and the Model's own types to use what comes back. There is
//! **no code here that depends on the version of the file**: [`describe`] is the same for every version, which is why a new version needs no change in a
//! consumer. The mock version stands in until a real adapter reads a real file; `main` makes a file of two mock versions and describes both with the same call.

use std::path::Path;

use build_log::{library_versions, open_with, testing, Error, Opened, Registry};

/// What an application does with an opened build. The file's version appears only as data, in the report.
pub fn describe(opened: &Opened) -> Result<String, Error> {
    let build = opened.source().build()?;
    let layers = opened.source().layers()?;
    Ok(format!(
        "{}\nbuild {} with {} layers",
        opened.report(),
        build.identity.build_id,
        layers.len()
    ))
}

/// Opens the file at `path` and describes it: all an application needs to know about a file it was handed.
pub fn run(registry: &Registry, path: &Path) -> Result<String, Error> {
    describe(&open_with(registry, path)?)
}

fn main() -> Result<(), Error> {
    println!("the library: {:?}", library_versions());
    let registry = testing::mock_registry(&["9001", "9002"])?;
    for key in ["9001", "9002"] {
        let path =
            std::env::temp_dir().join(format!("mock_consumer-{}-{key}.h5", std::process::id()));
        testing::write_mock_file(&path, key, "an example build", 3)?;
        let described = run(&registry, &path);
        let _ = std::fs::remove_file(&path);
        println!("{}", described?);
    }
    Ok(())
}
