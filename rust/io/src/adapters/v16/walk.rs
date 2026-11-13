//! How V16 reads its metadata for the toolkit's lazy source (IP-26 phase 26K, decision D26-19): the build's own level with the ids of its layers, one layer by itself,
//! and every layer. Each is the reader that the whole walk has always used ([`super::read`], [`super::layers`]), called for what is asked and nothing more.

use std::collections::BTreeMap;

use build_log_model::{BuildMeta, BuildOverview, Error, ErrorKind, LayerDetail, LayerId};
use hdf5::Group;

use super::{layers, read};
use crate::toolkit::meta::Walk;

/// The reader of a V16 mount: the mount, and the name of each layer's group, listed once when the file is opened.
pub struct V16Walk {
    mount: Group,
    names: BTreeMap<LayerId, String>,
}

impl V16Walk {
    /// Lists the layers of the mount (one listing, no layer opened). A mount without its layers' group is `Corrupt`.
    pub fn open(mount: &Group) -> Result<V16Walk, Error> {
        Ok(V16Walk {
            mount: mount.clone(),
            names: layers::layer_names(mount)?,
        })
    }
}

impl Walk for V16Walk {
    fn overview(&self) -> Result<BuildOverview, Error> {
        read::read_overview(&self.mount, self.names.keys().copied().collect())
    }

    fn layer(&self, id: LayerId) -> Result<LayerDetail, Error> {
        let name = self.names.get(&id).ok_or_else(|| {
            Error::new(ErrorKind::InvalidIndex {
                what: "a layer that the build does not hold".to_string(),
            })
        })?;
        layers::read_layer_detail(&self.mount, id, name)
    }

    fn full(&self) -> Result<BuildMeta, Error> {
        read::read_build(&self.mount)
    }
}
