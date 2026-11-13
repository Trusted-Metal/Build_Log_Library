# The `.lbl` archive

How a Build Log of **File_Version 18 and later** is stored, read and written. Written 2026-11-13; the facts about libhdf5 and the userblock were checked that day on libhdf5 1.14.5 and 2.0 (Python `h5py` 3.16), and the library's own tests run on the legs listed in the README.

## What it is, and which version lives where

An `.lbl` file is a **ZIP archive whose members are stored, never compressed**. One member is the HDF5 file with the build (`build.h5`). The others are whatever travels with the build: the machine configuration file (`machine_config.h5`), logs, images. This library reads and writes the first and **treats the others as opaque bytes**: it never opens the machine configuration (boundary rule M1, see the README), it only keeps it, lists it and hands out its bytes.

| File_Version | Lives in | Extension | Adapter says |
|---|---|---|---|
| 17 and earlier | a bare HDF5 file, never in an archive | `.h5`, `.hdf5` | `ContainerKind::BareHdf5` (the default) |
| 18 and later | an `.lbl` archive, never as a bare file | `.lbl` | `ContainerKind::Archive` |

**The rule is data, not code.** Versions are matched exactly, with no ordering, so the dispatcher holds no "18 or higher": each version's `AdapterSet` declares the container it lives in (`AdapterSetBuilder::container`), and a test, `production_versions_live_in_the_container_their_number_says`, holds the production list to the table above. A file of the other kind is refused with `ErrorKind::WrongContainer`, which names both kinds. The extension is checked too: a bare HDF5 file named `.lbl`, and an archive named `.h5` or `.hdf5`, are `WrongContainer`. Any other extension (a download in progress, a temporary name) lets the content decide.

## Layout

```text
byte 0        mimetype                  "application/vnd.lbl+zip" (stored, no extra field, so a tool can sniff it at byte 38)
              local header of build.h5, padded so that its data starts at byte 4096
byte 4096     build.h5                  the HDF5 file, from its superblock to its end, as HDF5 wrote it
(4096-aligned) machine_config.h5        optional
(4096-aligned) logs/..., attachments/...  optional, any names
(4096-aligned) lbl-manifest.json        what the archive says about itself (below)
              central directory, ZIP64 end record, ZIP64 locator, end record
```

