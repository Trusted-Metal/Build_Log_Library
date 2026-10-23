//! The adapters (IP-04 and the plans of the versions): **one folder for each on-disk version** (BL-VA-05), and the registration unit that each of them hands to
//! the registry.
//!
//! * **[`set`]** holds [`AdapterSet`], the registration unit of one version (BL-VA-03, BL-AR-10): its key, where its marker is, what it can do, and its
//!   reader, writer and patcher, each provided or declared unsupported. It is the only file here that is not a version.
//! * **A folder is a version.** Each one (the mock version, `mock/`, now; V17, V18 and V16 with IP-13, IP-05 and IP-12) holds that version's layout
//!   knowledge and nothing else, **reads every path relative to its mount** (decision D04-9), and **never names another version's module**; a guard scans
//!   the folders for it (phase 4E). Adding a version is a new folder and one line in the production list of the registry.
//! * **Internal:** hidden from the documentation; a consumer reaches an adapter only through the facade.

pub mod mock;
pub mod set;

pub use set::{AdapterSet, AdapterSetBuilder, MarkerPlace, ProbeFn, ReaderFn, Surface};
