# Build_Log Library — Implementation Plan

**Status: NOT STARTED (design revised).** This is a from-scratch plan for a new
sibling library, written before the library's repository exists. The original
draft assumed a behavior-preserving, single-file-path carve-out; this revision
folds in three rounds of design discussion since then — inspection of a real
reference fixture, the mechanics of how a future LBL dispatcher would actually
route between libraries, and how to keep clearbox-tauri's *current* single-file
output working while splitting its writer into separate libraries. See
`docs/MCF_INTEGRATION_PLAN.md` and `docs/MCF_OWNERSHIP_AUDIT.md` for the
sibling library this one is modeled on, and for the original (superseded)
single-LBL-library proposal this plan replaces.

## 0. Context

The original plan was a single "LBL" (per-build output) library owning
everything clearbox-tauri's capture writer produces. That is now split into
three domains, each with its own library:

1. **Machine_Configuration** — already `Machine_Config_Library` (MCF). Unchanged.
2. **Build_Log** — this plan. The x/y/z/power/state record of what the machine
   actually did during a build. Described by the requester as "the heart of
   capture."
3. **Sensors** — OPC UA telemetry today, possibly ClearBox-level sensors later.
   A separate future plan, explicitly out of scope here.

A thin **LBL dispatcher** will eventually sit above all three. Its mechanics
are discussed in Phase 4 below, as a recorded working assumption — it is
explicitly not being built now, but Build_Log's own design (Guiding Principles
10–11) is chosen so it stays compatible with that future surface rather than
fighting it later.

**Consumers of this library:**
- `clearbox-tauri` — the **writer**. Owns capture; will call this library
  during and after a live build.
- `clearbox_vision` — the **reader**. Visualizes/analyzes a finished build; per
  `crates/application/src/ports/build_loader.rs`, it never needs write access.

**Language:** Rust only. Both consumers are Rust. MCF's 5-language,
schema-code-generated setup is a large investment justified by MCF's need for
external config-authoring interchange; Build_Log has no such requirement today.
Build it hand-written, but keep the CDM/error/facade modules cleanly separated
from adapter internals (Guiding Principle 7) so a second language could be
added later without a rewrite, if a real consumer ever needs one.

**A real reference fixture exists.** A file named
`reference_LBL_final_no_manifest.h5` was inspected directly (via `h5dump`)
during planning. It is a *proposed target* layout, not today's production
layout, and several parts of this plan are grounded in what it actually
contains rather than assumption — those are marked "confirmed" below.
Everything else stays marked provisional. The filename's "no_manifest" is
itself a clue: it implies a manifest-based multi-file alternative was already
being weighed by whoever produced it, alongside the single-merged-file idea —
relevant context for Phase 4.

## Guiding Principles

1. **One crate, two capability traits — not two crates.** `BuildLogReader` and
   `BuildLogWriter` live in the same crate. A shared Common Data Model, shared
   on-disk layout definitions, and shared hash/canonicalization logic must have
   exactly one source of truth; splitting into separate reader/writer crates
   would force either duplicating that logic (silent drift risk) or pulling it
   into a third shared crate anyway, buying nothing. One crate also means one
   version number — no reader/writer compatibility matrix to track. Each
   consumer can still pin an independent git rev of the one crate, exactly as
   clearbox-tauri does with MCF today.
2. **The public facade is the only contract.** Consumers depend on
   `BuildLogReader`/`BuildLogWriter` and the Common Data Model only, never on a
   version-specific adapter type. Internal reorganization is free as long as
   that boundary's shape and behavior hold. Mirrors MCF's own stated rule that
   it "ends at its public facade."
