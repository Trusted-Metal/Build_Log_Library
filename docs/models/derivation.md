# Model Derivation

Maps HDF5 fixture structure to Rust types in `crates/model`. Updated
incrementally as each type is proposed and reviewed — this is a review aid,
not general reference documentation.

Reference fixture: `fixtures/reference_build_log_opcua_and_clearbox_sensors.h5`

## `Build` — `/Build` (group attributes)

All ten attributes are scalar. Dtypes below confirmed via direct h5py
inspection of the reference fixture (2026-09-24).

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Alloy` | str | `alloy` | `String` | Observed empty in this fixture instance |
| `Build_ID` | str | `build_id` | `String` | Version-like string (`"4.9.7.1"`), not numeric |
| `Date` | str | `date` | `String` | Format: `"2026-08-18 16:48:25 UTC+00:00"` |
| `Layer_Quantity` | int32 | `layer_quantity` | `i32` | |
| `Layer_Thickness` | float32 | `layer_thickness` | `f32` | |
| `Layer_Thickness_units` | str | `layer_thickness_units` | `String` | Added 2026-09-24 — was the only physical measurement in the file missing a `_units` companion. Value: `"mm"` (0.03 mm = 30 µm layer) |
| `Number_of_Parts` | int32 | `number_of_parts` | `i32` | |
| `Project_ID` | str | `project_id` | `String` | Observed empty in this fixture instance |
| `Source_MCF_Configuration_Hash` | str | `source_mcf_configuration_hash` | `String` | Hash of the source MCF config, not of this build log |
| `Source_MCF_Name` | str | `source_mcf_name` | `String` | Replaces the former `Source_MCF_Export_Date` (removed 2026-09-24) — a name is more directly useful for tracing which MCF config was used than an export timestamp. Value currently a placeholder: `"reference_config_v1_1.h5"`, not from a real capture. |
| `Username` | str | `username` | `String` | |

**Composition (added 2026-09-24):** `lasers`/`sensors` fields added,
`BTreeMap<String, _>` keyed by HDF5 group name — not a bare `Vec`, since
neither `Laser` nor `Sensor` stores its own name (deliberately excluded on
each), so identity has to live at this level or it's lost entirely. Neither
count assumed fixed.

**Resolved:**
- `Source_MCF_Export_Date` removed from the fixture, replaced with
  `Source_MCF_Name` — also resolves the "two different date formats in one
  group" question as a side effect: `Date` is now the only date-shaped
  field left in `/Build`, so there's nothing left to reconcile.
- `Source_MCF_Configuration_Hash` kept — not redundant with `Source_MCF_Name`
  even though they're adjacent: the name answers "what is this config
  called," the hash answers "are these exactly the bytes I think they are,"
  independent of naming. Complementary provenance, not duplicated.

## `Laser` — `/Build/Lasers/Laser_N` (group attributes)

Both `Laser_1` and `Laser_2` are present in the fixture and share this same
14-attribute set. Dtypes confirmed via direct h5py inspection of the
reference fixture (2026-09-24).

Identity (which laser this is) comes from the HDF5 group name, not from an
attribute inside it — deliberately not modeled as a `Laser` field; that's a
mapping/identity concern for whatever reads the group. (Identity is
preserved anyway, one level up — see `Build`'s `lasers` field above.)

**Composition:** `corrections: Corrections` and `monitoring_data:
ByLayer<LaserMonitoringLayer>` fields both added — see the
`Monitoring_Data` subsection below for the latter's shape.

Laser count is not fixed at two — this fixture happens to have exactly
`Laser_1`/`Laser_2`, but builds can have more. Whatever aggregate eventually
holds a build's lasers must not assume a fixed count (rules out something
like a `[Laser; 2]` array).

All fourteen attributes are tracked independently per laser group — nothing
in the schema links Laser_1's and Laser_2's values together. Where the notes
below say a value is "equal" or "differs" between them, that describes this
one fixture's data, not a structural rule.

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Actual_Bit_Resolution` | int32 | `actual_bit_resolution` | `i32` | Both `20` in this fixture |
| `Commanded_Bit_Resolution` | int32 | `commanded_bit_resolution` | `i32` | |
| `Nominal_Working_Distance` | float64 | `nominal_working_distance` | `f64` | Equal in this fixture (`670.0` for both) |
| `Nominal_Working_Distance_units` | str | `nominal_working_distance_units` | `String` | `"mm"` |
| `Rayleigh_Length` | float64 | `rayleigh_length` | `f64` | Equal in this fixture (`3.236` for both) |
| `Rayleigh_Length_units` | str | `rayleigh_length_units` | `String` | `"mm"` |
| `Scan_Head_Offset_X` | float64 | `scan_head_offset_x` | `f64` | Differs in this fixture (Laser_1 `-87.5`, Laser_2 `86.074`) |
| `Scan_Head_Offset_X_units` | str | `scan_head_offset_x_units` | `String` | `"mm"` |
| `Scan_Head_Offset_Y` | float64 | `scan_head_offset_y` | `f64` | Differs in this fixture (Laser_1 `23.5`, Laser_2 `-21.695`) |
| `Scan_Head_Offset_Y_units` | str | `scan_head_offset_y_units` | `String` | `"mm"` |
| `Scan_Head_Rotation` | float64 | `scan_head_rotation` | `f64` | Differs in this fixture (Laser_1 `0.0`, Laser_2 `180.0`) |
| `Scan_Head_Rotation_units` | str | `scan_head_rotation_units` | `String` | `"degrees"` |
| `Wavelength` | float64 | `wavelength` | `f64` | Equal in this fixture (`1070.0` for both) |
| `Wavelength_units` | str | `wavelength_units` | `String` | `"nm"` |

