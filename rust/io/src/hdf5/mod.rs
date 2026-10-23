//! HDF5 primitives (IP-03): layout-free helpers that read, and later write, every kind of attribute and dataset that the V16, V17 and V18 files hold.
//!
//! * **Layer:** the IO layer, below the adapters. An adapter knows *which* name to read and what the value *means*; a primitive reads or writes a value
//!   **together with the encoding it is stored in** (decision D03-2), so that nothing is narrowed, decoded or rewritten silently (BL-ST-07).
//! * **Layout-free (BL-ST-13):** no group, attribute or dataset name of any version appears in this module. A guard test (`tests/primitives_layout_guard.rs`,
//!   decision D03-11) fails if one does.
//! * **Internal:** the adapters of this crate and the integration tests use it. It is hidden from the documentation and is not part of the contract of the
//!   facade, whose public surface is the Model's types (BL-AR-03).
//!
//! # Where the parts will go (the phases of IP-03)
//!
//! | Module | What | Phase |
//! |---|---|---|
//! | [`value`] | `Encoding` and `Value`, the vocabulary of every other part | 3C (here) |
//! | [`attr`] | the attribute reader | 3D |
//! | [`dataset`] | dataset descriptors, compound and string datasets | 3E |
//! | [`range`] | ranged reads with cancel and progress | 3F |
//! | [`error`] | the mapping of the binding's errors into the library's taxonomy | 3G |
//! | [`file`] | opening a file: the libhdf5 minimum first, then the open with its failures mapped | 3G |
//! | [`extra`] | keeping what an adapter does not type: attributes with their stored form, and unmapped groups and datasets (IP-13) | 13B, 13D |
//!
//! The write half (`write`: the attribute writer, format bounds and the resizable appender) is scheduled with IP-08.

pub mod attr;
pub mod dataset;
pub mod error;
pub mod extra;
pub mod file;
pub mod range;
pub mod value;

pub use attr::read_attribute;
pub use dataset::{
    describe_dataset, read_name_value_rows, read_strings, Allocation, DatasetDescriptor,
    DatasetLayout, Fill, FillWhen, Filter, FilterKind, NameValue,
};
pub use error::from_binding;
pub use extra::{
    extra_attribute, keep_dataset_remainder, keep_group_remainder, keep_node,
    keep_other_attributes, keep_other_nodes, unread_attribute, MAX_DEPTH,
};
pub use file::{open_file, open_file_with_minimum};
pub use value::{
    strict_text, ByteOrder, Charset, Class, Encoding, Field, Padding, Shape, Stored, TextForm,
    TextLength, Value,
};
