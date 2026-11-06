# models/ — modelos ONNX de CAMCUTS (camcuts-infer)

Exportados con `export_models.py` (venv de la app, `onnx`/`onnxslim`/`onnxruntime-directml` en una carpeta aparte
vía `PYTHONPATH`, `YOLO_AUTOINSTALL=false`, sin instalar nada en el venv; los `.pt` se copian antes a una carpeta
temporal porque Ultralytics escribe el `.onnx` junto al `.pt`). Opset 17. Cada modelo tiene su versión `_fp16`
(pesos FP16, entradas/salidas en FP32), hecha con `onnxruntime.transformers.float16` sin `value_info`.

Precisión por defecto en Rust (`InferConfig::default()`): **YOLO-World FP32**, el resto **FP16**
(ver tolerancias medidas en `fixtures/README.md`).

Todas las imágenes de entrada son los cuadros BGR de 8 bits del muestreador (960 px el lado largo).

## yolo_world_x.onnx / yolo_world_m.onnx — detector (YOLOv8-World v2)

- Pesos: `yolov8x-worldv2.pt` (perfil normal) / `yolov8m-worldv2.pt` (perfil con < 3 GB de VRAM libre, `hardware.py`).
- `set_classes` con los prompts de `jewel.py`, en este orden (índice = clase; también en `*.classes.json`):
  `ring→anillo, necklace→collar, chain necklace→cadena, earring→arete, stud earring→topo, bracelet→pulsera,
  pendant→dije, jewelry→joya, person→persona, face→cara`.
- Entrada `images` f32 `[lote, 3, alto, ancho]` (dinámica, múltiplos de 32).
- **Letterbox** (`LetterBox(960, auto=True, stride=32, center=True)`): `r = min(960/h, 960/w)`;
  tamaño sin relleno `(round(w·r), round(h·r))` (round de Python); relleno = resto hasta múltiplo de 32,
  repartido `top = round(dh/2 − 0.1)`, `bottom = round(dh/2 + 0.1)` (igual en x); valor de relleno 114;
  redimensionado `cv2.INTER_LINEAR` solo si cambia el tamaño. 960×540 → 960×544 (2 px arriba y abajo).
- **Normalización**: BGR→RGB, `/255`, NCHW. Sin media ni desviación.
- Salida `output0` `[lote, 14, anclas]`: `cx, cy, w, h` (píxeles de la entrada) + 10 puntuaciones ya con sigmoide.
- **NMS** (`non_max_suppression`): mejor clase por ancla, se queda si `conf > 0.02` (estricto; `VERIFY_MIN`, con
  CLIP activo; sin CLIP Python usa 0.05); xywh→xyxy; como mucho 30000 candidatos por confianza; NMS por clase
  (cajas desplazadas `clase·7680`), IoU **> 0.7** suprime; máximo 300 detecciones; orden por confianza.
- **Reescalado** (`scale_boxes`): `gain = min(h1/h0, w1/w0)`, `gain_y = round(h0·gain)/h0`, `gain_x = round(w0·gain)/w0`,
  `pad_x = round((w1 − round(w0·gain))/2 − 0.1)` (ídem y); `(x − pad_x)/gain_x`; recorte a `[0, w0]×[0, h0]`.
- Python guarda las cajas como `int(x)` (truncado): `Detector::detect` devuelve ya los valores truncados.
- Después (lo hace el núcleo): `min_conf` 0.15 (preset joyas); detecciones de joya entre 0.02 y 0.15 que no
  solapan (> 0.5 de la menor) una fuerte → hasta 4, a CLIP; aceptadas si `p ≥ 0.6`.

## yolo11n_pose.onnx — pose COCO de 17 puntos

- Entrada `images` dinámica. Letterbox igual que arriba pero a **640** (960×540 → 640×360 + 12 px arriba y
  abajo = 640×384, `INTER_LINEAR`). BGR→RGB, `/255`.
- Salida `[lote, 56, anclas]`: `cx, cy, w, h, conf` + 17 × `(x, y, visibilidad)` (visibilidad ya con sigmoide).
- NMS: 1 clase, `conf > 0.4` (`PoseFinder`), IoU 0.7. Cajas con `scale_boxes`, puntos con `scale_coords`
  (mismo `gain`/`pad`, recorte de x e y al cuadro). Python solo corre la pose en cuadros con `persona ≥ 0.4`.

## mobile_sam_encoder.onnx + mobile_sam_decoder.onnx — silueta de la joya

- **Codificador** `image` f32 `[1, 3, 1024, 1024]` → `embeddings` `[1, 256, 64, 64]`.
  Letterbox `LetterBox(1024, auto=False, center=False)`: `r = min(1024/h, 1024/w)`, `INTER_LINEAR`, relleno 114 solo
  abajo/derecha (960×540 → 1024×576 + 448 abajo). BGR→RGB, `(x − [123.675, 116.28, 103.53]) / [58.395, 57.12, 57.375]`
  (sobre 0–255, el relleno también se normaliza).
- **Decodificador** (prompt encoder de caja + mask decoder, `multimask_output=False`):
  `image_embeddings [1,256,64,64]`, `boxes [N,4]` xyxy **multiplicadas por r** (el modelo suma 0.5 internamente)
  → `masks [N,1,256,256]` (logits) y `scores [N,1]`.