### `Monitoring_Data` — `/Build/Lasers/Laser_N/Monitoring_Data/Layer_M` (per-layer group)

Confirmed via direct h5py inspection (2026-09-24). Modeled as
`ByLayer<LaserMonitoringLayer>` — same sparsity wrapper as `Sensor`, though
no sparsity was actually observed here (both `Laser_1` and `Laser_2` have
both `Layer_1` and `Layer_2`); used anyway for consistency, not because
this fixture demonstrates a gap.

| Attribute/dataset | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Start_Laser_Time` (attr) | int64 | `start_laser_time` | `i64` | Plausibly microsecond-epoch — three orders of magnitude smaller than OPC-UA sensors' nanosecond-epoch `Source_Ts`, but that's an inference from magnitude, not confirmed |
| `End_Laser_Time` (attr) | int64 | `end_laser_time` | `i64` | |
| `Actual_X`/`Y`/`Z` | int32[] | `actual_x`/`y`/`z` | `Vec<i32>` | Parallel arrays — one entry per sample index |
| `Commanded_X`/`Y`/`Z` | int32[] | `commanded_x`/`y`/`z` | `Vec<i32>` | |
| `Power` | float32[] | `power` | `Vec<f32>` | |
| `State` | uint8[] | `state` | `Vec<u8>` | |

Sample counts: 727,922 (Layer_1), 719,160 (Layer_2) — match the fixture's
full-rate ClearBox-native sensor data exactly, consistent with sampling at
the same rate as those sensors.

**Fixture data note, not a modeling concern:** Laser_1 and Laser_2's
`Monitoring_Data` is byte-identical in this fixture (verified: full-array
equality on every dataset, not just matching samples). Confirmed this is a
function of how the fixture was constructed, not a real relationship
between the two lasers — not modeled or relied upon.

## `Corrections` — `/Build/Lasers/Laser_N/Corrections` (group + 3 children)

The group itself has no attributes. Confirmed via direct h5py inspection of
the reference fixture (2026-09-24), both lasers.

### `Correction_File` (dataset: `uint8` bytes + attrs)

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| (dataset payload) | uint8[] | `bytes` | `Vec<u8>` | 1,138,799 bytes (Laser_1) / 1,142,763 bytes (Laser_2) |
| `document_created_at` | str | `document_created_at` | `String` | |
| `document_id` | str | `document_id` | `String` | UUID |
| `document_name` | str | `document_name` | `String` | e.g. `"Story6.17.3.1_Laser_1_VMM.fc3"` |
| `document_type` | str | `document_type` | `String` | |
| `file_size` | int32 | `file_size` | `i32` | Redundant with `bytes.len()` — kept, not removed (unlike `Inverse_Correction_Data`'s cleanup) |
| `original_uri` | str | `original_uri` | `String` | Path into some external document-management system |
| `valid_as_of_date` | str | `valid_as_of_date` | `String` | |

### `Inverse_Correction_Data` (dataset only, zero attrs after cleanup)

Real shape `(257, 257, 2)`, dtype `float64`, both lasers. Modeled as flat
`Vec<f64>` + `(usize, usize, usize)` shape, not `ndarray::Array3` — that
choice depends on how the not-yet-built IO layer reads HDF5 datasets, which
isn't decided; flat `Vec` adds no dependency and doesn't foreclose either
option.

### `Power_Calibration` (group attrs + compound dataset child) → modeled as `CalibrationCurve`

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Algorithm_Equation` | str | `algorithm_equation` | `String` | Laser_1: `"W = a*V + b"`, Laser_2: `"W = c0 + c1*V + c2*V^2"` |
| `Algorithm_Type` | str | `algorithm_type` | `String` | Laser_1: `"LINEAR"`, Laser_2: `"POLYNOMIAL"` |
| `Input_units` | str | `input_units` | `String` | `"Volts"` |
| `Output_units` | str | `output_units` | `String` | `"Watts"` |