* **Every member's data is stored**, so byte `n` of a member is byte `data_offset + n` of the file, and reading a range reads nothing else. ZIP64 end records are always written, so a reader has one path for every size; a member of 4 GiB or more, and a header at 4 GiB or more, use the ZIP64 fields.
* **Alignment padding** is an extra field of the local header with the id `0xD935` (the one Android's `zipalign` uses): `u16 alignment`, then zeros. A reader computes the start of a member's data from its **local** header (`header_offset + 30 + name length + extra length`), never from the central directory.
* **The manifest** (`lbl-manifest.json`, last) has `lbl_container_version` (1), `primary`, `machine_config`, `schema_version` (a hint: the HDF5 file's own marker is the authority), `attachments`, `created_by` and `created_utc`. A reader ignores keys it does not know, and refuses a `lbl_container_version` above the one it reads (`UnsupportedInVersion`). Without a manifest the primary is the first `*.h5` member that is not `machine_config.h5`.
* The format of the headers is the same as the proof of concept's (`lbl-archive`), so an archive written by either is read by both.

## Why a stock HDF5 reads it

HDF5 looks for its superblock at byte 0 and then at 512, 1024, 2048, 4096, ... and treats the bytes before it as a **userblock** that it ignores. The archive puts the HDF5 member at byte 4096, so **`H5Fopen` on the `.lbl` itself reads the build**: no virtual file driver, no extraction, no copy, the same speed as a bare file, on every libhdf5 from 1.10.8 to 2.x. (A custom driver would have to be written for each of those: the class structure that a driver fills in differs between 1.10, 1.12, 1.14 and 2.x.) HDFView, `h5dump` and `h5py` open an `.lbl` the same way.

Two facts about the superblock that the sealing uses, both checked: HDF5 writes the **base address** as the userblock size (4096), and its **end-of-file address** as an absolute offset, so the HDF5 data ends exactly at the end-of-file address. That is how a seal knows where the HDF5 member ends even if bytes follow it.

## The life of an archive

1. **Working file.** `build_log_io::container::create_working_file(path)` creates an HDF5 file **at the final path** with a userblock of 4096 bytes (`FileCreateBuilder::userblock`) and returns the `hdf5::File`. The writer writes the build into it, growing it as the build runs. Nothing is copied at the end. It is created exclusively: an existing path is an error. Until it is sealed the file is an HDF5 file and not yet an archive; `open` says so with `ErrorKind::Unsealed`, which can be retried once the build ends.
2. **Post-build passes** write to the HDF5 data of the working file as they always did (`File::open_rw`).
3. **Seal.** `seal(path, &SealOptions)` (1) reads where the HDF5 data ends from its superblock and cuts off anything after it, (2) reads the HDF5 data once for its **CRC-32** (`SealOptions::with_progress` reports it; a terabyte takes minutes), (3) writes the headers of `mimetype` and `build.h5` into the userblock, which is exactly big enough, (4) appends the machine configuration, the other members and the manifest, and (5) writes the central directory and end records **last**. The file is an archive from the moment they are complete: **a seal that is interrupted leaves a working file**, never a damaged archive, and sealing it again discards what the first attempt appended.
4. **Read.** `open(path)` reads the central directory, the `mimetype` and the manifest, and then opens the HDF5 member as usual. It costs the same for a megabyte and for a terabyte. Members are read by range (`Archive::read_member_at`), and `Archive::verify` checks a member's CRC-32 when asked (it reads the whole member, so it is for ingest and archival, not for every query).
5. **Immutable.** A sealed archive is not modified. A new version is written next to it and published with an atomic rename.

### Changing the HDF5 member of a sealed archive

HDF5 writes new data at the end of its own data, which is where an archive keeps its other members, and it cuts the file back to its own end when it closes. So **opening a sealed archive with `File::open_rw` destroys it, even to change one attribute**: this was measured, and the file no longer opens as an archive. A pass that must write to a sealed archive does this:

```text
let detached = unseal(path, limit)?;     // the other members go to <name>.lbl.detached.zip, then the file is cut back to the HDF5 data
{ let file = File::open_rw(path)?; ... } // the pass
seal(path, &detached.to_options())?;     // CRC again, members back, directory last
detached.finish()?;                      // removes the side file
```

The side file is written, whole and atomically, **before** anything is cut, so an interruption loses nothing: `Detached::load(path)` reads it and the seal can be done again. `unseal` reads the other members into memory (`BudgetExceeded` over `limit`), checks each against its CRC-32, and refuses to run if a side file is already there.

## Modification while a source is open

HDF5 caches what it has read, and a source caches what it has parsed; a file changed in place under them gives results that mix two versions. For an archive:

| Situation | Windows | Other systems |
|---|---|---|
| A writer opens the file while a source has it open | refused: the archive is opened with a sharing mode that excludes writers | not prevented; detected (next row) |
| A source is opened while a writer has the file | `IoError` with `transient: true` (a sharing violation), so it can be retried | not prevented |
| The bytes under the open file change | cannot happen | the source answers `FileChanged` from then on |
| A new version is renamed over the file | allowed; the source keeps reading the file it opened. `Opened::check_file()` says `Superseded` | the same |
| The file is deleted | allowed; the source keeps reading. `check_file()` says `Removed` | the same |

The check before every call is one query of the open handle (`GuardedFile::is_modified`); `check_file()` also compares the path and is meant to be asked now and then. The CRC-32 is **not** checked on every read: it needs the whole member. For per-read integrity of the samples, write the datasets with HDF5's fletcher32 filter. **A bare HDF5 file (V17 and earlier) is not watched**: that is how it has always been.

## Where it is in the code

| What | Where |
|---|---|
| The archive format: read, seal, unseal, the file guard | [`rust/archive`](../rust/archive) (`build-log-archive`): no HDF5, builds for `wasm32` without its file part |
| Which kind of file a path is; `create_working_file`; the guarded source | [`rust/io/src/container`](../rust/io/src/container) |
| The container of a version | `AdapterSetBuilder::container`, read by `dispatch::open` |
| `ContainerKind`, `WrongContainer`, `Unsealed`, `FileChanged` | [`rust/model`](../rust/model), see [the Model's changes](models/cdm.md#changes) |
| For a consumer: `build_log::open`, `Opened::container`, `Opened::archive`, `Opened::check_file`, `build_log::archive` | [`rust/build-log`](../rust/build-log) |

## What is not here yet

* **The V18 adapter.** The archive layer and the rules are in place and tested with the mock versions; the adapter that maps V18's layout into the Model arrives with its own work, and registers itself with `.container(ContainerKind::Archive)` and the place of its marker (`Build`, `File_Version`). Until then `open` on a V18 archive is `Unversioned`: the dispatcher only looks for the markers that registered versions keep, and V18's is not registered yet.
* **The version writers.** A writer for a version in an archive calls `create_working_file` and then `seal`; the writer surface of the adapters is typed `Infallible` until the writers exist.
* **Reading the machine configuration.** Out of scope for this library by rule; the Machine Config Library reads the member in place (it needs a virtual file driver, because the member is not at a power-of-two offset HDF5 searches).
* **Compressed members.** A member that is compressed is listed and cannot be read by range; the HDF5 member must be stored, or the open is `UnsupportedInVersion`.
