# fixtures/ — salidas de referencia del pipeline PYTHON

Generadas por `make_fixtures.py` con el código real de `camcuts_next/camcuts` (solo importado) en CUDA
(RTX 5060, PyTorch 2.11, Ultralytics 8.4.192), el 2026-10-30.

## Archivos

| Archivo | Qué es |
|---|---|
| `f1_cara.png` … `f6_sin_joya.png` | 6 cuadros 960×540 RGB de 8 bits de `D:\CONTENIDO NAPO\2026\Octubre\27\videos` (ffmpeg `scale=960:540:flags=area`, como el muestreador en CPU). C9168 t=58 s (persona con cara), C9168 t=94 s (dije + pulsera), C9177 t=64.25 s (seis joyas), C9179 t=32 s (joyas blandas), C9175 t=24.5 s (persona + cadenas), C9180 t=15.25 s (sin joya). |
| `voz_c9174.wav` | 10 s de C9174 (desde 24 s), mono 16 kHz PCM16, con voz. |
| `python_ref.json` | Por cuadro: `detections_x` / `detections_m` (YOLO-World x/m, conf ≥ 0.02, cajas `int`), `clip` (recorte +15 % y `p` del verificador, candidatos débiles y joyas fuertes), `detections_x_verified` (tras `verify_weak`), `pose` (caja, conf, 17 puntos), `silhouettes` (caja, puntuación de SAM y PNG), `measure` (salida de `jewel.measure`: por joya `lap_p99`, `lap_var`, `masked`, `mask_frac`, `edge`, `area_frac`, etiqueta, conf, caja). `speech`: probabilidades de Silero por ventana de 512 e intervalos de `get_speech_timestamps`. `speed_ms_torch_cuda`: tiempos de PyTorch. |
| `sil/*.png` | Siluetas de MobileSAM (0/255, tamaño del cuadro) de cada caja de grupo de joya (`_groups` con min_conf 0.15). |

`measure` usa `lap_scale = 1.0` (decodificación en CPU) y sin cuadro previo (movimiento 0).

## Tolerancias (prueba `crates/camcuts-infer/tests/python_fixtures.rs`)

Emparejamiento uno a uno por etiqueta e IoU (≥ 0.5, de mayor a menor).

| Modelo | Exigido | Medido (config. por defecto) | FP16 / FP32 |
|---|---|---|---|
| YOLO-World x | IoU ≥ 0.9 en parejas con conf ≥ 0.05; todas las cajas de conf ≥ 0.10 con pareja | FP32: IoU mín 0.997 (todas), recall 100 %, Δconf ≤ 0.0024 | FP16: IoU mín 0.88, Δconf 0.025, pierde 2 de 53 cajas de 0.02 → **FP32 por defecto** |
| YOLO-World m | igual | FP32: IoU mín 0.995, recall 100 %, Δconf ≤ 0.0017 | FP16: IoU 0.966, Δconf 0.008 |
| MobileSAM | IoU de silueta ≥ 0.9 | FP16: mín 0.999, media 0.9997 (12 siluetas) | FP32: mín 0.998 |
| Pose | caja IoU ≥ 0.9, puntos ≤ 4 px, Δconf de punto ≤ 0.05 | FP16: IoU 0.998, 2.7 px, 0.004 | FP32: 1.000, 0.8 px |
| CLIP | \|Δp\| ≤ 0.05 | FP16: 0.006, ninguna decisión p ≥ 0.6 cambia (19 recortes) | FP32: 0.002 |
| Silero VAD | \|Δp\| ≤ 0.05 | 0.00000 en 313 ventanas | — |

Para el núcleo: `lap_p99` por joya de `measure` es el número a reproducir con las mismas siluetas
(tolerancia sugerida: ±1 nivel, el percentil de un entero).

## Regenerar

```
set PYTHONDONTWRITEBYTECODE=1
set YOLO_AUTOINSTALL=false
python fixtures/make_fixtures.py --weights <carpeta con COPIAS de los .pt>
cargo test -p camcuts-infer --release -- --test-threads=1 --nocapture
```
