//! The dispatch core (IP-04): peek at the version marker, choose the adapter registered for that version, hand over.
//!
//! * **Peek, then dispatch (BL-VA-02).** The dispatcher reads only the marker ([`marker`]), looks the version up in a registry ([`registry`]) and hands over.
//!   It holds no layout knowledge: **the place of the marker is data that each adapter set declares**, and no group or attribute name appears in this
//!   module (a guard test, `tests/primitives_layout_guard.rs`, scans it, decision D04-11).
//! * **The registry is data and is injectable (BL-VA-04).** Functions take `&Registry`; production uses [`registry::production`]; tests build their own.
//! * **Exact match (BL-VA-10).** A version is a string (decision G3) and is matched exactly: there is no ordering, no range and no "at least".
//! * **Internal:** hidden from the documentation. The facade's `open` is the contract of a consumer (phase 4F).
//!
//! # Where the parts will go (the phases of IP-04)
//!
//! | Module | What | Phase |
//! |---|---|---|
//! | [`marker`] | the marker reader and the version key | 4B |
//! | [`registry`] | the registry, the production list and exact-match resolution | 4C |
//! | [`open`] | `open_at` and `open`, the typed outcomes, the explicit probe | 4D |

pub mod marker;
pub mod open;
pub mod registry;

pub use marker::{peek_marker, VersionKey};
pub use open::{open, open_at, open_probing, Opened};
pub use registry::{production, resolve, resolve_for_create, Registry};
