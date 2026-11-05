//! The toolkit of the adapters (IP-12 phase 12A, decision D12-1): the code that more than one version uses, and that **names no attribute, no group and no version**.
//!
//! * **Layer:** inside the IO layer, beside the primitives (`hdf5`), the column engine (`columns`) and the adapters (`adapters`). An adapter uses it and **passes its own names as
//!   data**: a table, a layout value or a function. It lives outside `adapters/` because every folder of that is one version (BL-VA-05), and a version must never need another's code.
//! * **[`values`]** reads a value in whichever form a file stored it (a scalar or an array of one, any width, variable or fixed text). **[`content`]** says what a file's own content
//!   says about a failure and about a type. **[`context`]** reads the build's context from a table of attributes. **[`lasers`]** reads each laser in each layer, with its channels and the
//!   rule for the length of its stream, from a layout value. **[`columns`]** says which dataset holds a laser's channel in a layer, and **[`grids`]** which holds a laser's correction grid. **[`source`]** is the source of a build's metadata
//!   and samples that the registry hands to a consumer.
//! * **What is not here:** anything a version spells (a name, a unit, an encoding) and anything only one version reads (its parameters, its events, its images). The tables and layout values
//!   that the adapters pass are in their `layout.rs` and `lasers.rs`.
//! * **The rule of admission (decision D12-12, 2026-11-02): a thing is here when two or more versions use it and it names nothing.** What one version needs stays in that version's folder until a
//!   second version needs it, and then it moves here (a refactor that the golden and each version's tests prove). **"Used by every version" is not the rule**: the laser reader is used by the
//!   layer-major versions (V16 and V17), and V18's lasers are laid out the other way round, with the layers below the laser and the channels in one group. The front door of the laser reader is
//!   layer-major; its body (the stream rule, the null dataspace, the merge of a channel into a laser) does not depend on the traversal, and **the first step of V18's adapter is to split the two**
//!   (IP-05), so that V18 uses the body and brings its own front door. If a part does not fit a new version, that version has its own and the toolkit is not bent to it.
//! * **The guards** (`tests/adapter_isolation.rs`): no layout name of any version and no version's module is named in this folder, no folder of `adapters/` is called `toolkit`, and **every public
//!   function of the toolkit is used by two adapters or by other code of the toolkit** (`the_toolkit_holds_only_what_two_versions_use`): a function that one version alone uses is in that version.

#[doc(hidden)]
pub mod columns;
#[doc(hidden)]
pub mod content;
#[doc(hidden)]
pub mod context;
#[doc(hidden)]
pub mod grids;
#[doc(hidden)]
pub mod lasers;
#[doc(hidden)]
pub mod source;
#[doc(hidden)]
pub mod values;