- **Posprocesado** (`Predictor.postprocess` + `ops.scale_masks`): se descartan las máscaras con `score ≤ 0.25`
  (`conf` por defecto). Zona útil de la máscara: `span = round(lado·r)·256/1024` por eje (960×540 → 144×256).
  Bilineal sin `align_corners` (coordenada `(j+0.5)·span/destino − 0.5`, bordes repetidos) de esa zona al tamaño
  del cuadro; píxel activo si el logit **> 0**.
- Ojo, comportamiento de Python: `JewelSegmenter.masks` reparte las máscaras por posición, así que si SAM
  descarta una, las siguientes se corren. `OnnxDetector::silhouettes` lo replica; `Detector::silhouette` (una caja)
  devuelve todo ceros en ese caso. El embedding del último cuadro se guarda: varias cajas del mismo cuadro solo
  pagan el decodificador.

## clip_visual.onnx + clip_text.json — verificador CLIP ViT-B/32

- `pixels` f32 `[lote, 3, 224, 224]` → `embeds [lote, 512]` (sin normalizar). Pesos de `clip.load("ViT-B/32")`
  (la app corre en CUDA con pesos FP16; aquí FP32 / FP16).
- **Recorte** (`verify_weak`): caja entera de la detección agrandada `int(ancho·0.15)` / `int(alto·0.15)` por lado,
  recortada al cuadro. `Detector::verify(frame, bbox)` recibe la caja de la DETECCIÓN y hace este relleno.
- **Preprocesado** (torchvision sobre PIL): BGR→RGB; `Resize(224, BICUBIC)` del lado corto (largo =
  `int(224·largo/corto)`), con el remuestreo de PIL (a = −0.5, soporte escalado al reducir, dos pasadas con
  redondeo a 8 bits); `CenterCrop(224)` con `round((lado − 224)/2)`; `/255`;
  media `(0.48145466, 0.4578275, 0.40821073)`, desviación `(0.26862954, 0.26130258, 0.27577711)`.
- **Probabilidad**: `logits = 100 · (e/|e|) · textos` con los 15 embeddings de `verifier.embeddings`
  (7 de joya + 8 de otra cosa, ya normalizados), softmax, suma de los 7 primeros.
- `clip_text.json` también trae `typer` (11 tipos de `JewelTyper`, media de frases renormalizada), aún no usado.

## silero_vad.onnx — voz

- Copia de `silero_vad/data/silero_vad.onnx` del paquete `silero-vad` 6.2.3 (la app usa el modelo JIT
  equivalente; diferencia medida 0.00000).
- Audio mono 16 kHz f32 (`int16/32768`). Ventanas de 512 muestras, la última rellenada con ceros.
- Entradas: `input [1, 576]` = 64 muestras de contexto (las últimas de la ventana anterior, ceros al empezar)
  + 512 nuevas; `state [2, 1, 128]` (ceros al empezar, luego `stateN`); `sr` = 16000 (i64 escalar).
- Salida `output [1,1]` = probabilidad de voz. Los intervalos (`get_speech_timestamps`: umbral 0.5, histéresis
  0.35, `min_speech_duration_ms=400`…) los calcula el núcleo. Corre en CPU con un hilo.

## Runtime

`runtime/onnxruntime.dll` y `runtime/DirectML.dll` vienen de la rueda `onnxruntime_directml-1.24.4` (extraída,
no instalada). El crate `ort` 2.0.0-rc.15 usa `load-dynamic` + `api-24`. `DirectML.dll` se precarga desde
`runtime/` para que no gane la versión antigua de System32. Compilar con la toolchain GNU necesita
`.cargo/config.toml` (sustituto de dlltool con rust-lld, ver `.cargo/dlltool-lld.cmd`).

## bodas/ — edición Bodas (`camcuts-bodas.exe`)

Exportados con `models/export_bodas.py` (venv de la app, en CPU, sin instalar nada: usa `torch.onnx.export` directamente,
porque el exportador de Ultralytics exige el paquete `onnx`). Comprobación frente a Python: `models/check_bodas.py` y
`cargo run -p camcuts-infer --example bodas_check -- fixtures\f1_cara.png ...` (mismas cajas y confianzas a 3 decimales,
momentos a ±0.001 en CPU).

- `yolo_world_bodas_x.onnx` / `yolo_world_bodas_m.onnx`: YOLOv8-World v2 con 12 sujetos de boda incrustados, en este orden
  (`*.classes.json`): `face→cara, person→persona, bride in a white wedding dress→novia, groom in a suit→novio,
  wedding dress→vestido, wedding rings→anillos, bridal bouquet of flowers→ramo, wedding cake→pastel,
  champagne glass→copas, microphone→micrófono, high heel shoes→zapatos, sparkler→bengalas`. Misma entrada, letterbox y
  NMS que `yolo_world_x`; salida `[lote, 16, anclas]`. Solo FP32 (sin simplificar ni versión FP16).
- `clip_moments.json`: 16 momentos de la boda (`preparativos … salida`), cada uno la media renormalizada de 3-6 frases en
  inglés codificadas con el CLIP ViT-B/32 de `clip_visual.onnx`; `logit_scale` 100. El cuadro entero se rellena con negro a
  cuadrado antes del preprocesado de CLIP (así no se corta nada).