`Derivation_Equation_Constants` (compound dataset, `(name: S64, value: f64)`):
- Laser_1 (LINEAR): 2 rows — `b=50.5`, `a=107.5` (note: `b` before `a`, not
  alphabetical or equation order)
- Laser_2 (POLYNOMIAL): 3 rows — `c0=10.0`, `c1=5.0`, `c2=0.5`

Row order is confirmed not meaningful — modeled as `Vec<EquationConstant>`,
consumers must match on `name`.

**Resolved:**
- `file_size` stays as a real field — redundancy with `bytes.len()` accepted,
  not removed.
- `Algorithm_Type` went through two decisions, not one: first became a real
  enum (`Linear`/`Polynomial`/`Other(String)`) on the reasoning that
  `Power_Calibration`'s values are a deliberately curated, accepted set
  (`Polynomial` preferred). Then reverted back to `String` — that
  enforcement is a business rule, not a fact about the fixture's shape, and
  belongs at a future validation/adapter layer, not baked into this raw
  mirror type. Reverting also unblocked a real extraction: see `Sensor`'s
  `SensorCalibration` below — once `Power_Calibration`'s `Algorithm_Type`
  and `Output_units` matched ClearBox-native sensors' `Calibration_Data`
  exactly (the latter needed a fixture rename, `Derived_Output_units` →
  `Output_units`), the two became byte-for-byte identical on their shared
  core, so `PowerCalibration` as a distinct type was dropped entirely in
  favor of directly using the new shared `CalibrationCurve`.

## `Sensor` — `/Build/Sensors/*` (42 children, 4 structurally distinct kinds)

