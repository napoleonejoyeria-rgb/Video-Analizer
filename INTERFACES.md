# CAMCUTS en Rust: contrato entre crates

The reference implementation is the Python app in `C:\Users\Usuario\Tools\camcuts_next\camcuts\` (read it, never edit it). The Rust version must give the **same selects** as Python on the same footage, so port behaviour, not just shapes.

> **Cómo leer este archivo (2026-11-06).** Es un registro cronológico de decisiones y mediciones: cuando dos
> secciones no coinciden, manda la más reciente y, por encima de todo, el código. La descripción al día de cada
> parte está en `docs/` (índice en `README.md`); cada documento termina con "Discrepancias encontradas", que
> lista lo de aquí que ya no vale. Lo principal que cambió desde las primeras secciones:
> - El detector por defecto es el **propio**, no `world`, y el selector ya no dice "(beta)".
> - En segundo plano, el detector propio analiza hasta 2 clips a la vez y empieza por los archivos más grandes.
> - El modo ligero salta con menos de 4,5 GB de VRAM libre, y en él el YOLO-World mediano se sustituye por el propio.
> - Al reanalizar se usa `-skip_frame noref`, ya no `bidir`.
> - `prepare` compila los modelos con el tamaño del primer clip, no con un cuadro de 960×540.
> - `fixtures/` no trae cachés por clip.
> - **Pendiente de revisar:** `WORLD_GB_PER_WORKER` sigue en 2,6 GB, pero el 2026-11-05 se midieron unos 3,3 GB
>   por clip de YOLO-World x. Con ese valor podrían entrar 2 clips sin VRAM suficiente.

## Rules for everyone

- **Code comments in Spanish**, on every module, public item and every non-obvious block (the user asked for all code to be commented). Identifiers in English.
- **Toolchain:** `stable-x86_64-pc-windows-gnu`. There are no MSVC build tools on this PC. Don't add crates that need MSVC or a C++ toolchain beyond what the GNU toolchain ships.
- **Disk:** only ~20 GB free on C:. Share `target/` in this workspace. Don't download big extras, and clean your scratch files.
- **GPU:** the user edits video on this PC but wants speed, not harsh limits. Follow `GPU_TURNOS.md`: moderate (~60%) only while DaVinci is open; otherwise full use without long stretches pinned at 99%.
- **Never show a window on screen.** Test the GUI headless (egui_kittest / offscreen render to PNG).
- **Never write under `D:\`.** That is the footage drive; read-only. Never touch `C:\Users\Usuario\Tools\camcuts` (the installed app).
- **Compatibility:** read/write the same files as the Python app: `cache/*.json` (one per clip), `*.camcuts` project (format `camcuts-project`, VERSION 1), `colores.json`, `clip_types.json`, `clips_meta.json`, OTIO/CSV exports. A results folder made by Python must open in Rust and vice versa.

## Crate ownership (one writer per crate)

| Crate | Owner | Depends on |
|---|---|---|
| `crates/camcuts-core` | core agent | — |
| `crates/camcuts-infer` + `models/` + `runtime/` + `fixtures/` | models agent | `camcuts-core` (only the `Detector` trait and types below) |
| `crates/camcuts-gui` | GUI agent | `camcuts-core`, `camcuts-infer` |

## Types and trait that `camcuts-core` must export first

The core agent creates these in `camcuts-core/src/types.rs` in its first steps, with these exact names, so the other two can build on them. Changes to this API are announced by editing this file.

```rust
/// A frame as delivered by the sampler: BGR, 8 bits, row-major, width*height*3 bytes.
pub struct Frame { pub width: u32, pub height: u32, pub data: Vec<u8> }

/// A detection in frame pixels.
pub struct Detection { pub x1: f32, pub y1: f32, pub x2: f32, pub y2: f32, pub conf: f32, pub label: String }

/// Pose keypoints of one person (17 COCO points: x, y, conf).
pub struct Pose { pub keypoints: [[f32; 3]; 17], pub bbox: [f32; 4], pub conf: f32 }

/// Everything the models give the core for one frame.
pub trait Detector: Send {
    /// Jewel / person / face detections (YOLO-World with the same prompts as Python).
    fn detect(&mut self, frame: &Frame) -> anyhow::Result<Vec<Detection>>;
    /// Silhouette mask of a box (MobileSAM), same size as the frame, 0/255.
    fn silhouette(&mut self, frame: &Frame, bbox: [f32; 4]) -> anyhow::Result<Vec<u8>>;
    /// Pose keypoints (yolo11n-pose).
    fn pose(&mut self, frame: &Frame) -> anyhow::Result<Vec<Pose>>;
    /// CLIP verifier: probability that the crop is a jewel (weak detections 0.02–0.15, accepted at p ≥ 0.6).
    fn verify(&mut self, frame: &Frame, bbox: [f32; 4]) -> anyhow::Result<f32>;
    /// Speech probability per 32 ms window of 16 kHz mono audio (Silero VAD).
    fn speech(&mut self, audio_16k: &[f32]) -> anyhow::Result<Vec<f32>>;
}
```

### Edición Bodas (añadido)

- `Detector` has one more method **with a default** (existing implementations don't change):
  `fn moments(&mut self, frame: &Frame) -> anyhow::Result<Vec<(String, f32)>>` returns the probability of each wedding
  moment for the whole frame (CLIP zero-shot, `models/bodas/clip_moments.json`). Default: empty. `OnnxDetector` implements it
  only when the edition is Bodas.
- `camcuts_core::edicion` holds the process edition (`Joyas` by default, `Bodas` for `camcuts-bodas.exe`, or env
  `CAMCUTS_EDICION=bodas`). `scoring` / `analyse` / `export` / `cli` hand the work to `camcuts_core::bodas` with a one-line
  check, so the jewellery behaviour and its parity tests are unchanged.
- `scoring::Segment` has two new fields, `moment` and `moments` (empty in the jewellery edition).
- In the Bodas edition `OnnxDetector` loads `yolo_world_bodas_{x,m}.onnx` (12 wedding subjects, labels in
  `camcuts_core::bodas::SUJETOS`) from `models/` or `models/bodas/`.
- **Each edition keeps its own files** (`camcuts_core::edicion`, one place for all of it): default results folder
  `Edicion::carpeta_resultados` (`<material>\CAMCUTS` for jewellery, `<material>\CAMCUTS Bodas` for weddings),
  project `app` name (`nombre_app`, `de_app`), GUI export prefix (`prefijo_export`) and CLI selects name
  (`nombre_selects`; jewellery names unchanged). A cache is reused only if `edicion::cache_vigente` (its
  `edition` key matches; jewellery caches without the key stay valid for jewellery); otherwise it is re-analysed.
  The GUI opens a project of the other edition read-only (autosave never overwrites it; "Guardar como" makes a copy).

### Detector propio (añadido)

- `DetectorKind` (`World` = YOLO-World, the default; `Propio` = the trained YOLO11 in `camcuts-infer/src/trained.rs`)
  now lives in `camcuts_core::detector` (with `parse`, `as_str`, `from_env`, `take_arg`, `de_cache`).
  `camcuts_infer::DetectorKind` / `camcuts_infer::trained::DetectorKind` re-export it, so existing code is unchanged.
- `Detector` has one more method **with a default**: `fn kind(&self) -> DetectorKind` (default `World`).
  `OnnxDetector` returns `Propio` only when it really uses the trained model (jewellery edition).
- CLI: `--detector propio|world` (without it: env `CAMCUTS_DETECTOR`, else `world`); `Args::detector_kind()` resolves it
  (always `World` in Bodas). `camcuts.exe --analizar` and `examples/analizar.rs` put it in `InferConfig::detector`.
- Cache: jewellery analyses write `"detector": "world"|"propio"`. A cache is reused only if `detector::cache_vigente`
  (same detector; caches without the key count as `world`; Bodas ignores the key). `--rescore` uses every cache.
- Parallel clips: `hardware::parallel_clips` is the single decision (light mode = 1, `--background` = 1 unless
  `--workers`, and at most `WORLD_MAX_WORKERS` = 2 with YOLO-World: 4 clips caused NVIDIA TDR resets on the RTX 5060).
- GUI (jewellery only): "Detector: Clásico | Propio (beta)" in the "Análisis" part of the material menu, saved in the
  `.camcuts` project as `"detector"` (missing = `world`) and passed to the analysis as `--detector`.

### Velocidad del análisis (2026-11-03)

- `Detector` has one more method **with a default** (no-op): `fn prepare(&mut self, width: u32, height: u32) -> anyhow::Result<()>`.
  The CLI calls it once per analysis thread while the first clip's ffmpeg starts. `OnnxDetector` opens and compiles
  its GPU models (detector, pose, SAM, CLIP) at the same time, one thread each, on a black 960×540 frame. Results
  don't change.
- `camcuts-infer` runs every DirectML model with **fixed shapes**: `runtime::ShapedSession` keeps ONE fixed-shape
  session per model, for the analysis frame (`batch`/`height`/`width` for YOLO and pose, `batch` = 1 for CLIP,
  `n` = 1 for the SAM decoder). When the orientation changes, it closes that session and compiles the new one.
  Anything else (`--zoom` crops, batches) goes to a dynamic session. One session per shape put two vertical clips at
  7.6 of 8 GB of VRAM. Static shapes allow DirectML to compile the whole graph. YOLO-World x FP32 per frame went
  from 47.6 to 39.9 ms, pose from 8.4 to 4.8 ms. Outputs are identical: the selects match 100% on Octubre 27.
  `CAMCUTS_FORMA_DINAMICA=1` turns it off, for comparisons.
  Exception (2026-11-06): the shared own detector (`Compartido`) keeps up to 2 fixed sessions per model
  (`InferConfig::max_fijas` = `MAX_FIJAS_COMPARTIDO`, LRU). Two clips of different shape in parallel recompiled
  the single session on almost every call: a landscape and a vertical clip with `--workers 2` took 153 ms per
  detector call against 31 ms after, with 2 compilations per model and identical selects. YOLO-World keeps 1.
  `CAMCUTS_PROFILE` prints each fixed-session compilation. `CAMCUTS_WORLD_FP16=1` loads YOLO-World FP16, for
  tests only: it is 2.7× faster, but on Octubre 27 C9169 lost 53% of its approved time, so FP32 stays the default.
- Sampler without proxy: `fps` comes before `scale_cuda`, so only the sampled frames are scaled. The frames are the
  same. Decoding alone took 25% of the GPU before and takes 13% now (4K 4:2:2 10-bit Sony).
- CLI: clips are analysed largest file first, and results are returned in input order (before this, they came back in
  the order the threads finished).
- Parallel clips: `hardware::parallel_clips` is also capped by `hardware::vram_workers(free_gb, detector)`, unless
  `--workers` is given. Each YOLO-World x clip takes about 2.6 GB (`WORLD_GB_PER_WORKER`): about 1.5 GB of models and
  about 1.1 GB for its ffmpeg with NVDEC. The cap leaves 0.5 GB free. Before, with DaVinci holding 2–3 GB, two clips
  didn't fit. `WORLD_MAX_WORKERS` stays at 2: two clips already keep the GPU at 85–90% during busy stretches, NVDEC is
  saturated on still clips (about 280 fps of 4K 4:2:2 for the whole machine), and a third clip needs VRAM the card
  doesn't have.

### Respaldo automático del detector propio (2026-11-03)

- With `--detector propio`, after the normal pass, every clip (new or cached) for which
  `detector::necesita_respaldo` is true is analysed again with YOLO-World, and that analysis replaces the own one.
  The rule: own-detector cache, at least `RESPALDO_MIN_JOYA_S` (2 s) of jewel, and the uncalibrated "joyas" preset
  approves less than `RESPALDO_FRACCION` (0.25) of the jewel time. It always uses that preset, never the chosen one or
  the filters. On the 37 test clips (Octubre 27, Candongas, Anillo) only C9181 trips it: 0.09, while every other clip
  is at 0.5 or more.
- The fallback cache is a normal YOLO-World cache (`"detector": "world"`) plus `"respaldo": "propio"`
  (`detector::CLAVE_RESPALDO`). `detector::cache_vigente` accepts it for both detectors, so it is never redone.
- `cli::run_con_respaldo(args, factory, respaldo)` takes a second factory for the YOLO-World detectors. `cli::run` is
  that call with no fallback. `camcuts.exe --analizar` and `examples/analizar.rs` pass one; `camcuts-cli` has no
  models. `--sin-respaldo` turns it off.
- The analysis loop is `cli::analizar_lote`, run once per pass. In the fallback pass, the hardware line ends in
  "· respaldo con el detector clásico" and each clip line in "(respaldo con el clásico)".
- Measured with DaVinci open: C9181 went from 1.8 s to 22.8 s approved in 21 s, in light mode with YOLO-World m.
  YOLO-World x gives 61 s. Candongas and Anillo don't trip it.

### Avance por clip y orden de los clips (2026-11-03)

- When `CAMCUTS_PROGRESO` is set (`cli::VARIABLE_PROGRESO`; the window sets it on the `--analizar` child), the
  analysis prints `progreso<TAB>N<TAB>path` lines for each clip:
  - `0` at the start;
  - one line per whole percent while sampling, capped at 99;
  - `100` when the clip is done, or `error` if it fails or is RAW without a proxy.

  Without the variable, CLI output is unchanged.
- In the window, `App::avance_clips` (cleared when the analysis ends) feeds `App::avance_de`. The list shows `N%`
  where the score goes (0% = queued, `error` in coral), with a thin bar under the row. The status bar shows the
  clip and its percentage. A progress line doesn't reload the list; only `[i/n]` lines do.
- Largest-file-first only when 2 or more clips run in parallel. With one at a time ("Segundo plano", the window's
  default, or light mode), clips keep the list order. A 509-clip folder used to start with a 9-minute clip, and the
  window showed nothing for minutes.
- The window's "Reanalizar · detector" uses `detector::cache_vigente`, so a fallback cache isn't flagged.
- The map toggle (Ctrl+5): the minimap needs an interior taller than 90 px (it was 120, which the default 200 px
  timeline never reached). Turning the map on shows the timeline and makes it at least 200 px tall.

### Poca VRAM con DaVinci abierto (2026-11-05)

DaVinci holds about 4 GB of the RTX 5060's 8 GB. A YOLO-World x clip needs about 3.3 GB: 2.2 GB of models plus
1.1 GB for ffmpeg with NVDEC.

- **What went wrong.** With 3.2 GB free, 1.4 GB went to system RAM and the analysis ran about 10× slower: 109 s
  for a 35 s clip. With 3.8 GB free, DirectML could not compile the model at all
  (`DmlGraphFusionHelper ... 80070057`), so every clip came out as an error. To the user, the GPU lit up and
  nothing got analysed.
- `hardware::LOW_VRAM_GB` is now 4.5 (Python used 3.0). Below that, light mode: YOLO-World m, one clip. The same
  clip takes 8.5 s there.
- Fallback 1: if YOLO-World x fails to open or run on DirectML, `OnnxDetector::detect_batch` switches to m and
  calls `hardware::bajar_a_ligero()`. From then on `gpu_mode()` returns Ligero, so new detectors use m. It prints
  an `equipo: GPU OCUPADA …` line, which the window shows in red.
- Fallback 2: if a World clip with at least `LENTO_MIN_MUESTRAS` (80) samples runs slower than
  `LENTO_S_POR_MUESTRA` (0.4 s per sample), the CLI also switches to light mode. Normal is about 0.05 s; spilled to
  RAM it was 0.8 s. Each thread recreates its detector after its current clip. `CAMCUTS_LENTO_S` overrides the
  threshold, for tests.
- Decoding on the CPU to save VRAM was slower (17 s against 8.5 s), so it was not adopted.
- Verified on 2026-11-05 with DaVinci open, using the installed app and the window's options: 4 "marzo" clips,
  including C9243 (9 min of 4K), took 114 s. C9243 alone took 85 s, where before it was heading past 30 minutes.

### Light mode analyses with the own detector, not YOLO-World m (2026-11-05, later)

YOLO-World m confuses the brand's jewels. Compared on "Videos Yo Manos Detalle" (33 ring clips, S-Log3):

| Detector | Approved | Notes |
|---|---|---|
| m (light mode) | 1318 s | Almost everything labelled "pulsera"; C9269, C9275 and C9276 got 0 s |
| x | 2083 s | |
| Own detector | 2134 s | Labels "anillo"/"topo"/"arete"; C9262 got 48 s (x 25 s, m 3.8 s) |

- The rule: when YOLO-World is requested in the jewelry edition and the own model is installed
  (`trained::disponible`), the own detector analyses instead of m. That happens in two cases:
  - in light mode (`OnnxDetector::new`);
  - when x fails to open (`detect_batch`). This switch only happens while x has not yet returned a result in
    that detector (`world_dio_resultado`), so the whole clip comes from the own detector. If x fails after
    giving results (mid-clip: the dynamic `--zoom` session, VRAM), the rest of the clip uses m with no mark, and
    the next clip gets a new light-mode detector.
- `InferConfig::propio_si_no_cabe` turns the swap off. Its default is true. The fallback factory (the app and
  `examples/analizar.rs`) sets it to false, because that pass has to run YOLO-World, so it uses m in light mode.
- The cache gets `"detector":"propio","en_lugar_de":"world"`. `detector::cache_vigente` accepts that cache for
  World too, so it isn't redone once memory is free again.
  - `Detector::en_lugar_de()` is a trait method with default `None`. Wrappers must forward it: the profiling
    wrapper in `analyse.rs` does.
  - The CLI runs the own detector's fallback whenever an entry carries `en_lugar_de`, not only with
    `--detector propio`.
- The "equipo:" line replaces `hardware::TEXTO_LIGERO` with `cli::TEXTO_LIGERO_PROPIO`.
- Verified with the installed app:
  - Light mode with World requested: all 4 clips come from the own detector and carry the mark. C9181 went
    through the fallback with m (1.8 → 22.8 s).
  - A second run in normal mode reuses all 4 caches.
  - With the app copy that has no x model, the analysis continues with the own detector.

### Own detector by default, shared models, I/P-only re-analysis (2026-11-05, evening)

- **Default detector.**
  - `DetectorKind::default()` is now `Propio`, both for the CLI with no `--detector` and for the window.
  - Caches with no `detector` key are still `World` (`de_cache` uses `unwrap_or(World)`, not the default).
  - The window saves `detector_v: 2` next to `detector`. A state without it (Python, or before this change, when
    "world" was only the old default) loads as `Propio`, and its classic-detector clips show "Reanalizar ·
    detector".
  - The label "(beta)" is gone, and the selector now lists "Propio" before "Clásico".
  - The weddings edition is unaffected: `Args::detector_kind` always returns `World` there.
- **Clips in parallel with the own detector.**
  - `camcuts_infer::fabrica(cfg)` hands every thread the same `Compartido` detector (`Arc<Mutex<OnnxDetector>>`),
    so the models sit in VRAM once. YOLO-World still gets one detector per thread.
  - After `bajar_a_ligero` the factory creates a new one. `CAMCUTS_COMPARTIR=0` turns sharing off.
  - The factory keeps only a `Weak` (2026-11-06): when every thread drops it at the end of the pass, its models
    leave VRAM before the YOLO-World fallback pass measures the free memory.
  - `hardware::set_gpu_mode` can be called again (an `AtomicU8`, not a `OnceLock`): the fallback pass publishes
    its own mode. The CLI creates the first detector after publishing the mode and uses its `kind()` as the
    effective detector (light mode with World requested and no own model installed stays at m, 1 clip).
  - `OnnxDetector::prepare` returns early when its models are already open.
- **How many clips.**
  - `vram_workers` for the own detector: the first clip takes 1.8 GB (`OWN_GB_PER_WORKER`), each extra one 1.0 GB
    (`OWN_GB_EXTRA_WORKER`, its ffmpeg's NVDEC, measured at 0.96 GB). With 3.3–4 GB free (DaVinci open), 2 fit.
  - `parallel_clips` with the own detector: light mode no longer limits it, and background mode gives
    `BACKGROUND_WORKERS_PROPIO` = 2. YOLO-World is unchanged.
  - The light-mode case where the classic detector is requested counts as `Propio`, because that's what analyses.
  - `hardware::linea_con_clips` writes the final number into the "equipo:" line.
- **Measured on 6 "Manos" clips, normal priority, GPU free.**

  | Workers | Time | Decoder | GPU (mean / p95) |
  |---|---|---|---|
  | 1 | 36.8 s | 79% | 44% / 62% |
  | 2, unshared | 33.3 s | 90% | 49% / 75% |

  With the own detector the limit is NVDEC (~260–290 fps on 4K 4:2:2 10-bit), not the models. CPU decoding
  manages 82 fps and saturates the CPU, so it was not adopted.
  - Not measured cleanly yet: 2 shared workers with enough free VRAM. In the test, DaVinci had 5–5.8 GB in use and
    `--workers 2` was forced with 2.8 GB free; it went to RAM.
- **I/P-only re-analysis.**
  - `sampler::saltar_b` adds `-skip_frame noref` to GPU/CPU decoding when the clip is H.264 and no proxy is being
    written. Proxy commands never skip. (2026-11-06: it was `bidir`, which also drops B frames used as references;
    on a default libx264 clip, b-pyramid normal, 27 of its 29 frames came out corrupted. `noref` gives frames
    identical to a full decode, test `robustez::saltar_no_referencia_da_los_mismos_cuadros`, and on Sony saves the
    same: 20 s of C9262 on NVDEC, 1.36 s with either, 2.5 s without skipping.)
  - `analyse::prepare_clip` doesn't rewrite a proxy that `sampler::proxy_vigente` accepts. That function checks the
    x264 signature `crf=20.0` plus `deblock=1` in the first 512 KB, matching bytes rather than text: an earlier
    version that sliced a lossy string panicked and gave "fallo interno al abrir el clip". Old CRF 28 proxies are
    redone.
  - Sony GOP: I B B P … over 15 frames, so 1/3 of the frames are decoded. Decoding 20 s takes 0.85 s against 1.97 s.
  - Re-analysing the 6 clips: 19.6 s against 52.9 s for the first analysis, under the same DaVinci load. In
    another back-to-back run, with DaVinci at 100% GPU, it was 55.5 s against 81.7 s.
  - Approved: 257.8 s against 259.0 s, and the same stretches per clip.
  - `CAMCUTS_SALTAR_B=0` turns it off.

- **SAM embedding cache.**
  - `MobileSam` keeps the last 4 frames (`CACHE_CUADROS`). With a single slot, the parallel clips sharing
    `Compartido` evicted each other's embedding, and the encoder ran again for every box.
  - In C9280 it went from 37 s to 114 s with 2 clips. After the fix, shared 89.4 s, unshared 86.9 s, 1 clip 93.9 s.
- **Own model r6 installed (2026-11-05).**
  - Fine-tuned from r2 on 103 frames of "Videos Yo Manos Detalle", with loose rings and studs on an ear model.
  - Held-out loose rings went from 66% to 99.9% "anillo". Held-out studs went from 71% to 95% "topo".
  - Approved time doesn't drop: Manos 2127 → 2138 s, Octubre 27 725 → 828 s.
  - Known cost: a signet ring on a fist (C8726) comes out "topo".
  - Details: `training\runs\r6`, `training\scripts\eval_r6.py`, `training\e2e_runs\r6cmp`. r2 is backed up in
    `bench_perf\modelos_r2`.
- **Open machine issue.**
  - Since about 13:45 on 2026-11-05 the MobileSAM encoder takes 340 ms per call on DirectML (fp32: 706 ms),
    against 25–35 ms earlier in the day.
  - It is the same with the 10:31 exe and with r2, so it is not the code. The rest of the models are normal: YOLO
    5 ms, pose 6 ms.
  - It started while DaVinci was at full load. The DXGI budget is 7 GB, there is no throttling, and the driver
    didn't change.
  - Re-measure with `cargo run --release -p camcuts-infer --example bench` with DaVinci closed or after a reboot.

### Viewer: colour and preview compression (2026-11-05)

- `motor::graduador`, with "auto" selected:
  - It uses the camera profile from `detect_profile`, e.g. S-Log3 → Rec.709. Before, it only stretched the luma
    contrast, and an S-Log3 looked grey and washed out.
  - It falls back to "auto" (stretch if the clip is log) when the metadata says nothing.
  - Camera conversions use `decoding_for(clip, "ffmpeg")`, because the viewer decodes with ffmpeg. Before it used
    OpenCV's defaults, which assume limited range, and a full-range Sony file came out flatter.
- The analysis still uses "auto" exactly as before: the sharpness thresholds were calibrated with it.
- `color::sony_xml` now falls back to the NonRealTimeMeta XML the camera embeds in the MP4 when the `M01.XML`
  sidecar is missing.
  - It reads the last 256 KB of the file, then the first. On the A7 IV the XML sits about 700 bytes before the
    end.
  - C9254 had no sidecar.
- Proxies (`sampler::PROXY_ARGS`, also used by the window's `TrabajoProxies`) are now `veryfast` CRF 20, without
  `-tune fastdecode`.
  - The old `ultrafast`/`fastdecode` CRF 28 turned off deblocking and CABAC, so skin and jewel edges came out in
    blocks.
  - 10 s at 540×960 encodes in 0.41 s of CPU.
  - About 7 MB per minute, against 4.3 before.
  - Proxies that already exist are not rebuilt.
- Opt-in test `video::pruebas::cuadro_real_con_color_automatico`: set `CAMCUTS_VISOR_CLIP`,
  `CAMCUTS_VISOR_FUENTE` and `CAMCUTS_VISOR_PNG`.

### GPU engine: DirectML (default) or CUDA (tests) (2026-11-03)

- `camcuts-infer::runtime::Engine` is `Cuda` or `DirectMl`, one per process: ORT is a single DLL, chosen before it
  is opened. `CAMCUTS_MOTOR` = `directml` (default if unset), `cuda` (fails if CUDA doesn't load), or `auto` (CUDA if
  `runtime_cuda` loads and the GPU answers, else DirectML). CLI `--motor auto|cuda|directml` sets it; it is applied
  by `camcuts_infer::motor_listo`, which `--analizar` and `examples/analizar.rs` call. With `auto`, if CUDA fails
  its test inference, the same exe is relaunched with DirectML. `Device::DirectMl` now means "the GPU, with the
  process engine".
- `runtime_cuda` is one shared folder: `CAMCUTS_RUNTIME_CUDA`, next to the exe, the folder above the exe, or
  `<workspace>\dist\runtime_cuda`. It holds ORT GPU 1.24.4, CUDA 12.9 (cudart, cublas, cublasLt, cufft) and cuDNN 9.20
  without `cudnn_adv` (1.93 GB). CUDA options for tests: `CAMCUTS_TF32=1`, and `CAMCUTS_CUDNN_BUSQUEDA` =
  `exhaustive` | `heuristic` | `default`.
- Why DirectML stays the default (Octubre 27, `--no-proxy`, 2 clips in parallel):
  - DirectML: 164–166 s.
  - CUDA FP32: 308 s. cuDNN frontend logs 133 "No execution plans support the graph" errors.
  - CUDA TF32: 191 s, 99.63% agreement.
  - Per call with TF32: YOLO-World x 54 vs 63 ms, but SAM 38 vs 25 ms, CLIP 17 vs 6 ms, pose 24 vs 8 ms.
  - The first start on the RTX 5060 JIT-compiles ORT's kernels (no sm_120 binaries in the package): about 75 s,
    cached in `%APPDATA%\NVIDIA\ComputeCache` (+112 MB).
- Parity tests under CUDA: YOLO-World x FP32 gives Δconf 0.0023 (TF32: 0.0006) and IoU ≥ 0.9965. `verificador_clip`
  fails its internal check: `verify` (batch 1) and `verify_rects` (batch N) differ by 0.0043 against the 2e-3
  tolerance, because cuBLAS uses different kernels for each batch size in FP16. The tolerance is unchanged: CUDA is
  not used by default.

## Fixtures

The models agent writes `fixtures/` with reference outputs computed by the **Python** pipeline: a few frames as PNG plus JSON with detections, silhouettes, lap_p99 per jewel, and per-clip cache JSONs. Both Rust crates test against them. Tolerances are stated in `fixtures/README.md`.