3. **Build_Log owns exactly the per-layer capture record** — commanded and
   actual x/y/z, power, and state, per laser, per layer. Nothing else — in
   particular, **not images**: Pre_Exposure/Post_Exposure image capture is
   owned by the future Sensors library (decided; see Ownership Boundary and
   Open Decisions §7). See Ownership Boundary below. Do not let convenience
   pull sensor values, images, or calibration data into this library's scope,
   regardless of how tightly a sensor happens to be sampled to the laser
   clock (see Ownership Boundary's sharpened Sensors rule).
4. **Versioned adapters, dispatched by a version marker peeked before parsing**
   — same registry pattern as MCF's `capabilities/mod.rs`
   (`HashMap<&str, OpenFn>`/`HashMap<&str, CreateFn>`, with `resolve_open`/
   `resolve_create` taking the registry as a parameter so tests can inject
   fake entries). Adding a new version means a new adapter module and one
   registry entry; old versions are never touched.
5. **The on-disk layout adopts the new top-level `/Build_Log` structure from
   the start — this is not a behavior-preserving wrap of today's
   `/Built_Part/Monitoring_Data`.** Decided explicitly during planning ("do
   both at once"): the library carve-out and the layout migration happen
   together, not sequenced. Consequence: Phase 1's parity check against
   historical builds must be **semantic** (same layers, same values, same
   timing) rather than a byte-for-byte diff of file layout, since the paths
   are intentionally changing at the same time as the code that owns them.
6. **Hash-on-write, verify-on-read, from day one.** Ported from MCF's
   `hash.rs`/`is_valid()`, already proven working in this app. Recommended
   default: a mismatch is a non-fatal `is_valid: Some(false)` flag, not a hard
   error — see Open Decisions §1. **The hash must be computed purely from
   data values, never from absolute path or mount position.** This isn't
   just hygiene: Build_Log is designed to be mounted at different group
   depths depending on the caller (see Principle 10), and a possible future
   LBL surface may physically copy this data into a different file entirely
   (Phase 4). A hash that incorporated file position would break under both.
7. **No multi-language machinery now, but don't box it out.** CDM (`models.rs`),
   errors (`error.rs`), and the facade traits (`facade.rs`) stay physically
   separate from adapter internals (`capabilities/v1_0/*`) — so that codegen or
   another language binding could be retrofitted onto that boundary later
   without touching consumer-facing code.
8. **Migration is incremental and side-by-side in both consumer apps, never a
   big-bang cutover.** Each app runs its existing hand-rolled path alongside
   the new library on real captures until parity is proven, then — and only
   then — the old path is deleted. Per Principle 5, "parity" now means
   semantic equivalence of the captured data, not identical bytes on disk.
9. **Build_Log never depends on MCF, and MCF never depends on Build_Log.**
   Mirrors MCF's own stated principle. Only the future LBL dispatcher depends
   on both.
10. **The location-based constructor is the foundational primitive; the
    standalone-file constructor is a convenience wrapper around it.** In
    HDF5, a `File`'s root is itself a `Group` — so `BuildLogWriter`/
    `BuildLogReader` are designed around "operate against a `Group` I was
    handed," and "open/create a whole file by path" is implemented as: open
    the file, take its root group, delegate to the same `_at` constructor.
    This is what lets clearbox-tauri mount Build_Log's writer *inside* the one
    physical file it already owns — via a `Group` carved out by its existing
    single owning writer task — without ever creating a second independent
    HDF5 file session. See "Concurrency Model" under Public API below for why
    this is safe. It is also what keeps a possible future three-files-then-
    merge LBL design (Phase 4) from conflicting with today's single-file
    need: both are just different callers choosing a different entry point
    off the same primitive.
11. **Version-peek and hash logic operate relative to whatever `Group` the
    library is handed — never a hardcoded absolute path.** Required for
    Principle 10 to be safe: Build_Log must work identically whether it's
    mounted at `/Build_Log` in a file it created itself, or at a caller-chosen
    location inside a file someone else opened.
12. **Build_Log's own core only ever targets the new proposed layout — it
    never grows a mode for producing or accepting the old
    `/Built_Part/Monitoring_Data` shape.** Per Principle 5, there is no
    legacy-shape support built into this library. Compatibility with what
    clearbox-tauri produces today is a **future LBL library** concern, not
    Build_Log's and not clearbox-tauri's own application code — see Phase 4.

## Ownership Boundary

### Build_Log owns
- The build-output file's own header attributes (schema version, build
  id/name, timestamps, layer count, hash) — file-level metadata for the
  *build record*, distinct from MCF's machine-level metadata. **Confirmed
  against the reference fixture**, `/Build_Log`'s own group carries:
  `File_Version` (Build_Log's own version marker, scoped to its own group,
  not the bare file root — decided to use MCF's string convention, e.g.
  `"1.0"`, not the fixture's own raw integer counter — see Open Decisions §2),
  `Build_ID`, `Build_Status`, `Date`, `Layer_Quantity`, `Layer_Thickness`,
  `Number_Of_Parts`, `Alloys`, `Parameter_ID_For_Material`, `Scanner_ID`,
  `Starting_Height`, `Username`, and `Source_MCF_Configuration_Hash`/
  `Source_MCF_Export_Date` (a provenance *reference* to the MCF snapshot this
  build was recorded against — not a duplicate of MCF's data). **Not**
  `Opcua_Join_Status`/`Opcua_Join_Version` — decided as a Sensors library
  concern, not Build_Log's; the reference fixture's placement of them on
  `/Build_Log`'s root was an artifact of the pre-split design, not something
  this library's own layout carries forward. See Open Decisions §8.
- `/Build_Log/Monitoring_Data/Layer_N/Laser_N/*` — **confirmed against the
  reference fixture**: `Laser_Data/{Laser_Power, Laser_State}`,
  `Position/{Actual_X/Y/Z, Commanded_X/Y/Z}`. See Common Data Model below for
  exact dtypes.

### Stays with MCF — never duplicated here
- `/Machine_Configuration/*` (the reference fixture shows this as a full,
  native MCF-schema-shaped subtree — `Machine/Optical_Trains/...`,
  `Extensions/ClearBox/...`, `Extensions/TM_OPCUA/...` — not the hand-written
  subset mirror clearbox-tauri writes today into `/Built_Part/Calibration_Data`).
  Build_Log should reference this data (by hash, per
  `Source_MCF_Configuration_Hash`) without ever owning or reinterpreting it.

### Explicitly out of scope — future Sensors library
- `/Sensors/*` — **sharper rule than originally written, confirmed against the
  reference fixture**: Sensors owns *every* sensor value time series,
  regardless of acquisition method or sampling rate. A ClearBox-native sensor
  sampled at full per-point laser rate (`Oxygen_Sensor_ClearBox_Optical_Train_1`
  — `f64`, 727,922 samples in one layer, matching the laser sample count
  exactly, no independent timestamp) lives under `/Sensors` exactly like a
  sparse, asynchronous OPC UA signal (`Chamber_Oxygen_Sensor_OPCUA` — string
  values, 11 samples, each with its own `Source_ts`). Build_Log owns strictly
  motion + power + state, never a sensor value, no matter how tightly it's
  sampled to the laser clock. **Do not add a sensor-append method to
  `BuildLogWriter` "for convenience."**
- Sensor *calibration/definition* metadata stays MCF's job even when the
  sensor is ClearBox-native — confirmed via `/Sensors/.../Definition_Path`,
  a string attribute pointing back into
  `/Machine_Configuration/Extensions/ClearBox/Optical_Train_0N/Sensors/<name>`.
  Reference by path, don't duplicate — the same discipline Build_Log's own
  `Source_MCF_Configuration_Hash` field follows.
- **Decided: Pre_Exposure/Post_Exposure image capture belongs to Sensors, not
  Build_Log** (see Open Decisions §7 — this reverses this plan's earlier
  recommendation). Confirmed technical facts, carried forward for whoever
  builds Sensors: `u8`, fixed 2D shape (e.g. `5068×5064`, ~25MB/image),
  tagged with the standard HDF5 Image Specification attributes
  (`CLASS=IMAGE`, `IMAGE_SUBCLASS=IMAGE_GRAYSCALE`, `IMAGE_VERSION=1.2`); a
  dedicated heavy accessor (never eagerly loaded with a plain per-layer read)
  will be needed there for the same size reasons this plan originally raised
  for Build_Log. This also removes an awkward split the earlier draft
  flagged: the OPC UA snapshot *trigger event* (`CameraSnapshotCommand_1`)
  and the pixel *payload* now both live under Sensors, rather than being
  split across two libraries.

### Stays app-owned legacy, or unresolved — do not assume
- clearbox-tauri's `/Configuration/sensor_configurations` — app-owned, outside
  MCF's and Sensors' schema (per `docs/MCF_OWNERSHIP_AUDIT.md`).
- **clearbox-tauri's `/Configuration/build_configuration` (`BuildConfig`) is
  no longer a confident "stays legacy" call.** Its fields (`build_id`,
  `username`, `layer_count`, `num_of_parts`, `alloy`, `material_id`,
  `starting_height`, `layer_thick`) overlap substantially with `/Build_Log`'s
  own root attributes in the reference fixture. See Open Decisions §6 — not
  resolved, do not assume either way.
- clearbox_vision's melt-pool/camera data (`domain::entities::Layer.image_sets`)
  and any `processed`/`smoothed`/`analog_sensors`/`digital_sensors` fields on
  its `Laser` entity (`crates/domain/src/entities/laser.rs:36-44`) — these are
  clearbox_vision's own downstream analysis outputs. Build_Log supplies only
  the raw commanded/actual/power/state values (and possibly Pre/Post-Exposure
  images — see Open Decisions §7) that seed them.

## Common Data Model

Fields marked **confirmed** come directly from `h5dump` against the reference
fixture. Fields marked **provisional** are still sketches, pending the open
decisions referenced.

```rust
pub struct BuildLogMeta {
    pub schema_version: String,          // decided: String, matching MCF's "1.0"/"1.1"
                                          // convention — diverges deliberately from the
                                          // raw reference fixture's own integer counter
                                          // (File_Version: 18), which was that file's own
                                          // ad hoc versioning, not Build_Log's v1.0 identifier
    pub build_id: String,                 // resolved (Open Decisions §6): absorbed from legacy BuildConfig
    pub build_status: String,             // confirmed on-disk; not a BuildConfig field, net-new
    pub username: String,                 // resolved (§6): absorbed from legacy BuildConfig
    pub layer_quantity: u32,               // resolved (§6): absorbed from legacy BuildConfig.layer_count
    pub number_of_parts: u32,              // resolved (§6): absorbed from legacy BuildConfig.num_of_parts
    pub alloys: String,                    // resolved (§6): absorbed from legacy BuildConfig.alloy
    pub parameter_id_for_material: String, // resolved (§6): absorbed from legacy BuildConfig.material_id
    pub starting_height: f32,              // resolved (§6): absorbed from legacy BuildConfig
    pub layer_thickness: f32,              // resolved (§6): absorbed from legacy BuildConfig.layer_thick
    pub scanner_id: String,                // confirmed on-disk; not a BuildConfig field
    pub date: String,                      // confirmed on-disk; not a BuildConfig field
    pub source_mcf_configuration_hash: Option<String>, // confirmed: provenance reference, not owned data
    pub source_mcf_export_date: Option<String>,        // confirmed
    // NOT here, decided (Open Decisions §6, resolved): build_plate_thick/
    // build_plate_weight are MCF's territory; est_build_time/
    // est_build_time_unit/save_bin_data are not Build_Log's concern.
    pub build_log_hash: String,          // net-new; not present in the reference fixture today
    pub is_valid: Option<bool>,          // net-new; computed fresh on every read, mirrors MCF
    pub extra: serde_json::Map<String, serde_json::Value>, // forward-compat passthrough
}

pub struct Layer {
    pub index: u32,
    pub start_time: i64,                 // confirmed: attribute, not dataset
    pub end_time: i64,                    // confirmed: attribute, not dataset
    pub actual_layer_thickness: f32,      // OPEN QUESTION — may not belong under
                                          // Build_Log's Layer at all; see Open Decisions §10
    pub commanded_layer_thickness: f32,   // OPEN QUESTION — same as above, see Open Decisions §10
    pub lasers: Vec<LaserLayerRecord>,
}

pub struct LaserLayerRecord {
    pub laser_index: u32,
    pub start_time: i64,                  // confirmed: per-laser, distinct from Layer's own timing
    pub end_time: i64,                     // confirmed
    pub complete: bool,                    // confirmed: attribute, per laser
    pub state: Vec<LaserState>,            // confirmed dtype: u8 on disk
    pub power: Vec<f32>,                   // confirmed dtype: f32, NOT f64
    pub commanded: Vec<[i32; 3]>,          // confirmed dtype: i32 (raw scanner counts), NOT f64 —
                                            // unit conversion to physical mm needs MCF's bit-resolution/
                                            // field-size data and is the CONSUMER's job, not Build_Log's
    pub actual: Vec<[i32; 3]>,             // confirmed dtype: i32, same reasoning
}
```

Datasets backing `state`/`power`/`commanded`/`actual` are extendable
(`H5S_UNLIMITED` max dim) in the reference fixture — consistent with a
streaming-append writer, not a write-once layout.

`extra`/forward-compat passthrough mirrors MCF's `MachineConfigMeta.extra` —
the same mechanism that let MCF add `is_valid` without becoming a breaking
change for every existing caller.

**Images:** decided — Pre_Exposure/Post_Exposure image capture is not part of
Build_Log's CDM at all. It belongs to the future Sensors library (see
Ownership Boundary and Open Decisions §7).

**Layer thickness fields, flagged, not decided:** `actual_layer_thickness`/
`commanded_layer_thickness` are confirmed present on-disk (as `Layer_N`
attributes in the reference fixture), but whether they belong in Build_Log's
`Layer` struct at all is now an open question, not an assumption — see Open
Decisions §10. Not an MCF concern — layer thickness is set at the build
level, which is Build_Log's own territory (`BuildLogMeta.layer_thickness`,
confirmed as `/Build_Log`'s root `Layer_Thickness` attribute — see Ownership
Boundary). `commanded_layer_thickness` at the per-layer level is likely pure
redundancy of that same build-level value, not new information.
`actual_layer_thickness` is the real open question — whether/how it's
populated and where it should live. Not resolved — do not build against
either field yet.

## Public API — Facade Traits

```rust
pub trait BuildLogReader: Send {
    fn get_meta(&self) -> Result<BuildLogMeta, BuildLogError>;
    fn layer_count(&self) -> Result<u32, BuildLogError>;
    fn get_layer(&mut self, index: u32) -> Result<Layer, BuildLogError>;
    fn is_valid(&self) -> bool;
    fn close(self: Box<Self>);
}

// Location-based primitive (Guiding Principle 10) — this is what clearbox-tauri
// actually calls, attaching to a Group carved out of a file it already owns.
pub fn open_build_log_at(location: &Group) -> Result<Box<dyn BuildLogReader>, BuildLogError>;

// Convenience wrapper — opens its own whole file, then delegates.
pub fn open_build_log(path: &Path) -> Result<Box<dyn BuildLogReader>, BuildLogError> {
    let file = hdf5::File::open(path)?;
    open_build_log_at(&file) // File derefs to its root Group
}
```

Either constructor returns a stateful handle — repeated `get_layer()` calls on
the same handle reuse the open location and any cached header data, which is
what clearbox_vision's existing `BuildLoader::open_layer_context`/
`load_layer_with_context` exists to provide; this library should make that the
*default* behavior of a single open handle rather than requiring a second,
separate context type.

```rust
pub trait BuildLogWriter: Send {
    fn set_meta(&mut self, meta: BuildLogMeta) -> Result<(), BuildLogError>;
    fn append_layer(&mut self, layer: Layer) -> Result<(), BuildLogError>;
    fn append_layers(&mut self, layers: &[Layer]) -> Result<(), BuildLogError>; // batch
    fn save(self: Box<Self>) -> Result<(), BuildLogError>; // computes+writes hash, closes
}

pub fn create_build_log_at(location: &Group, version: &str)
    -> Result<Box<dyn BuildLogWriter>, BuildLogError>;

pub fn create_build_log(path: &Path, version: &str) -> Result<Box<dyn BuildLogWriter>, BuildLogError> {
    let file = hdf5::File::create(path)?;
    create_build_log_at(&file, version)
}
```

`append_layers` exists because clearbox-tauri's current writer already batches
(`hdf5/writer.rs`'s "streaming batches match single legacy write" test,
`hdf5/async_writer.rs`) — the trait is designed against that real existing
access pattern.

**Recommendation: keep both traits synchronous**, matching MCF and the
underlying `hdf5` crate's own sync API. Let clearbox-tauri's existing
`async_writer.rs` continue to own the async boundary — see Open Decisions §4.

### Concurrency Model for the clearbox-tauri Integration

This directly addresses "isn't one-file-multiple-writers unsafe in HDF5?" —
yes, it is, and this design doesn't do that. The distinction that matters is
between an **HDF5-level writer** (an actual open file handle doing I/O — there
must be exactly one of these touching the file at any moment) and a
**library-API writer** (a typed Rust object with `append_*` methods — there
can be as many of these as convenient, since they're just method dispatch).

```
 Capture Pipeline ──┐
  (produces layers) │
                     ├──► [ one mpsc channel ] ──► ONE Owning Writer Task
 Telemetry Bridge ───┘        (today: async_writer.rs                    │
  (future Sensors               or its successor)                        │
   producer)                                                              │
                                                          owns the ONE real│
                                                          hdf5::File handle
                                                                           │
                                     ┌─────────────────────────────────────┴───┐
                                     │                                          │
                           BuildLogWriter                               (future) SensorsWriter
                           (create_build_log_at,                        (same pattern,
                            attached to Group                            attached to Group
                            "/Build_Log")                                 "/Sensors")
                                     │                                          │
                                     ▼                                          ▼
                          writes under /Build_Log/...              writes under /Sensors/...
                                     │                                          │
                                     └─────────────────┬────────────────────────┘
                                                        ▼
                                       the ONE physical .h5 file on disk
```

The load-bearing fact: a `Group` obtained from an already-open `File`
(`file.group("Build_Log")` or `file.create_group("Build_Log")`) is **not** a
new file handle — it's a navigational pointer into the same open session.
`BuildLogWriter`/`SensorsWriter` never call `hdf5::File::open`/`create`
themselves in this mode; their only entry point is the `_at` constructor,
which only ever accepts a `Group` that already exists. The one owning task
drains its channel one message at a time — a layer-complete message triggers
exactly one call into `BuildLogWriter`, a sensor event triggers exactly one
call into the future `SensorsWriter` — so at any instant exactly one HDF5
mutation is in flight, performed by the one task that holds the one handle.
This is materially different from (and does not require relaxing HDF5's rule
against) two independent tasks each independently calling
`hdf5::File::open(path)` on the same path concurrently — that remains unsafe
and this design never does it.

## Versioning & Dispatch

Two independent dispatch layers exist, at different altitudes, and must not be
conflated:

- **Inside Build_Log** (this library's own concern): a version marker
  attribute, peeked from whatever `Group` the library was handed (never an
  assumed absolute path — Guiding Principle 11), before full parsing.
  `HashMap<&str, OpenFn>` / `HashMap<&str, CreateFn>` registries;
  `resolve_open`/`resolve_create` take the registry as a parameter so tests
  can inject fake entries — copied directly from MCF's `capabilities/mod.rs`.
  Each version's module (`capabilities/v1_0/`) contains **both** its reader
  and its writer together (`layout.rs`, `reader.rs`, `writer.rs`).
- **Above all three libraries** (a future LBL dispatcher's concern, not this
  library's — see Phase 4): routing by *group name* (`Build_Log` → this
  library, `Machine_Configuration` → MCF, `Sensors` → the future Sensors
  library), which is a stable, essentially permanent mapping, orthogonal to
  any of the three libraries' own internal content-version dispatch.

**Confirmed against the reference fixture:** `/Build_Log`'s version attribute
lives scoped to its own group (not the bare HDF5 file root), which is exactly
what Guiding Principle 11 requires and resolves what was previously an open
question (see Open Decisions §5).

Adding v1.1 later: new `capabilities/v1_1/` folder, one new registry entry,
v1.0 untouched — copy MCF's "adding a new file version" checklist from its
`docs/contributing.md`.

## Hash & Integrity

- Port MCF's `hash.rs` approach directly: canonicalize the CDM (sorted-key
  JSON, excluding only `build_log_hash` itself) → SHA-256 → store as a header
  attribute. With images now out of scope (decided — see Ownership Boundary),
  Build_Log's CDM has no oversized binary payload to exclude the way MCF
  excludes its correction grids — the position/power/state arrays are exactly
  the content a manufacturing build record needs hash coverage over, so they
  should stay included, not excluded for size reasons.
- `is_valid()` recomputes fresh on every call, no caching — matches MCF.
- **Must be computed purely from data values** — see Guiding Principle 6.
  Never fold in the `Group`'s absolute path or any file-level positioning.
- **Decided:** a hash mismatch is non-fatal (`is_valid: Some(false)`, caller
  decides) — exactly as MCF does. See Open Decisions §1.

## Error Model

Single `BuildLogError` enum (`thiserror`), wrapping `hdf5::Error` and
`serde_json::Error`, plus domain variants: `LayerNotFound(u32)`,
`UnsupportedVersion(String)`, `MissingGroup(String)`, `Closed`. No
`HashMismatch` variant — consistent with the non-fatal `is_valid` design above.

## Repo & Crate Scaffolding

- New repository, sibling to `Machine_Config_Library`, `clearbox-tauri`, and
  `clearbox_vision` — e.g. `C:\Users\ChrisParham\Desktop\Repo\Build_Log_Library`.
- `hdf5 = { package = "hdf5-metno", version = "0.12" }` — must match the exact
  version both consumer apps already pin, to avoid duplicate-version symbol
  conflicts in either app's dependency graph.
- Module layout:
  - `src/lib.rs`
  - `src/models.rs` — the CDM
  - `src/facade.rs` — `BuildLogReader`/`BuildLogWriter` trait definitions, plus
    the `_at` location-based constructors as the primary API and the
    `Path`-based convenience wrappers (Guiding Principle 10)
  - `src/error.rs`
  - `src/hash.rs`
  - `src/capabilities/mod.rs` — dispatch registry
  - `src/capabilities/v1_0/{layout.rs, reader.rs, writer.rs}`
- `fixtures/` — at least one real historical clearbox-tauri build for
  round-trip testing, plus a deliberately-tampered copy for the
  `is_valid() == false` path. Mirrors MCF's own `fixtures/` pair already
  validated working in this app.

## Testing Strategy

- Unit tests per v1.0 module (reader, writer, hash) in isolation.
- Round-trip test: write a CDM layer set, read it back, assert equality and
  `is_valid() == true`.
- Tamper test: mutate a written value without recomputing the hash, assert
  `is_valid() == false` **and** that the file still opens and layers are
  still readable (non-fatal, per Hash & Integrity above).
- **Group-attached-mode test** (the mode production clearbox-tauri actually
  uses): create a plain `hdf5::File`, carve out a named subgroup that is
  *not* the file root, call `create_build_log_at`/`open_build_log_at` against
  it, and confirm behavior is identical to the standalone-file mode — this is
  what proves Guiding Principles 10–11 actually hold, not just the simpler
  whole-file case.
- Integration test against a real historical clearbox-tauri build file —
  confirms v1.0's layout matches production reality. Per Guiding Principle 5,
  this is a semantic comparison (same layers, same values, same timing), not
  a byte-for-byte layout diff, since the on-disk paths are changing at the
  same time as the library carve-out.
- API-surface guard test: a test that only ever calls through the
  `BuildLogReader`/`BuildLogWriter` trait objects, never a version-specific
  type directly.

## Phased Implementation Plan

### Phase 0 — Library Core (NOT STARTED)
- **0a — Schema archaeology.** Confirm the remaining open dtype/shape
  questions not already settled by the reference fixture inspection, and
  resolve Open Decisions §6 (BuildConfig absorption) and §10 (layer-thickness
  placement) before finalizing the CDM.
- **0b** — Scaffold the new repo, `Cargo.toml`, module layout.
- **0c** — Implement the CDM per confirmed findings above.
- **0d** — Implement `v1_0::{layout, reader, writer}`, including both the
  `_at` location-based constructors and the `Path`-based wrappers.
- **0e** — Implement `hash.rs`/`is_valid()`, honoring the
  position-independence constraint (Guiding Principle 6).
- **0f** — Implement the dispatch registry and facade traits.
- **0g** — Fixtures and the full test suite above — including the
  Group-attached-mode test — green, before touching either consumer app.

### Phase 1 — clearbox-tauri Writer Integration (NOT STARTED)
- **1a** — Add the git/path dependency, same pattern used for MCF.
- **1b** — Identify (or introduce, if it doesn't already cleanly exist) the
  single owning writer task that holds the one `hdf5::File` handle for a
  build — today's capture pipeline and telemetry bridge should already be
  funneling through something like this; confirm rather than assume. Have it
  call `create_build_log_at(&group, version)` against a `/Build_Log` subgroup
  it carves out of the one file it owns, per the Concurrency Model above.
- **1c** — Run side-by-side with the existing hand-rolled
  `/Built_Part/Monitoring_Data` writer on real captures. Per Guiding
  Principle 5, compare **semantically** (layer-by-layer, laser-by-laser value
  equality) rather than diffing file bytes, since both the paths and the code
  are changing together.
- **1d** — Once parity is confirmed — including under streaming/batched
  writes, reconciled against `hdf5/async_writer.rs`'s existing behavior — cut
  the writer over to `/Build_Log` via the library exclusively.
  `/Machine_Configuration` (MCF) and `/Sensors` sections are untouched by this
  phase.
- **1e** — Optional: wire the library's reader into `hdf5/verify.rs`'s
  existing round-trip verification, using real `is_valid()`.

### Phase 2 — clearbox_vision Reader Integration (NOT STARTED)
- **2a** — New `infrastructure::hdf5::build_log_adapter` implementing the
  existing `BuildLoader` port (`crates/application/src/ports/build_loader.rs`)
  using `open_build_log` (the standalone-file wrapper — clearbox_vision reads
  a finished, closed file, so it has no need for the `_at` primitive).
- **2b** — Map the CDM to `domain::entities::{Build, Layer, Laser}` —
  `LaserLayerRecord` feeds `Laser.position`/`power`/`state`/`raw`;
  `processed`/`smoothed`/sensor fields are untouched, computed downstream by
  clearbox_vision itself. Note the dtype corrections above (position is
  `i32` raw scanner counts, not physical units — clearbox_vision's existing
  coordinate-conversion logic, which already needs MCF's calibration data,
  is the right place for that conversion, not this library).
- **2c** — Ship behind a flag/env var, A/B against the existing
  `v16_adapter.rs` path on real files.
- **2d** — Cut over once parity is confirmed. `application`/`domain`/
  `presentation` crates require zero changes, by construction of the existing
  `BuildLoader` port.

### Phase 3 — Cutover & Cleanup (NOT STARTED)
- **3a** — Delete clearbox-tauri's old hand-rolled Monitoring_Data write path
  once Phase 1d has been stable in production for a full release cycle.
- **3b** — Delete clearbox_vision's `v16_adapter.rs` and its associated
  hand-rolled reader files once Phase 2d is stable.
- **3c** — Update both repos' ownership-audit docs to reflect Build_Log as a
  real, live library rather than "transitional infrastructure."

### Phase 4 — Explicitly Deferred (not part of this plan, recorded for compatibility)
- **The Sensors library** (OPC UA today; possibly ClearBox-level sensors
  later) — separate effort, separate plan.
- **A compatibility adapter, from LBL's own unified API to what
  clearbox-tauri produces today, lives inside the future LBL library** —
  not inside Build_Log, and not inside clearbox-tauri's own application code
  (Guiding Principle 12). Build_Log's core targets only the new layout;
  bridging to the old `/Built_Part/Monitoring_Data` shape, if ever needed
  once LBL exists, is LBL's job to own in one place, rather than splitting
  that translation logic across every consumer that still needs it. Phase 1's
  side-by-side comparison below is a separate, temporary migration-validation
  step, not this adapter — it exists only to prove parity during the cutover,
  and goes away once Phase 3 completes.
- **The LBL dispatcher.** Two independent dispatch layers, discussed above
  under Versioning & Dispatch: LBL routes by group name only; each library
  (MCF, Build_Log, Sensors) keeps sole ownership of its own internal
  content-version dispatch. LBL's genuinely hard problem is not routing — a
  generic, schema-blind "copy this group's contents into that named subgroup"
  routine handles routing trivially — it's **coordinating writes into one
  shared physical file** across three independently-designed writers, which
  plain HDF5 does not support concurrently without either strict
  serialization or SWMR.
- **Preferred answer, and the one actually consistent with what's already
  decided: there is no new mechanism to invent here.** If/when LBL gets
  built, it should simply be — or wrap — the same single-owning-task,
  `Group`-attached model already decided for clearbox-tauri's own Build_Log
  integration (Guiding Principles 10–11, the Concurrency Model above),
  generalized from two libraries to three. Whatever owns the build's file
  lifecycle holds the one open `hdf5::File` and calls
  `create_machine_config_at`/`create_build_log_at`/`create_sensors_at`
  against three `Group`s carved out of it. MCF's one-time, build-start
  calibration write fits into that same owning task without difficulty,
  since — unlike Build_Log/Sensors — it isn't a continuous stream. This
  requires no merge step, no separate files, and is a strict extension of
  what this plan already commits to, not a competing design.
- **Fallback only, not the plan, and only if that assumption ever breaks:**
  the previous draft of this section presented a three-separate-files
  (`MCF.h5`, `BL.h5`, `Sensors.h5`) plus post-build copy-merge as *the*
  answer for LBL. That's the wrong emphasis — it's a fallback, relevant only
  if a future deployment genuinely can't put all three producers under one
  owning task or process (e.g. OPC UA telemetry collection running as a
  fully separate service, or MCF's snapshot written by some other separate
  tool) — a real possibility, but not something established as necessary
  yet. If that fallback is ever needed: each library produces its own
  standalone file (zero concurrent-write problem, since each library
  exclusively owns its own file for the whole build), then something
  performs a one-time, post-build, schema-blind copy-merge into a single
  combined file — always one physical file (Open Decisions §9: resolved),
  never HDF5 external links, since this is a manufacturing/quality record
  that should be one portable, self-contained artifact, not four files that
  must never be separated. That's simply a
  different *caller* choosing the `Path`-based convenience constructor
  (Guiding Principle 10) instead of the `_at` primitive — still not a
  conflicting design, just a different entry point on the same primitive,
  used only if the preferred single-owning-task answer above turns out not
  to apply. Confirmed compatible with Guiding Principle 6 (hash computed
  from data values only) either way — a true copy preserves those values
  byte-for-byte, so `is_valid()` still checks out post-merge, as long as no
  library's hash ever incorporates its own file position.

## Open Decisions Requiring Sign-Off

1. **~~Hash-mismatch severity~~ — RESOLVED.** Non-fatal `is_valid` flag,
   exactly as MCF does.
2. **~~`BuildLogMeta.schema_version` convention~~ — RESOLVED.** String,
   matching MCF's `"1.0"`/`"1.1"` convention — not the reference fixture's own
   plain integer (`File_Version: 18`), which was that file's own ad hoc
   versioning, not Build_Log's.
3. **~~New repo name/location~~ — RESOLVED.** Confirmed as `Build_Log_Library`,
   sibling to `Machine_Config_Library`; repo setup itself is being handled
   directly, outside this plan.
4. **~~Sync trait vs. async~~ — RESOLVED: sync.** Matches MCF and the
   underlying `hdf5` crate, which has no async API — an `async fn` here would
   just be a blocking call wearing an async signature, misleadingly implying
   yield/cooperative-scheduling behavior that doesn't exist. Also avoids real
   `dyn`-dispatch cost: `Box<dyn BuildLogReader>`/`Box<dyn BuildLogWriter>`
   (Guiding Principle 2) can't cleanly support native `async fn` without
   `async-trait`'s boxing macro or manual `Pin<Box<dyn Future>>` returns —
   overhead for no actual concurrency benefit, since there's no real async
   I/O underneath either way. Keeps the library's own test suite free of any
   async-runtime dependency, too. The owning task calls in exactly however
   it already calls blocking `hdf5` crate code today (inline, or via
   `spawn_blocking`) — worth confirming which, during Phase 1, given
   `append_layers` on a full layer (727,922 samples/laser) is real work, but
   that's an implementation detail to verify, not a reason to reconsider this.
5. **~~Where Build_Log's own version attribute lives~~ — RESOLVED.** Confirmed
   against the reference fixture: a `File_Version` attribute scoped to
   `/Build_Log`'s own group, never the bare file root. Guiding Principle 11
   generalizes this correctly for any mount point, not just `/Build_Log`.
6. **~~Does `BuildLogMeta` absorb legacy `BuildConfig`?~~ — RESOLVED, fully.**
   Distinct from Guiding Principle 12 (backward-compatibility with the old
   shape — a future LBL adapter's job, not Build_Log's) — this is about
   Build_Log's own *forward-looking* CDM scope. `BuildConfig`
   (`machine_config/types.rs:22-49`) has 13 fields; all 13 now have a
   decided home:
   - **8 → absorbed into Build_Log's `BuildLogMeta`**, matching confirmed
     `/Build_Log` root attributes in the reference fixture: `build_id`→
     `Build_ID`, `username`→`Username`, `layer_count`→`Layer_Quantity`,
     `num_of_parts`→`Number_Of_Parts`, `alloy`→`Alloys`, `material_id`→
     `Parameter_ID_For_Material`, `starting_height`→`Starting_Height`,
     `layer_thick`→`Layer_Thickness`.
   - **2 → MCF's territory, not Build_Log's:** `build_plate_thick`,
     `build_plate_weight` (`f64`, mm/kg) — machine/build-plate properties.
   - **3 → not Build_Log's concern:** `est_build_time`, `est_build_time_unit`
     (`f64`/`String`), `save_bin_data` (`bool`) — not carried into Build_Log's
     CDM. The old legacy group is superseded for the 8 absorbed fields;
     anyone needing its exact old shape uses the future LBL adapter
     (Principle 12), not something Build_Log itself replicates.
7. **~~Do Pre_Exposure/Post_Exposure images stay in Build_Log or move to
   Sensors?~~ — RESOLVED.** Images are owned by the future Sensors library,
   not Build_Log — this reverses the earlier draft's recommendation. See
   Ownership Boundary and Common Data Model for the technical facts carried
   forward for whoever builds Sensors.
8. **~~Who writes `Opcua_Join_Status`/`Opcua_Join_Version`?~~ — resolved for
   Build_Log's purposes.** This is a Sensors library concern, not Build_Log's
   — removed from Build_Log's CDM and Ownership Boundary entirely (see
   above). The full design (where these attributes live, who writes them)
   is deferred to the future Sensors library's own plan.
9. **~~Future LBL merge mechanics~~ — RESOLVED: it will be one file, always.**
   True copy-merge, not HDF5 external links, if the Phase 4 fallback (three
   independently-produced files) is ever actually needed — external links'
   four-files-that-must-never-separate shape is explicitly ruled out. Still
   Phase-4-scoped and not blocking anything today, since the preferred path
   (single owning task, generalized to three libraries) needs no merge step
   at all.
10. **Do `actual_layer_thickness`/`commanded_layer_thickness` belong under
    Build_Log's `Layer` at all?** Flagged, not decided — do not assume they
    stay in Build_Log's CDM as currently sketched. Not an MCF concern —
    layer thickness is a build-level setting, and Build_Log already has that
    at the build level via `BuildLogMeta.layer_thickness` (confirmed as
    `/Build_Log`'s own root `Layer_Thickness` attribute). Per-layer
    `commanded_layer_thickness` is likely redundant with that same
    build-level value, not distinct information — a candidate for dropping
    from `Layer` entirely. `actual_layer_thickness` is the genuinely open
    piece: whether/how it's actually populated (both were `0` in the
    reference fixture's own sample data) and, if it is meaningful, whether it
    belongs on `Layer` here or somewhere else. Left open deliberately — needs
    a real decision during Phase 0a before the CDM is finalized, not assumed
    either way in the meantime.

## Verification Checklist

- [ ] Phase 0 test suite green — including the Group-attached-mode test —
      with zero changes to either consumer app
- [ ] Phase 1 semantic parity confirmed against real historical build files,
      via the single owning writer task calling `create_build_log_at`
- [ ] Phase 2 parity confirmed against the same files via clearbox_vision's
      own rendering/analysis output
- [ ] Both apps' full existing test suites still pass after cutover
- [ ] Ownership docs updated in both repos