Full survey + representative deep-dives done 2026-09-24 (superseding the
pre-session summary's "34/4/2/1" claim — the real split, verified via attrs
+ children present, is **35 OPC-UA / 4 event markers / 1 image-capture / 2
ClearBox-native**; `Images` looked like it belonged with the other 4
"command/event" markers at the top level but has a completely different
internal shape, so it's its own category, not folded in).

Identity (which sensor) comes from the HDF5 group name, same as `Laser` —
not a `Sensor` field.

Modeled as `SensorKind` (an enum, not one struct with optional fields):
different kinds carry genuinely different per-layer *data shapes*, not just
different attributes, so mixing them into one struct would make invalid
combinations (e.g. image buffers alongside a float timeseries) constructible
when real data never does that.

### OPC-UA sensors (35)

Originally had `Scope` (constant `"Global"` across all 35 — zero
differentiating information) and `Signal_Type` (`"Analog"`: 23, `"Digital"`:
12) group attrs. **Both removed from the fixture 2026-09-24.** `Scope` was
pure ceremony. `Signal_Type` did vary, but its distinguishing information is
redundant with `Value`'s own content (`Digital` sensors' values are always
`"True"`/`"False"`-shaped, `Analog` sensors' are always numeric-string-shaped)
— recoverable by inspecting the string itself, so not lost by removing the
label. Caveat: this redundancy only holds because Digital values are spelled
`"True"`/`"False"` here — a hypothetical `"1"`/`"0"` encoding would be
ambiguous with a numeric reading. True of everything in this fixture, not
guaranteed in general.

`Calibration_Data`: always an empty group + one `Output_units` attr, always
observed as an empty string across all 35. Kept as a real field
(`calibration_output_units`) since removal wasn't asked for, unlike
`Scope`/`Signal_Type`.

`Monitoring_Data/Layer_N` (sparse — 9 of 35 sensors missing `Layer_1`, 4
missing `Layer_2`): `Source_Ts` (`Vec<u64>` timestamps) + `Value` — dtype is
`object` (byte-strings) in HDF5, **not** a native numeric/bool type, modeled
as `Vec<String>`. What a string actually means (boolean-like vs numeric) is
no longer recorded anywhere now that `Signal_Type` is gone; parsing is left
to whatever reads this, not decided here.

**`SensorKind::OpcUa`:**

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Calibration_Data/Output_units` | str | `calibration_output_units` | `String` | Always empty string across all 35 sensors |
| `Monitoring_Data` (per layer) | group | `monitoring_data` | `ByLayer<OpcUaReading>` | Sparse — 9/35 missing `Layer_1`, 4/35 missing `Layer_2` |

**`OpcUaReading`** (one layer's worth):

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Source_Ts` | uint64[] | `source_ts` | `Vec<u64>` | One per sample; 11–33+ samples observed per layer in this fixture |
| `Value` | object (byte-strings) | `value` | `Vec<String>` | Not a native numeric/bool type in HDF5; parsing left to the reader |

### ClearBox-native sensors (2): `Oxygen_Sensor_ClearBox_Optical_Train_1`, `Photodiode_Reflection_Sensor_ClearBox_Optical_Train_2`

No `Scope`/`Signal_Type` ever present. `Calibration_Data` is rich — shares
`CalibrationCurve`'s exact core (`Algorithm_Equation`, `Algorithm_Type`,
`Input_units`, `Output_units` post-rename, `Derivation_Equation_Constants`),
plus `Input_Range_High/Low`, `Output_Range_High/Low`, `Sample_Period` (all
`f64`) that `Power_Calibration` doesn't have — modeled as `SensorCalibration
{ curve: CalibrationCurve, input_range, output_range, sample_period }`.
`Algorithm_Type` values observed here: `"Log-Linear"`, `"LINEAR"` —
confirmed genuinely open-ended (more types exist in the real system beyond
what this fixture shows), unlike `Power_Calibration`'s curated set — same
attribute name, different field, `String` is correct for both but for
different reasons.

`Monitoring_Data/Layer_N`: just a flat `Value` dataset, real `float64` (not
string-encoded), **no `Source_Ts`** — modeled as `ByLayer<Vec<f64>>`.
High-frequency full-rate data (727,922 / 719,160 samples per layer in this
fixture), presumably regularly spaced at `sample_period` so per-sample
timestamps aren't needed. Both sensors have both layers — no sparsity here.

**`SensorKind::ClearBoxNative`:**

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Calibration_Data` | group | `calibration` | `SensorCalibration` | See table below |
| `Monitoring_Data` (per layer) | float64[] | `monitoring_data` | `ByLayer<Vec<f64>>` | No companion timestamps, unlike OPC-UA. 727,922 / 719,160 samples per layer in this fixture |

