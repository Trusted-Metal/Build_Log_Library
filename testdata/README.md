# testdata: the committed miniatures

Eight small HDF5 files cut from real Build Log files, so that tests run on any machine and in CI without the large fixtures. Each keeps the **structure** of the file it was cut from (a structure-fingerprint check showed this for every file) and drops most of the data. The repository owner approved them by hash: `special_layers_v17.h5`, `v16.h5`, `v17_march.h5`, `v17_mock.h5`, `v17_oct15.h5`, `v18.h5` on 2026-10-19; `v17_triggered_2026_10_19.h5` on 2026-10-20; `v17_2026_08_18_images.h5` on 2026-10-21. They were copied here on 2026-10-21.

## Files

| File | What it is | Cut from | Layers kept | Size | SHA-256 |
|---|---|---|---|---|---|
| `special_layers_v17.h5` | the special layers of the V17 original (layers 1964-1967 and 2006-2008) | `V17_fixture.h5` | 1964-1967,2006-2008 of 1-2110 | 11.48 MB | `ac1b13d3ed6bbf25b0ffa08ef4f130d5b1ff69cee565fabf79e494154bacd553` |
| `v16.h5` | V16, from the 15-layer trimmed copy of the 200-layer build | `V16_trimmed_fixture.h5` | 1-2 of 1-15 | 5.34 MB | `7f72b76a80f7cc171c551d2e5c7c45463e92816dbc257ff64f43dffdd67fd8f0` |
| `v17_2026_08_18_images.h5` | V17 of 2026-08-18 from tauri, the fullest: camera images and OPC UA events in each layer | `V17_2026_08_18_with_images.h5` | 1-3 of 1-3 | 9.83 MB | `be510775b0dd7fcb004820033320e809d41a310313a5768fb7df6d24b1b387c2` |
| `v17_march.h5` | V17 of March 2026, from the 15-layer trimmed copy of the 2,110-layer real build | `V17_fixture_trimmed.h5` | 1-2 of 1-15 | 9.10 MB | `696c5ca46a6e451e8ea0cc3de29054eb3eeafdfbca409e33c4f22eaa16c9f466` |
| `v17_mock.h5` | V17 current-release shape, from the 2026-10-16 mock capture | `V17_fixture_generated.h5` | 1-2 of 1-10 | 8.76 MB | `eda925a0cf1143dd3883eef2fd12865dec5d228c4872bc4590eb548a3de30b22` |
| `v17_oct15.h5` | V17 of 2026-10-15, a real machine build with an empty Laser_2 | `XXXXXXXXXXXXXXXXXXXX_2026-10-15T18-13-06.h5` | 1 of 1 | 8.27 MB | `1b0dffad23bc04dc69570c16d503c03881c610326e53d76a811ed6d346125f00` |
| `v17_triggered_2026_10_19.h5` | V17 of 2026-10-19 with the OPC UA join output (Global_Sensors, Build_Status): one layer, both lasers recorded | `V17_triggered_2026_10_19.h5` | 1 of 1 | 8.63 MB | `02bcee2da612064acfefbf7dd9808e7976c59d3780453637059a805325483d01` |
| `v18.h5` | V18 (provisional), the cleaned reference with the version marker | `V18_fixture.h5` | 1-2 of 1-2 | 6.60 MB | `342f0d542159b1f4d626cef6de711b2f110eaf58c872b38afc22a2ec114f566c` |
| **Total** | | | | **68.02 MB** | |

Beside every `.h5` is a `.sidecar.json`: what it was cut from (name and hash), the parameters, the layers kept, the structure fingerprint of the source and of the miniature (equal), and the redactions by attribute, path and length. A sidecar holds no original value, and the names of the source files, here and in the sidecars, have the same redaction: a name that held a build id shows it as `X` characters.

## How they were made

- **Cut down:** the first two layers, at most 25,000 samples per dataset, images cropped to 128 by 128, calibration data copied unchanged. `special_layers_v17.h5` is different: it holds layers 1964 to 1967 and 2006 to 2008 of the 2,110-layer V17 build with their own ids, and it still declares 2110 layers (7 are present, on purpose). Those layers hold the three layer structures that build has, which the first layers do not.
- **Redacted before the file existed** (never edited afterwards): `Build_ID`, `Laser_Serial_Number`, `Machine_Serial`, `Scanner_ID`, `Scanner_Serial_Number`, `Username`, `document_id`, `document_name`, `original_uri`. A replaced value keeps its length and its HDF5 type; every character but a space is now `X` (`0` for a numeric serial number). 58 values in all; the sidecars list them by attribute, path and length, never by value.
- **Left as they were:** group and dataset names (they are the structure, and V18's sensor groups are named for OPC UA nodes), equipment, vendor and material names, dates and times, and all process data and calibration grids.

## The expectation files

`expectations/` holds, for each miniature, the facts the Common Data Model must show for it: the layers and lasers (and whether each is recorded, recorded and empty, or not recorded), the channels with their kind and declared unit, the times as the file wrote them, and for V18 the sensors. An independent script that reads only the miniature writes them; they are never edited by hand, and each names the SHA-256 of its miniature, so a changed miniature makes its file stale. They hold no sample value and no value that is not already in the miniature.

## The primitives file

`primitives.json` holds how each kind of value in the miniatures is stored: for every distinct encoding of an attribute (class, width, signedness, byte order, text form and padding, scalar or array) an instance with its value and how many the miniatures hold, for every distinct kind of dataset its layout, chunks, filters and first values, and the `File_Version` of each file. The same independent script writes it, it is never edited by hand, and it says nothing about what a value means.

## Rules

1. **Do not edit, regenerate or replace a file.** HDF5 does not write the same bytes twice, so a rebuilt miniature has another hash. A change needs the repository owner's approval and a new line in `SHA256SUMS`.
2. **Check them:** `sha256sum -c SHA256SUMS` inside this folder. `.gitattributes` marks `*.h5` as binary and `SHA256SUMS` as LF text, so a checkout on Windows does not change a byte. A Rust test in the test kit (`testdata_matches_checksums`) makes the same check in CI.
3. **The tools that made these files are not in this repository.** They are kept on the maintainer's machine; the sidecars and this table are the record of what each file is and where it came from.
4. **Size budget:** at most 12 MB a file and 75 MB in all; this set is 68.02 MB, and its largest file is 11.48 MB.
5. **V18 is provisional.** `v18.h5` follows the V18 fixture, which is still being finalized; if V18 changes, the miniature is cut again and approved again.
