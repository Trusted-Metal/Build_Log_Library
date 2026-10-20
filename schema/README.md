# schema/

The **language-neutral specification** of the Build Log Library: the common data model and the file schemas as data, from which each language binding is generated and against which a drift check runs.

**Nothing is here yet.** The specification, its generator and the drift check arrive with the spec work (IP-18, after the data model has settled; the choice of spec format is gate G6). Until then the Rust types in [`rust/model`](../rust/model) are the only definition.

## Rule

This folder is **top-level and stays outside every language directory** (`rust/` today, others later): the specification belongs to no language. It is a rule, with a guard of its own (IP-01 task T01.5): `schema/` is a top-level directory, and no directory named `schema` exists under `rust/`.