**`SensorCalibration`:**

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Algorithm_Equation`/`Algorithm_Type`/`Input_units`/`Output_units`/`Derivation_Equation_Constants` | — | `curve` | `CalibrationCurve` | Shared core — same type as `Corrections`/`Power_Calibration`, see table above |
| `Input_Range_High`/`Input_Range_Low` | float64 | `input_range` | `(f64, f64)` | |
| `Output_Range_High`/`Output_Range_Low` | float64 | `output_range` | `(f64, f64)` | |
| `Sample_Period` | float64 | `sample_period` | `f64` | |

### Event markers (4): `AddLayerCommand`, `CameraSnapshotCommand_1`, `ExposeCommand_scan_manager_scanner_1`, `WaitCommand`

No `Calibration_Data` at all. `Monitoring_Data/Layer_N` is a single record,
not a timeseries — verified `shape=(1,)` on every dataset field in every
sample checked: `ZHeight` (group attr, `f64`), `Source_Ts`/`Start_ns`/
`Stop_ns` (`u64`), optional `Value` (opaque string — `WaitCommand`'s was
`"{}"`, looks like serialized JSON, not parsed). Modeled as `EventRecord`.
Sparse the same way as OPC-UA sensors (some only `Layer_1`, some only
`Layer_2`).

**`SensorKind::EventMarker`:**

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Monitoring_Data` (per layer) | group | `monitoring_data` | `ByLayer<EventRecord>` | Sparse — some markers only have `Layer_1`, some only `Layer_2` |

**`EventRecord`** (one layer's worth — a single record, not a series):

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `ZHeight` (group attr) | float64 | `z_height` | `f64` | |
| `Source_Ts` | uint64, shape `(1,)` | `source_ts` | `u64` | Single value, not a series — verified `shape=(1,)` on every marker checked |
| `Start_ns` | uint64, shape `(1,)` | `start_ns` | `u64` | |
| `Stop_ns` | uint64, shape `(1,)` | `stop_ns` | `u64` | |
| `Value` | object (byte-string), optional | `value` | `Option<String>` | Not present on every marker (absent on `AddLayerCommand`); opaque where present (`WaitCommand`'s was `"{}"`) |

### Image capture (1): `Images`

Structurally its own thing, not lumped with the event markers despite
matching them at the top level (`children=['Monitoring_Data'], attrs=[]`).
`Monitoring_Data/Layer_N/{Pre_Exposure,Post_Exposure}/raw`: real 2D grayscale
images, confirmed `(5068, 5064)` `uint8`, using HDF5's own Image
Specification attrs (`CLASS='IMAGE'`, `IMAGE_SUBCLASS='IMAGE_GRAYSCALE'`,
`IMAGE_VERSION='1.2'`) — not modeled (identical across every image seen, not
meaningful per-instance variation). Modeled as `ImagePair { pre_exposure,
post_exposure: GrayscaleImage }`.

**`SensorKind::ImageCapture`:**

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Monitoring_Data` (per layer) | group | `monitoring_data` | `ByLayer<ImagePair>` | Only 1 sensor (`Images`) uses this variant |

**`ImagePair`:**

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| `Pre_Exposure/raw` | uint8[5068,5064] | `pre_exposure` | `GrayscaleImage` | |
| `Post_Exposure/raw` | uint8[5068,5064] | `post_exposure` | `GrayscaleImage` | |

**`GrayscaleImage`:**

| Attribute | Fixture dtype | Rust field | Rust type | Notes |
|---|---|---|---|---|
| (dataset payload) | uint8[] | `data` | `Vec<u8>` | Flattened; real shape tracked separately via `height`/`width` |
| (dataset shape[0]) | — | `height` | `usize` | Confirmed `5068` in this fixture |
| (dataset shape[1]) | — | `width` | `usize` | Confirmed `5064` in this fixture |

**Resolved:**
- `Images`/`ImageCapture` stays under `Sensor`/`SensorKind` — decided, though
  explicitly "for now," not asserted as a permanent architectural fit. Keeps
  traversal uniform (everything under `/Build/Sensors` is one `Sensor` type,
  no special-casing `Images` out at the top level); revisit if that stops
  making sense once more is built on top of `SensorKind`.

- `ByLayer<T>` kept over a bare `BTreeMap<u32, T>` — the "gaps are
  meaningful, not bugs" note stays in one place rather than repeated at
  every per-layer field.
- `Output_units`/`Calibration_Data` kept on OPC-UA sensors for now, despite
  being content-free for all 35 (always empty group + always-blank attr) —
  same shape of argument that got `Scope`/`Signal_Type` removed, but not
  acted on this round. Revisit later if it comes up again.
