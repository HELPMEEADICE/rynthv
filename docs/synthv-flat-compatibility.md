# Flat compatibility and verification

| Input or feature | Status |
| --- | --- |
| Supplied Hatsune Miku AI v200 SFPK | Native DDS load and WAV synthesis exercised |
| Plaintext packages with the supported four-stage DDS contract | Import, model resolution, native preparation, vocal-mode timbre, and WAV export |
| ZIP SFPKS batches | Inspected and imported with synthetic fixtures and a generated wrapper containing the supplied SFPK |
| Thin voice with locally available shared models | Resolved against the real sample's shared models using a derived metadata fixture. Pointer sharing was verified with synthetic topology fixtures |
| Independent `sing_model` | Parsed as a separate dependency. Its executable contract remains unverified |
| Encrypted Flat 1.3.x and other container layouts | Outside the implemented reader |
| Flat `styles[].extra` | Decoded and retained. Its DSP mapping remains unverified |
| Korean in the supplied package | Declared by the manifest and present in F0. The acoustic and duration models lack a Korean language slot |

The community changelog reports an encryption change and native Korean support in Flat 1.4.0. Those release notes do not identify the supplied archive's precise producer version. The compatibility checks use the detected container and stage structures. [Community Flat changelog](https://vocakey.wikidot.com/synthesizerv-flat)

## Verification

The workspace tests label generated ZIPs, manifests, and minimal DNNI markers as synthetic. They exercise metadata and embedding validation, bounded parsing, batch atomicity, dependency diagnostics, source conflicts, model sharing, removal, and the headless command contract. The markers have no trained weights.

Real-sample tests live in `tests/native_voice_samples.rs`. The Flat test uses the project planner and Japanese lyric frontend, checks that changing a real Vocal Mode changes audio while preserving the pitch prediction, and optionally writes a WAV. The SV1 test synthesizes the example Mandarin score. The SV2 test checks preparation of a caller-supplied DNNI v3 revision. Configure sample paths through the indicated environment variables.

On Windows x64, the Miku lyric and Vocal Mode test produced 96,000 finite mono samples at 48 kHz, with a peak of 0.525367. The explicit-phoneme CLI render produced 112,800 samples. Blending against the derived metadata fixture changed the rendered PCM at the same frame count. The supplied POPY AI v101 SV1 NOFS rendered the Mandarin score to 132,000 mono samples. A local SV2 DNNI v3 revision and matching Flat reference audio were unavailable for this run.

```powershell
$env:RVM_FLAT_SAMPLE = 'C:\oss\SV\Flat\Hatsune Miku AI-installer-v200.sfpk'
$env:RVM_DICTIONARIES = 'C:\oss\SV\clf-data'
$env:RVM_FLAT_WAV = 'C:\path\miku-lyrics.wav'
cargo test --release --test native_voice_samples real_flat_voice -- --ignored --nocapture

$env:RVM_SV1_SAMPLE = 'C:\oss\SV\SV1\POPY_AI_101\voice.nofs'
cargo test --release --test native_voice_samples real_sv1 -- --ignored --nocapture

$env:RVM_SV2_REVISION = 'C:\path\to\sv2-revision'
cargo test --release --test native_voice_samples real_sv2 -- --ignored --nocapture
```

Headless verification measures the WAV and checks finite, nonzero samples. Comparing F0, phoneme boundaries, or acoustic output with Flat requires reference audio and matching score settings. Live editor verification remains an on-device step.
