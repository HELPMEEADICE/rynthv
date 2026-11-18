# Flat resource format and native routing

## Observed package

The supplied `Hatsune Miku AI-installer-v200.sfpk` is a 40,774,878-byte ZIP starting with `50 4B 03 04`. Its root contains `config.json`, `info.nofs`, `profile.png`, and `background.png`. The four neural resources are under `models/`, and six phoneme tables are under `phonesets/`. The archive has local and central timestamps that differ. The reader validates the archive structure, member offsets, expanded sizes, and the CRC of each consumed member.

The package identifies the voice by name, vendor, language, and a decimal version string. This sample uses the labels `acoustic`, `vocoder`, `duration`, `f0`, `duration_style`, and `f0_style`. `base_model_opus` is an opaque acoustic identifier. Three other names look hexadecimal, and two of those names differ from the MD5 of their serialized contents. RynthV indexes model IDs verbatim and computes its own SHA-256 for caching. The sample has no `format_version` field. Voice revision `200` identifies the voice release.

SFPKS is supported as a ZIP containing SFPKs, including members in nested directories. The community guide documents this layout. The supplied directory contains one SFPK. Batch verification wraps it in a generated container and separately uses synthetic fixture members. [SV Flat Practical Guide](https://5105.oneapp.dev/)

## NOFS-JSON and embeddings

RynthV accepts a UTF-8 JSON manifest in `info.nofs` or `voice.nofs`. It recognizes the sample's field names and the documented `base_model`, `timing_model`, `f0_model`, `pitch`, and `timing` names. If both names for the same field are present, their values must agree. Unrecognized JSON fields are retained in inspection output. A declared `format_version` is reported verbatim. [NOFS-JSON field guide](https://5105.oneapp.dev/)

| Field | Encoding | Native consumer |
| --- | --- | --- |
| `styles[].data` | 32 little-endian Float32 values in 256 HEX characters | DDS acoustic conditioning |
| `f0_style` / `pitch` | 32 little-endian Float32 values in 256 HEX characters | DDS F0 style input, before the model's learned scalar projection |
| `duration_style` / `timing` | 128 little-endian Float32 values in 1024 HEX characters | DDS duration record conditioning |
| `styles[].extra` | One little-endian Float32 in eight HEX characters, or a JSON number | Parsed and validated for inspection |

The acoustic network consumes the bank and style as distinct condition blocks. The bank is the `(base)` row. The style starts at `(default)` or zero when the manifest omits it, then adds the named or `(auxiliary)` rows weighted per frame by RynthV's Vocal Mode controls. This mapping follows the native DDS input layout and the community's description of the layer roles. The shipped sample has a base and named modes. The neutral behavior across other banks needs comparison with Flat reference audio. Missing pitch and timing overrides use the selected models' serialized condition vectors. [Flat style-vector roles](https://5105.oneapp.dev/)

`phonesets/<name>.data` contains a length-prefixed name and three counted arrays of length-prefixed strings, in order: local symbols, classes, and unified symbols. Lengths and counts use little-endian u32. Package tables apply to matching language slots in each stage. The selected stage supplies its unified vocabulary and language index. The `.txt` records are informational. Unknown model symbols are reported when a score uses them.

## Model resolution

The supplied acoustic model has an inline DNNI v1 tree. Its vocoder, duration, and F0 models are inline DNNI v2 trees. All four roots identify the corresponding SV1 DDS native stages. The sample's vocoder bytes also occur in the supplied Kasane Teto SV1 NOFS, and its F0 bytes occur in the supplied Halo SV1 NOFS. The package style, timing, and pitch vectors differ from the serialized model vectors.

The registry indexes the voice's bundled sources first, then scans other packages and any configured model directory. An external ID with multiple candidate sources is usable when their bytes agree. A recognized DNNI root and native stage setup are required. Prepared trees are shared by decoded model SHA-256 while referenced by a loaded voice. The voice identity used by projects hashes vendor and name, with a separate decimal revision. The synthesis content identity includes the manifest, phone tables, and model digests, so edits invalidate cached renders.

The archive reader accepts stored and Deflate members. It bounds archive size, expanded member sizes, entry count, metadata, embeddings, artwork, and inline graph depth. The importer writes generated local names and never extracts archive paths into the filesystem.
