# The Common Data Model

What the Model crate (`rust/model`, package `build-log-model`) holds, where each concept lives, and the rules for changing it. It replaces `derivation.md`, which mapped one V18 reference file to the pre-cleanup types; those types are gone (see **Changes** below).

**What the CDM is.** Plain Rust types for a build, its layers, lasers and channels, and the *requests* that reach their samples. It is **version-neutral**: the adapter of each file version fills it, and no type names a group, dataset or attribute of any file. It has **no HDF5** and builds for `wasm32`. It holds what a file *says*, as recorded: a value the file spells in its own way keeps that spelling, and where a value needs a note (a placeholder dropped, a channel not read) the note is an **origin** on a side table, not a change to the value.

**Bulk data is never in a metadata type.** Samples, event values, images and correction grids are reached through a column accessor (`ColumnSource` and the requests in `column`, `event_columns` and `image_regions`), so a build of 89 GB is never held whole.

## The modules

Every file of `rust/model/src` is a module of this table, and every public type is re-exported from the crate root (and from the `build-log` facade), so a consumer never names a module.

| Module | Holds | Main types |
|---|---|---|
| `identity` | The keys of layers, lasers and channels, and what "not there" means | `LayerId`, `LaserKey`, `ChannelKey`, `ByLayer`, `Presence`, `Availability` |
| `meta` | The build, its layers, its lasers, and each laser in each layer | `BuildIdentity`, `BuildMeta`, `LayerMeta`, `LaserMeta`, `LaserLayerMeta` |
| `context` | The build as a whole beyond its identity: the machine, the material, the starting height and the build plate | `BuildContext`, `BuildPlate` |
| `channel` | What a channel is: kind, clock class, data type, unit, captured width | `ChannelDescriptor`, `ChannelKind`, `ChannelDType`, `NumericType`, `ClockClass` |
| `time` | A time exactly as a file gave it, and the time axis with its accuracy | `TimeValue`, `TimeAxis`, `TimeWindow`, `Segment`, `Instant` |
| `units` | A number with its unit, the unit vocabulary and the as-written spelling | `Quantity`, `Unit`, `Dimension`, `UnitSource`, `parse_embedded` |
| `params` | The parameter snapshot: per-laser, per-optical-train values, equations as a kind plus named constants | `LaserParameters`, `OpticalTrainParameters`, `ScannerParameters`, `Equation`, `ParameterValue` |
| `quality` | The outcome of a build, with "none available" expressible | `Outcome`, `QualityRecord` |
| `provenance` | Where a value came from: the side table of origins and the build's provenance | `Origin`, `Origins`, `NotReadCause`, `Provenance`, `JoinStatus` |
| `events` | OPC UA event series, described (their records are columns) | `EventSeries`, `EventKind`, `EventDataType`, `EventPart` |
| `event_columns` | The times and values of one place of one series | `EventRequest`, `EventPlace`, `EventColumns`, `TypedValues` |
| `image_regions` | A rectangle of a camera frame and the pixels that come back | `ImageRequest`, `ImageRegion`, `ImageData` |
| `column` | Column access: requests, typed shared results, cancellation and progress, the traits a reader implements | `ColumnSource`, `BuildSource`, `ColumnRequest`, `ColumnData`, `CancelToken`, `InMemorySource` |
| `availability` | What a consumer can do with each layer, laser and channel | `AvailabilityReport`, `Item`, `AvailabilityCounts`, `availability_of` |
| `validation` | What is wrong or odd about a build, found without reading a sample | `ValidationReport`, `Finding`, `FindingCode`, `Severity`, `validate_build` |
| `extra` | Unknown attributes and groups, kept so they round-trip verbatim | `Extra`, `ExtraAttribute`, `AttrValue`, `UnknownNode` |
| `capabilities` | What a file version can do, and which versions are in play | `Capabilities`, `Feature`, `Direction`, `VersionReport`, `CDM_VERSION` |
| `error` | The error taxonomy: kinds, classes, locations and violations | `Error`, `ErrorKind`, `ErrorClass`, `Location`, `Violation` |
| `json` | Canonical metadata JSON and the JSON Schema export (reserved; nothing in it yet) | |

## The rules the guards keep

`rust/model/tests/policy_guards.rs` reads the source of every module and fails the build when one of these breaks.

1. Every enum is `#[non_exhaustive]`, or says `// closed: <reason>`.
2. Every public struct says who builds it (`// constructed-by: producer`, `library` or `caller-options`). A producer's struct is plain, so a new field is a compile error that makes the producer decide; a library's or a caller's is `#[non_exhaustive]` and has `Default` or `new`.
3. No metadata struct holds bulk data: no `Vec`, boxed or shared slice or reference to a slice of numbers, unless the type says `// bulk-container: <reason>`.
4. An enum for an open domain says `// open-domain: <reason>` and has an `Other(…)` variant that carries what it could not name.
5. No HDF5 path or group name appears in the Model.
6. Every source file is a module of the table above, and the pre-cleanup types do not come back (`old_vec_bearing_types_are_gone`).

## Changing the CDM (the evolution checklist)

Before a change to a type, answer these in the change itself.

1. **Optional or required?** A new optional field is safe. A new required field only if every existing adapter can supply a default.
2. **New concept or replacement?** For every added field, decide whether it is a new, orthogonal concept or a replacement of one the model already has. **If it is a replacement, the old field is removed in the same change**, so a consumer reading only the old one never silently gets empty data.
3. **Removing or renaming** a field is a breaking change: it needs a review of the consumers and an entry under **Changes** below (the CDM is 0.x, so a break is allowed, and each one is listed).
4. **Where does it come from?** Name the home of the field in each file version that carries it, or say why a version does not; a field no version supplies and no consumer asks for is not added.
5. **Does it have a guard?** A new module is added to the table and to the guards' list of modules; a new enum or struct carries the markers of the rules above.
6. **Is it checked independently?** The expectation files of the miniatures, and the conformance checks that read them, are extended first; then the adapter.
7. **Is it re-exported?** From the crate root, and so from the facade.

## Changes

| When | Change | Why |
|---|---|---|
| 2026-11-02 (IP-02 phase 2T) | **Removed the pre-cleanup types** that held whole `Vec`s and mirrored one V18 reference file: `Build`, `Laser`, `LaserMonitoringLayer`, `Sensor`, `SensorKind`, `SensorCalibration`, `OpcUaReading`, `EventRecord`, `ImagePair`, `GrayscaleImage`, `Corrections`, `CorrectionFile`, `InverseCorrectionGrid`, `CalibrationCurve`, `EquationConstant` and `LegacyByLayer`. **Breaking for anyone who named them.** Use `BuildMeta` with `BuildIdentity` and `BuildContext` for `Build`; `LaserMeta`, `LaserLayerMeta` and `LaserParameters` for `Laser`; `ColumnSource` and `ColumnRequest` for the sample vectors; `ChannelDescriptor` and `ChannelKind` for `Sensor`; `EventSeries` and `EventRequest` for the OPC UA records; `ImageRegion` and `ImageRequest` for the frames; `Equation` and `NamedConstant` for the calibration curve; `ByLayer` (keyed by `LayerId`) for `LegacyByLayer`. The correction grid and the correction document have no replacement yet: their descriptors are planned. | The new types are the CDM; the old ones were shaped like one file, held bulk data in metadata, and duplicated what the new types hold (the replacement rule above). The first adapter (V17) landed on 2026-10-23, and nothing but the Model's own guards and the facade's re-export test named them. |
