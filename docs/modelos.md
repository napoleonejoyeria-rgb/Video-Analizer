# Modelos e inferencia (`camcuts-infer`)

Este documento explica qué modelos usa CAMCUTS, de dónde salen, cómo se consiguen en un clon nuevo y cómo los ejecuta `crates/camcuts-infer`.

No repite lo que ya está escrito en otros documentos. Léelos junto a este:

| Documento | Qué tiene |
|---|---|
| [`models/README.md`](../models/README.md) | La especificación exacta de cada modelo: entradas, letterbox, normalización, NMS y reescalado. |
| [`fixtures/README.md`](../fixtures/README.md) | Las referencias de Python y las tolerancias medidas, en FP16 y en FP32. |
| [`INTERFACES.md`](../INTERFACES.md) | El registro de decisiones y mediciones, con fecha. Aquí solo se resume. |
| [`GPU_TURNOS.md`](../GPU_TURNOS.md) | Las reglas para usar la GPU: candado, DaVinci abierto y estabilidad. |
| [`docs/entrenamiento.md`](entrenamiento.md) | Cómo se entrena, se evalúa y se instala el detector propio (`jewel_det`). |

---

## 1. Qué modelo se usa para qué

`camcuts-infer` implementa el trait `camcuts_core::Detector` con `OnnxDetector` (`src/lib.rs`). Los modelos se abren la primera vez que se usan. Por ejemplo, un análisis sin voz no carga Silero.

### Edición de joyas (`camcuts.exe`)

| Tarea | Archivo que se usa | Precisión | Cuándo |
|---|---|---|---|
| Joyas, personas y caras | `jewel_det_fp16.onnx` + `jewel_det.classes.json` | FP16 | **Por defecto.** Es el detector propio (YOLO11s, 9 clases). |
| Ídem con el detector clásico | `yolo_world_x.onnx` + `yolo_world_x.classes.json` | FP32 | Con `--detector world` o con `CAMCUTS_DETECTOR=world`. |
| Ídem en modo ligero | `jewel_det_fp16.onnx` (si está instalado) o `yolo_world_m.onnx` | FP16 / FP32 | Ver [§10](#10-modo-ligero-sustitución-por-el-propio-y-caída-de-x-a-m). |
| Respaldo automático | `yolo_world_x.onnx` (o `_m` en modo ligero) | FP32 | Clips en los que el propio casi nunca da la joya nítida. |
| Silueta de la joya | `mobile_sam_encoder_fp16.onnx` + `mobile_sam_decoder_fp16.onnx` | FP16 | Hasta `jewel::SAM_MAX` (3) cajas por cuadro. |
| Pose (17 puntos COCO) | `yolo11n_pose_fp16.onnx` | FP16 | Solo en cuadros con persona. |
| Verificador de detecciones débiles | `clip_visual_fp16.onnx` + `clip_text.json` | FP16 | Detecciones de joya entre 0.02 y 0.15; se aceptan con p ≥ 0.6. |
| Voz | `silero_vad.onnx` | FP32 | Siempre en la CPU, con un hilo. |

### Edición Bodas (`camcuts-bodas.exe`)

| Tarea | Archivo que se usa | Precisión | Notas |
|---|---|---|---|
| Sujetos (12 clases de boda) | `yolo_world_bodas_x.onnx` (`_m` en modo ligero) | Solo FP32 | Se buscan en `models\` y, si no están, en `models\bodas\` (`OnnxDetector::modelo_bodas`). |
| Momento de la boda | `clip_visual_fp16.onnx` + `clip_moments.json` | FP16 | CLIP zero-shot del cuadro entero (`src/moments.rs`). |
| Silueta, pose, voz | Los mismos que en joyas | FP16 / FP32 | |

**Bodas nunca usa el detector propio.** `Args::detector_kind` devuelve siempre `World`, `InferConfig::puede_usar_propio` es falso y `OnnxDetector::kind` devuelve `World`. El paquete de Bodas no lleva `jewel_det`.

**La palabra "Napoleone".** `camcuts-bodas.exe` no puede contenerla. `scripts\armar_dist.ps1` (`SinNapoleone`) busca la palabra en el binario y se detiene si la encuentra. En `camcuts-infer` solo aparece en un comentario de documentación de `trained.rs`, que no llega al binario. No la pongas en cadenas, mensajes ni nombres de archivo.

### Qué lleva cada paquete

`scripts\armar_dist.ps1` copia solo lo que la app usa:

- **`dist\CAMCUTS\` (joyas):** `jewel_det_fp16`, `yolo_world_x` y `yolo_world_m` en FP32, SAM, pose y CLIP en FP16, Silero, los JSON y `runtime\`.
- **`dist\CAMCUTS Bodas\`:** `yolo_world_bodas_x`/`_m`, `clip_moments.json`, SAM, pose y CLIP en FP16, Silero y `runtime\`.

Las pruebas y el bench necesitan además las versiones que no van en los paquetes. Ver [§4](#4-conseguir-y-verificar-los-modelos-en-un-clon-nuevo).

### Módulos del crate

| Archivo | Qué hace |
|---|---|
| `src/lib.rs` | `InferConfig`, `OnnxDetector`, `Compartido`, `fabrica`, `motor_listo`. Decide qué modelo se abre y gestiona las caídas. |
| `src/runtime.rs` | Carga `onnxruntime.dll`, elige el motor (DirectML o CUDA), crea sesiones y define `ShapedSession`. |
| `src/trained.rs` | El detector propio: clases, umbrales, reescalado de confianza y supresión entre clases. |
| `src/yolo.rs` | YOLO-World, el detector propio y la pose: letterbox, NMS de Ultralytics y reescalado. |
| `src/sam.rs` | MobileSAM: codificador, decodificador de caja y caché de embeddings. |
| `src/clip.rs` | Verificador CLIP: recorte, preprocesado de PIL y probabilidad de joya. |
| `src/moments.rs` | Bodas: momento del cuadro entero con CLIP. |
| `src/vad.rs` | Silero VAD por ventanas de 512 muestras. |
| `src/imgops.rs` | Redimensionados idénticos a OpenCV y PIL, letterbox y `round` de Python. |

---

## 2. FP16 frente a FP32

### Cómo se hacen las versiones FP16

- **Modelos de Ultralytics, SAM y CLIP:** `models/export_models.py` (`to_fp16`) convierte el `.onnx` FP32 con `onnxruntime.transformers.float16.convert_float_to_float16(keep_io_types=True)`. Antes borra `value_info`: con esas formas intermedias, ONNX Runtime rechazaba CLIP. Las entradas y salidas siguen en f32, así Rust no necesita el tipo f16.
- **Detector propio:** `training/scripts/export_onnx.py --fp16` exporta el modelo ya en FP16, en CUDA. `F32Wrapper` hace la conversión de entrada y salida dentro del grafo.
- **YOLO-World de Bodas:** solo hay FP32.

### Qué precisión usa cada modelo por defecto

`InferConfig::default()` fija la precisión de cada modelo:

| Campo | Por defecto | Modelo |
|---|---|---|
| `fp16_world` | `false` (FP32) | YOLO-World x/m. `CAMCUTS_WORLD_FP16=1` lo cambia, solo para pruebas. |
| `fp16_own` | `true` | Detector propio |
| `fp16_sam` | `true` | MobileSAM |
| `fp16_pose` | `true` | yolo11n-pose |
| `fp16_clip` | `true` | CLIP |

### Por qué YOLO-World va en FP32

- **Paridad con Python** (`fixtures/README.md`). En FP16 la confianza se movía hasta 0.025 y cruzaba los umbrales 0.02 y 0.15. La IoU mínima bajaba a 0.88 y se perdían 2 de 53 cajas de 0.02. En FP32 la confianza se mueve ≤ 0.0024 y la IoU es ≥ 0.997.
- **De punta a punta** (`INTERFACES.md`). FP16 es 2,7 veces más rápido, pero en Octubre 27 el clip C9169 perdió el 53 % de su tiempo aprobado.

### Por qué los demás van en FP16

- **SAM, pose y CLIP:** pasan las tolerancias en FP16 sin cambiar ninguna decisión. Las cifras están en la tabla de `fixtures/README.md`.
- **Detector propio:** se comparó r2 en FP16 y en FP32 con `camcuts-e2e bench`, sobre los 301 cuadros de evaluación. El resultado está en `training/bench/propio_fp16.json` y `propio_fp32.json`:
  - diferencia de confianza: mediana 0,0013 y p99 0,012;
  - 5 de 924 parejas difieren en más de 0.02;
  - IoU: mediana 0,998;
  - una caja cruza el umbral de 0.15.

  No se ha vuelto a medir con r6.

---

## 3. De dónde sale cada modelo

### `models/export_models.py`: los modelos de Ultralytics, SAM, CLIP y Silero

Usa el entorno de Python de la app de referencia, **sin instalar nada en él**. Los paquetes `onnx`, `onnxslim`, `onnxconverter_common` y `onnxruntime-directml` se extraen en una carpeta aparte y se pasan por `PYTHONPATH`.

```powershell
$env:PYTHONPATH = "<carpeta con onnx, onnxslim, onnxconverter_common, onnxruntime-directml>"
$env:YOLO_AUTOINSTALL = "false"
python models/export_models.py --weights <carpeta con COPIAS de los .pt> --out models [--only world_x,world_m,pose,sam,clip,vad]
```

- **Copias de los `.pt`, nunca los originales:** el exportador de Ultralytics escribe el `.onnx` junto al `.pt`.
- **Dónde están los `.pt` en esta máquina:** en `C:\Users\Usuario\Tools\camcuts_next\` (`yolov8x-worldv2.pt`, `yolov8m-worldv2.pt`, `yolo11n-pose.pt`, `mobile_sam.pt`). Esa carpeta es la app de referencia: se lee y nunca se modifica.
- **CLIP ViT-B/32:** sale de `clip.load("ViT-B/32")`. Hay una copia del `.pt` en `training\weights\clip\ViT-B-32.pt`.
- **Silero:** se copia de `silero_vad/data/silero_vad.onnx`, del paquete `silero-vad` 6.2.3.
- **Formato:** opset 17. Cada modelo sale en FP32 y en FP16, salvo Silero.
- **`--only fp16`:** solo rehace las copias FP16 a partir de los FP32 que ya existen.
- **Prompts de YOLO-World:** están incrustados con `set_classes`. Son los 10 de `jewel.py` y su orden fija el índice de clase. También están en `PROMPTS` (`src/lib.rs`) y en `yolo_world_*.classes.json`.
- **`clip_text.json`:** trae los 15 embeddings de texto del verificador (7 de joya y 8 de otra cosa). También trae `typer`, que aún no se usa.

### `models/export_bodas.py` y `models/check_bodas.py`: la edición Bodas

- **`export_bodas.py`:** exporta `yolo_world_bodas_x/m.onnx` con los 12 sujetos de boda y `clip_moments.json` con 16 momentos. Cada momento es la media renormalizada de 3 a 6 frases. Corre en la CPU y usa `torch.onnx.export` directamente, porque el exportador de Ultralytics exige el paquete `onnx`.
- **`check_bodas.py`:** es la referencia de Python. Imprime las detecciones (≥ 0.05) y los tres momentos más probables de cada imagen.
- **`examples/bodas_check.rs`:** hace lo mismo en Rust, en la CPU.

Para comparar los dos:

```powershell
python models/check_bodas.py <carpeta con yolov8x-worldv2.pt> <clip_root> fixtures\f1_cara.png
cargo run -p camcuts-infer --example bodas_check -- fixtures\f1_cara.png
```

Se validó que salen las mismas cajas y confianzas a 3 decimales, y los mismos momentos a ±0,001.

### El detector propio (`jewel_det`)

Sale del entrenamiento (`training/`), no de `export_models.py`:

1. `training/scripts/train.py` entrena el modelo.
2. `training/scripts/export_onnx.py` lo exporta y escribe `jewel_det.classes.json` con las clases y los umbrales.

Está todo en [`docs/entrenamiento.md`](entrenamiento.md). El instalado es **r6**: `jewel_det.classes.json` lo dice en `"source"`.

### Las bibliotecas de `runtime\`

`onnxruntime.dll` y `DirectML.dll` se sacaron de la rueda `onnxruntime_directml-1.24.4`, extraída sin instalarla. El crate `ort` (2.0.0-rc.15) usa `load-dynamic` y `api-24`: no enlaza nada al compilar.

### Si vuelves a exportar un modelo

Un modelo exportado de nuevo no tiene por qué dar el mismo SHA-256. Después de exportar:

1. Regenera el manifiesto ([§4](#4-conseguir-y-verificar-los-modelos-en-un-clon-nuevo)).
2. Pasa las pruebas de paridad ([§12](#12-pruebas-de-paridad-con-python)).
3. Anota el cambio en `INTERFACES.md`.

---

## 4. Conseguir y verificar los modelos en un clon nuevo

Los `.onnx` (1,73 GB en total) y las `.dll` de `runtime\` **no van en Git** (`.gitignore`). Los `.json` de clases y textos sí van.

`models/MANIFIESTO.sha256` fija el SHA-256 y el tamaño de cada `.onnx` y de cada `.dll`: 17 modelos y 2 bibliotecas.

### Pasos

1. **Clona el repositorio** y ajusta la compilación una vez. Con la toolchain GNU, el script reescribe la ruta del sustituto de dlltool en `.cargo\config.toml`:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\configurar_compilacion.ps1
   ```

2. **Copia los modelos y el runtime** con esta estructura:

   ```
   models\*.onnx              (joyas y comunes)
   models\bodas\*.onnx        (yolo_world_bodas_x.onnx, yolo_world_bodas_m.onnx)
   runtime\onnxruntime.dll
   runtime\DirectML.dll
   ```

   De dónde copiarlos:
   - **La copia completa:** en esta máquina, `C:\Users\Usuario\Tools\camcuts_rs\models\` y `...\runtime\`.
   - **La carpeta compartida de la empresa:** la menciona `scripts\verificar_modelos.ps1`, pero el repositorio no fija su ruta. TI debe definirla.
   - **No sirven los paquetes de `dist\`.** Solo traen lo que usa la app. Faltan FP32 de SAM, pose, CLIP y `jewel_det`, y FP16 de YOLO-World. El verificador diría `FALTA`, y las pruebas de paridad y el bench no correrían.

3. **Comprueba los archivos:**

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1
   ```

   Si todo está bien, escribe `19 correctos, 0 con problemas`. Si falta un archivo o un hash no coincide, sale con código 1 y muestra `FALTA`, `TAMANO MAL` o `HASH MAL`.

4. **Rehaz el manifiesto solo si cambias un modelo a propósito** (por ejemplo, al instalar un detector propio nuevo). Después sube el manifiesto con el cambio:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1 -Generar
   ```

   `-Generar` recorre todos los `*.onnx` de `models\` (también `models\bodas\`) y las `*.dll` de `runtime\`.

### Qué cubre y qué no cubre el manifiesto

- **No cubre los `.json`.** `jewel_det.classes.json`, `clip_text.json` y `clip_moments.json` van en Git, y su versión es la del commit.
- **El detector propio va en pareja:** `jewel_det*.onnx` y su `jewel_det.classes.json` deben subirse en el mismo commit que el manifiesto.

### Dónde busca la app los modelos

- **`runtime::default_models_dir` y `default_runtime_dir`:** buscan `models\` y `runtime\` junto al ejecutable. Si no están, usan las del workspace donde se compiló (`CARGO_MANIFEST_DIR`).
- **`runtime::find_dll`:** también acepta `ORT_DYLIB_PATH`, o `onnxruntime.dll` junto al ejecutable.
- **`DirectML.dll` se precarga** desde la carpeta de `onnxruntime.dll`. Windows trae en System32 una versión antigua que ONNX Runtime no acepta.

### Sin el detector propio

El detector por defecto es el propio (`DetectorKind::default()` = `Propio`). Sin `jewel_det.classes.json` o sin `jewel_det*.onnx`, `TrainedDetector::open` falla y el análisis de cada clip falla con él.

Si no tienes el modelo propio, analiza con `--detector world`. `trained::disponible` solo se consulta para la sustitución del modo ligero ([§10](#10-modo-ligero-sustitución-por-el-propio-y-caída-de-x-a-m)).

---

## 5. El motor: DirectML, CUDA o CPU

ONNX Runtime es una sola DLL y solo se puede cargar **una vez por proceso** (`runtime::environment`, con un `OnceLock`). Por eso el motor se elige antes de abrirla.

### Cómo se elige el motor

| `CAMCUTS_MOTOR` / `--motor` | Qué pasa |
|---|---|
| sin valor o `directml` | DirectML con `runtime\`. Es el valor por defecto. |
| `cuda` | CUDA con `runtime_cuda\`. Si no carga, es un error. |
| `auto` | CUDA si `runtime_cuda\` está y la GPU responde. Si no, DirectML. |

- **`motor_listo` (`src/lib.rs`)** lo llaman `camcuts.exe --analizar`, `camcuts-bodas.exe --analizar` y `examples/analizar.rs`:
  1. pasa `--motor` a la variable de entorno;
  2. abre ONNX Runtime;
  3. con CUDA, hace una inferencia de pose sobre un cuadro negro (`prueba_cuda`).

  Si CUDA se eligió en automático y falla, vuelve a lanzar el mismo ejecutable con DirectML (marca `CAMCUTS_RESPALDO_DIRECTML`) y devuelve su código de salida.
- **`runtime_cuda\`** no va en el repositorio. Pesa 1,93 GB: ORT GPU 1.24.4, CUDA 12.9 y cuDNN 9.20 sin `cudnn_adv`. Se busca así (`runtime::cuda_runtime_dir`):
  1. `CAMCUTS_RUNTIME_CUDA`;
  2. junto al ejecutable;
  3. en la carpeta de encima;
  4. en `dist\runtime_cuda` del workspace.
- **`Device::DirectMl` significa "la GPU, con el motor del proceso".** El nombre es histórico: con el motor CUDA también vale `DirectMl`.
- **`CAMCUTS_DISPOSITIVO=cpu`** pone todos los modelos en la CPU. Sirve para pruebas con la GPU ocupada.

### Por qué DirectML es el motor por defecto

Se midió en Octubre 27, con 2 clips en paralelo:

| Motor | Tiempo |
|---|---|
| DirectML | 164–166 s |
| CUDA FP32 | 308 s |
| CUDA TF32 | 191 s |

Con CUDA solo YOLO-World x va algo más rápido. SAM, CLIP y la pose, modelos de muchas operaciones pequeñas, van entre 1,5 y 3 veces más lentos. Los detalles están en `INTERFACES.md`, en la sección "GPU engine".

### Opciones de sesión (`runtime::session_dims`)

- **Siempre:** `GraphOptimizationLevel::Level3`.
- **DirectML:**
  - sin patrón de memoria;
  - ejecución secuencial;
  - 2 hilos de CPU sin espera activa, para no quitarle CPU a ffmpeg ni a las medidas de imagen.
- **CUDA:**
  - búsqueda de convolución `exhaustive` (`CAMCUTS_CUDNN_BUSQUEDA` = `heuristic` | `default`, para comparar);
  - TF32 solo con `CAMCUTS_TF32=1`;
  - arena `SameAsRequested`.

  Si el proveedor CUDA no se registra, es un error: nada de seguir en la CPU sin avisar.
- **Silero VAD** usa siempre la CPU con 1 hilo (`SileroVad::open`).

---

## 6. Sesiones de forma fija (`ShapedSession`)

DirectML rinde mucho mejor cuando conoce las formas al crear la sesión: compila el grafo entero de una vez. Por eso `runtime::ShapedSession` guarda una sesión con las dimensiones fijadas (`session_dims` con `with_dimension_override`) para la forma que se repite en cada muestra.

### Qué tiene forma fija

| Modelo | Forma fija | Dónde se decide |
|---|---|---|
| YOLO-World, detector propio, pose | `batch`, `height`, `width` | `Yolo::predict`: un solo cuadro con lado largo = `sampler::LONG_EDGE` (960) |
| CLIP | `batch` = 1 | `ClipVerifier::probabilities` / `embeddings`: un recorte por llamada |
| Decodificador de SAM | `n` = 1 | `MobileSam::masks`: una caja por llamada |
| Codificador de SAM | (modelo exportado con forma estática 1×3×1024×1024) | sesión normal (`runtime::session`) |

- **Sesión dinámica:** recibe todo lo demás, como los recortes de `--zoom` y los lotes.
- **En la CPU** siempre se usa la dinámica.
- **`CAMCUTS_FORMA_DINAMICA=1`** desactiva las formas fijas, para comparar.

**Resultado** (`INTERFACES.md`, 2026-11-03): YOLO-World x FP32 pasó de 47,6 a 39,9 ms por cuadro y la pose de 8,4 a 4,8 ms. Las salidas son idénticas.

### Cuántas sesiones fijas se guardan (LRU)

- **`ShapedSession::get`** usa `lru_tomar`. Si la forma ya tiene sesión, la marca como la más reciente. Si no, suelta las más viejas hasta que cabe una más y compila la nueva. Nunca hay más de `max_fijas` copias del modelo en la VRAM.
- **Por defecto, `max_fijas` = 1** (`InferConfig::max_fijas`). Un clip vertical tras uno apaisado cierra la sesión anterior y compila la nueva. Con una sesión por forma, dos clips verticales llevaron la VRAM a 7,6 de 8 GB.
- **En el detector compartido, `max_fijas` = 2** (`MAX_FIJAS_COMPARTIDO`). Lo pone `Fabrica::crear` y llega a cada modelo por `set_max_fijas`: detector propio, pose, decodificador de SAM y CLIP.
  - **Por qué:** dos clips de forma distinta en paralelo (apaisado y vertical, o 16:9 y DCI) recompilaban la sesión única en casi cada llamada.
  - **Cuánto se ganó:** con `--workers 2` se pasó de 153 ms a 31 ms por llamada al detector.
  - **YOLO-World sigue con 1:** tiene un detector por hilo.
- **`CAMCUTS_PROFILE`** imprime cada compilación de sesión fija. `runtime::compilaciones_fijas()` las cuenta.
- **Prueba:** `runtime::pruebas::dos_formas_alternas_no_se_recompilan`.

### Precompilación (`Detector::prepare`)

La CLI llama a `prepare(ancho, alto)` una vez por hilo mientras arranca el ffmpeg del primer clip.

- **Qué hace `OnnxDetector::prepare`:** abre el detector, la pose, SAM y CLIP a la vez, cada uno en su hilo. Pasa por cada uno un cuadro negro del tamaño de análisis del primer clip (`cli.rs`; 960×540 si no lo sabe). En Bodas, el hilo de CLIP calcula momentos.
- **Si ya están abiertos**, vuelve enseguida. Así ocurre con el detector compartido, que prepara el primer hilo.
- **Los detectores de prueba** (`de_prueba`) no recuerdan sus fallos y su cuadro negro no cuenta como resultado de YOLO-World x.
- **Si cayeron al mediano o al propio**, el detector real hereda ese cambio.

---

## 7. El detector compartido entre hilos

**`camcuts_infer::fabrica(cfg)`** devuelve la fábrica que usa cada hilo de análisis (`cli::DetectorFactory`). La llaman `crates/camcuts-gui/src/analisis.rs` (`camcuts.exe --analizar`) y `examples/analizar.rs`.

### Cómo decide `Fabrica::crear`

1. Mira si hay un compartido vivo para el mismo modo de GPU. El campo `hecho` guarda `(¿modo ligero?, Weak<Mutex<OnnxDetector>>)`. Si `upgrade()` funciona y el modo coincide, devuelve otro `Compartido` sobre el mismo detector.
2. Si no, crea un `OnnxDetector::new(cfg)`.
3. Si `CAMCUTS_COMPARTIR=0`, o si el detector que sale no es el propio (`kind() != Propio`), lo devuelve tal cual: **uno por hilo**. Así va YOLO-World, también el de boda, porque lo limita la GPU y turnarse costaría.
4. Si es el propio, sube `max_fijas` a 2, lo envuelve en `Arc<Mutex<…>>`, guarda solo un `Weak` y devuelve `Compartido`.

### Por qué se comparte

Con el detector propio, el límite es el decodificador NVDEC, no los modelos, así que turnarse casi no cuesta.

A cambio, cada clip en paralelo solo necesita la memoria de su ffmpeg (unos 0,96 GB) y no 1,7 GB. Con DaVinci abierto caben 2 clips.

### Por qué la fábrica guarda un `Weak` (2026-11-06)

Antes guardaba un `Arc`. Como la fábrica vive durante todo `cli::run_con_respaldo`, los modelos (unos 0,75 GB) seguían en la VRAM durante el pase de respaldo con YOLO-World. Eso falseaba la VRAM libre que ese pase mide para decidir su modo.

Con el `Weak`, cuando todos los hilos sueltan el detector al terminar el pase, sus modelos salen de la VRAM.

### `Compartido`

`Compartido` es un `Arc<Mutex<OnnxDetector>>`. Cada método del trait toma el candado un momento (`con`):

- reenvía todos los métodos: `detect`, `silhouette`, `pose`, `verify`, `speech`, `moments`, `kind`, `en_lugar_de` y `prepare`;
- si un hilo entra en pánico, recupera el candado envenenado (`PoisonError::into_inner`) para que los demás sigan con los modelos.

### Otros detalles

- **Después de `bajar_a_ligero`** cambia el modo, así que la fábrica crea un compartido nuevo.
- **La fábrica del respaldo automático no comparte:** la app y `examples/analizar.rs` crean un `OnnxDetector::new` por hilo con `detector: World` y `propio_si_no_cabe: false`.
- **Prueba:** `pruebas::fabrica_no_retiene_el_compartido`.

---

## 8. Caché de embeddings de SAM

El codificador de MobileSAM es la parte cara: unos 30 ms, y unos 120 ms con DaVinci ocupando la GPU. El decodificador es barato. Por eso `MobileSam` guarda los embeddings de los **últimos 4 cuadros** (`CACHE_CUADROS` en `src/sam.rs`).

- **Clave:** `(huella del cuadro, alto, ancho)`. La huella es FNV-1a de 64 bits (`fingerprint`).
- **Política LRU:** si hay acierto, el embedding pasa al final. Si la caché está llena, se suelta el más viejo.
- **Por qué 4 y no 1:** con un solo hueco, los clips en paralelo que comparten `Compartido` borraban el embedding del otro, y el codificador se repetía en cada caja. Medido el 2026-11-05: C9280 pasó de 37 s a 114 s con 2 clips. Tras el arreglo: compartido 89,4 s, sin compartir 86,9 s y un solo clip 93,9 s.
- **El reparto de máscaras imita a Python.** Si SAM descarta una máscara con puntuación ≤ 0.25, las siguientes se corren un puesto, igual que en `JewelSegmenter.masks`. Está explicado en `models/README.md`.

---

## 9. Fallos de SAM y CLIP: se recuerdan y se avisan una vez

SAM y CLIP son **opcionales**. Sin ellos el análisis sigue, aunque peor:

- **Sin SAM**, se mide sobre la caja.
- **Sin CLIP**, las detecciones débiles no se confirman.

### Qué hace `OnnxDetector` cuando uno no se puede abrir

`OnnxDetector::sam` y `OnnxDetector::clip` abren el modelo en su primer uso. Si falla, `recordar_fallo`:

1. Imprime un aviso **una sola vez**, con la primera línea del error recortada a 300 caracteres:
   - `aviso: SAM no disponible (…): se mide sobre la caja`
   - `aviso: CLIP no disponible (…): las detecciones débiles no se verifican`
2. Guarda el mensaje en `sam_fallo` o en `clip_fallo`.
3. Las llamadas siguientes **no vuelven a intentar abrirlo**: devuelven `SAM no disponible: …` o `CLIP no disponible: …` enseguida.

**Por qué:** antes se reintentaba en cada caja, hasta `SAM_MAX` veces por muestra. Con el detector compartido, eso bloqueaba a los demás hilos.

**Excepción:** los detectores de prueba de `prepare` (`de_prueba`) ni recuerdan ni avisan. El detector real lo vuelve a intentar en su primer uso.

Solo se recuerdan los fallos **al abrir**. Un error de inferencia en una llamada concreta no se recuerda y la siguiente llamada lo intenta otra vez.

### Qué hace el núcleo con esos errores

- **Silueta:** `jewel.rs` y `bodas.rs` miden esa caja entera y cuentan el fallo (`MeasureState::sam_fallos`). Al guardar, la caché lleva `"sam": false` (`analyse::CLAVE_SAM`) y se imprime un aviso por clip.
- **Verificación:** la detección débil no se acepta (`analyse.rs`).

**Prueba:** `pruebas::fallo_de_sam_se_recuerda`, en la CPU y con una carpeta de modelos vacía.

---

## 10. Modo ligero, sustitución por el propio y caída de x a m

### Modo ligero

La CLI decide el modo y lo publica con `hardware::set_gpu_mode`:

- **Modo ligero:** si quedan menos de `hardware::LOW_VRAM_GB` (4.5 GB) de VRAM libre, o con `--gpu ligero`. Normalmente pasa con DaVinci abierto.
- **Sin modo publicado** (pruebas, la ventana), manda `InferConfig`.

### Sustitución de YOLO-World m por el detector propio (`propio_si_no_cabe`)

YOLO-World m confunde las joyas de la marca. En los 33 clips de anillos de "Videos Yo Manos Detalle" casi todo salía "pulsera":

| Detector | Tiempo aprobado |
|---|---|
| m | 1318 s |
| x | 2083 s |
| propio | 2134 s |

Por eso, en la edición de joyas, si se pide YOLO-World y el grande no cabe, analiza el propio.

`InferConfig::puede_usar_propio()` exige las cuatro condiciones:

- se pidió `World`;
- `propio_si_no_cabe` es verdadero (lo es por defecto);
- la edición no es Bodas;
- el modelo propio está instalado (`trained::disponible`).

**En `OnnxDetector::new`,** con el modo ligero publicado:

- `cfg.world` pasa a `M`;
- si `puede_usar_propio()`, `cfg.detector` pasa a `Propio` y `en_lugar_de` a `Some(World)`.

**La caché del clip lleva la marca:** `"detector":"propio","en_lugar_de":"world"`. `detector::cache_vigente` la acepta también para YOLO-World, así que no se repite cuando vuelve a haber memoria. `Detector::en_lugar_de()` solo devuelve algo si de verdad analiza el propio.

**El respaldo automático pone `propio_si_no_cabe: false`.** Ese pase tiene que ser con YOLO-World, así que en modo ligero usa m.

### Caída de x a m a mitad del análisis (`OnnxDetector::detect_batch`)

YOLO-World x puede no caber aunque el modo sea normal: otra app llena la GPU después de decidir el modo. Entonces DirectML no compila el modelo (`80070057`, "El parámetro no es correcto").

**Cuándo salta:** si x falla al abrir o al inferir en la GPU (`cfg.world == X` y `device == DirectMl`).

**Qué hace:**

1. `sustituir_por_propio(puede_usar_propio(), world_dio_resultado)` decide si pasa al propio.
2. `hardware::bajar_a_ligero()` publica el modo ligero. Solo la primera vez del proceso imprime:
   - `equipo: GPU OCUPADA — el detector grande no cabe…; se sigue con el detector propio | el mediano (modo ligero)`; la ventana muestra esa línea en rojo;
   - `aviso: no se pudo usar YOLO-World x: …`.
3. Suelta x (`self.world = None`) y pone `cfg.world = M`.
4. Sigue de una de dos maneras:
   - **Al propio, con la marca `en_lugar_de`:** solo si se puede y x **aún no dio ningún resultado** en ese detector. Así el clip entero sale del propio.
   - **Si no, con m**, que es el mismo YOLO-World. Pasa cuando el fallo llega a mitad de clip, por ejemplo en la sesión dinámica de `--zoom` o por falta de VRAM. Ese clip termina con m sin marca. Para el clip siguiente, la CLI crea un detector nuevo en modo ligero.

**Otros casos:**

- **En la CPU** no hay caída: el error sube tal cual.
- **En Bodas** solo existe la caída a `yolo_world_bodas_m`.
- **Prueba:** `pruebas::al_propio_solo_si_x_no_dio_nada`.

La otra caída, por lentitud (`LENTO_S_POR_MUESTRA`, `LENTO_MIN_MUESTRAS`, `CAMCUTS_LENTO_S`), vive en el núcleo. Ver `INTERFACES.md`, sección "Poca VRAM con DaVinci abierto".

---

## 11. El detector propio en inferencia (`src/trained.rs`)

Resumen. El entrenamiento está en [`docs/entrenamiento.md`](entrenamiento.md).

### Formato

- Mismo formato de entrada y salida que YOLO-World. Se reutiliza `yolo::Yolo` tal cual: letterbox 960, NMS por clase con IoU 0.7 y reescalado.
- `jewel_det.classes.json` trae `names` (9 clases, ya en español), `imgsz`, `thresholds` por clase, `default_threshold` y, opcionalmente, `weak_threshold`.

### Confianza

- **Umbral de trabajo:** `u` = 0.4 en las 7 clases de joya del modelo instalado.
- **Joyas por encima de `u`:** se reescalan para que `u` caiga en 0.15, el `min_conf` de los presets: `c' = 0.15 + (c − u)·0.85/(1 − u)` (`core_conf`).
- **Personas y caras:** salen tal cual si superan `PEOPLE_CONF` (0.10).

### Supresión entre clases (`cross_class`)

El NMS de Ultralytics es por clase, así que una pieza puede salir dos veces con etiquetas distintas. Si dos cajas de joya de distinta clase se solapan con IoU > 0.6 (`CROSS_IOU`), se queda la más segura.

### Banda débil (opcional)

Con `weak_threshold` = `lo`, las joyas con `lo ≤ c < u` van a la banda débil del núcleo, `c'` ∈ [0.021, 0.149] (`WEAK_BAND`), y CLIP decide.

**El modelo instalado (r6) no trae `weak_threshold`**, así que no hay banda débil: lo que no llega a 0.4 se descarta. Se probó y no se activó; ver el historial en `docs/entrenamiento.md`.

### Detalles

- **Umbral pedido al NMS (`nms_conf`):** el menor de todos (umbrales, `PEOPLE_CONF` y `lo`), es decir, 0.10.
- **Cajas:** se truncan a enteros, como `int(x)` en Python.
- **Pruebas:** `trained::tests::bandas_de_confianza` y `take_arg_quita_la_opcion`.

---

## 12. Pruebas de paridad con Python

`tests/python_fixtures.rs` compara `OnnxDetector` con las salidas del pipeline de Python guardadas en `fixtures/python_ref.json`. Esas referencias se generaron con `fixtures/make_fixtures.py` usando el código real de `camcuts_next`.

### Qué mide cada prueba

| Prueba | Qué compara | Qué exige (solo con la precisión por defecto) |
|---|---|---|
| `detecciones_yolo_world` | YOLO-World x y m, 6 cuadros | IoU ≥ 0.9 en parejas con conf de Python ≥ 0.05; todas las cajas de Python ≥ 0.10 con pareja (misma etiqueta, IoU ≥ 0.5) |
| `siluetas_mobile_sam` | 12 siluetas | IoU de máscara ≥ 0.9 |
| `pose_yolo11n` | Personas y sus 17 puntos | IoU de caja ≥ 0.9, puntos a ≤ 4 px, diferencia de confianza de punto ≤ 0.05 |
| `verificador_clip` | 19 recortes de `verify_weak` | \|Δp\| ≤ 0.05. Además, en las dos precisiones: el recorte de `verify()` es el de Python, y `verify()` (lote 1) y `verify_rects()` (lote N) difieren < 2e-3 |
| `voz_silero` | 313 ventanas de `voz_c9174.wav` | \|Δp\| ≤ 0.05 y el mismo número de ventanas |

- **Cada prueba corre FP16 y FP32 e imprime las cifras de las dos.** Solo exige las tolerancias a la configuración por defecto de `InferConfig`. Si `CAMCUTS_WORLD_FP16=1` está puesto, la prueba de detecciones exige a FP16.
- **El detector va fijado a `World`** (`config()`), porque las referencias son de YOLO-World. **El detector propio no tiene prueba de paridad:** no existe en Python. Sus pruebas son las del entrenamiento.

Las tolerancias y las cifras medidas están en `fixtures/README.md`.

### Requisitos

- Todos los modelos, en FP16 y en FP32: x, m, SAM, pose y CLIP, más Silero y `runtime\`.
- La GPU (DirectML). Sigue `GPU_TURNOS.md`: mira `.gpu_lock` y, con DaVinci abierto, uso moderado.

### Cómo correrlas

```powershell
# pruebas de paridad (GPU), una a una y con las cifras
cargo test -p camcuts-infer --release --test python_fixtures -- --test-threads=1 --nocapture

# pruebas unitarias del crate (9, sin GPU)
cargo test -p camcuts-infer --lib
```

- **Pruebas unitarias:** necesitan `runtime\onnxruntime.dll`. `fabrica_no_retiene_el_compartido` termina en pánico sin él; `fallo_de_sam_se_recuerda` se salta sola.
- **Con el motor CUDA, `verificador_clip` falla:** `verify` y `verify_rects` difieren 0,0043, por encima de 2e-3. cuBLAS usa núcleos FP16 distintos según el tamaño del lote. La tolerancia no se cambió porque CUDA no es el motor por defecto (`INTERFACES.md`).
- **Para regenerar las referencias:** `fixtures/README.md`, sección "Regenerar".

---

## 13. El bench (`examples/bench.rs`)

```powershell
cargo run -p camcuts-infer --release --example bench          # GPU
cargo run -p camcuts-infer --release --example bench -- cpu   # CPU
```

Usa `fixtures/f2_dije_pulsera.png` (960×540). Cada medida es la media de 15 repeticiones, tras 2 de calentamiento que compilan los grafos de DirectML. Corre en tandas cortas para no acaparar la GPU.

### Qué mide

| Línea | Qué es |
|---|---|
| `CPU: preparar SAM / preparar YOLO / máscara de SAM al cuadro` | El trabajo de CPU: letterbox y tensor CHW antes de la llamada, y la máscara de 256×256 llevada al tamaño del cuadro después. |
| `YOLO-World X/M fp16=…: lote 1, lote 6` | ms por cuadro. El lote 1 usa la sesión fija; el lote 6 (`batch_world: 6`), la dinámica. |
| `pose fp16=…` | ms por cuadro. |
| `MobileSAM fp16=…: 1 caja, 3 cajas, solo decodificador` | Codificador y decodificador con un cuadro distinto en cada llamada (cambia un byte para saltarse la caché), y solo el decodificador con el embedding guardado. |
| `CLIP fp16=…` | ms por recorte en un lote de 4. |
| `Silero VAD (CPU)` | ms por ventana de 512 muestras. |

Cada modelo se mide en FP16 y en FP32. **No mide el detector propio.** Para eso:

- `CAMCUTS_PROFILE=1` en un análisis real;
- `camcuts-e2e bench`, de `training/e2e`, que no va en el repositorio.

### Última medida (2026-11-06)

En `bench_perf\regresion\bench.log`, en FP16:

| Modelo | Tiempo |
|---|---|
| YOLO-World X | 14,3 ms |
| YOLO-World M | 9,7 ms |
| Pose | 6,9 ms |
| SAM, 1 caja | 30,4 ms |
| SAM, solo decodificador | 4,7 ms |
| CLIP | 4,8 ms por recorte |
| Silero | 0,173 ms por ventana |

En esa misma ejecución, YOLO-World X FP32 (397 ms) y SAM FP32 (728 ms) salieron anómalos. Cuadra con el problema de la máquina anotado en `INTERFACES.md`: el codificador de SAM lento desde el 2026-11-05. Vuelve a medir con DaVinci cerrado o tras reiniciar antes de sacar conclusiones.

### Otros ejemplos

- **`examples/analizar.rs`:** el análisis completo por línea de comandos, sin la ventana. Usa la misma fábrica compartida y el mismo respaldo que la app:

  ```powershell
  cargo run -p camcuts-infer --release --example analizar -- <clips o carpetas> --out <dir> [opciones de la CLI]
  ```

- **`examples/bodas_check.rs`:** la comprobación de los modelos de Bodas en la CPU ([§3](#3-de-dónde-sale-cada-modelo)).

---

## 14. Variables de entorno de `camcuts-infer`

| Variable | Efecto |
|---|---|
| `CAMCUTS_DETECTOR` | `propio` o `world`. Sin ella, el propio. `--detector` manda sobre ella. |
| `CAMCUTS_MOTOR` | `directml` (por defecto), `cuda` o `auto`. |
| `CAMCUTS_RUNTIME_CUDA` | Carpeta de `runtime_cuda`. |
| `CAMCUTS_TF32` | `1`: TF32 en CUDA. |
| `CAMCUTS_CUDNN_BUSQUEDA` | `exhaustive` (por defecto), `heuristic` o `default`. |
| `CAMCUTS_DISPOSITIVO` | `cpu`: todos los modelos en la CPU. |
| `CAMCUTS_WORLD_FP16` | `1`: YOLO-World en FP16, solo para pruebas. |
| `CAMCUTS_FORMA_DINAMICA` | `1`: sin sesiones de forma fija. |
| `CAMCUTS_PROFILE` | Imprime cada compilación de sesión fija. |
| `CAMCUTS_COMPARTIR` | `0`: un detector propio por hilo, sin compartir. |
| `ORT_DYLIB_PATH` | Ruta explícita de `onnxruntime.dll`. |
| `CAMCUTS_RESPALDO_DIRECTML` | Interna: marca el proceso relanzado con DirectML. |

---

## Discrepancias encontradas

En todos los casos manda el código actual.

1. **`models/README.md` (MobileSAM)** dice que se guarda "el embedding del último cuadro". El código guarda los de los **últimos 4** (`sam::CACHE_CUADROS`).
2. **`models/README.md` (yolo_world_m)** dice que es el perfil "con < 3 GB de VRAM libre". La doc de `WorldSize::X` en `src/lib.rs` dice "≥ 3 GB libres". En Rust, el umbral del modo ligero es `hardware::LOW_VRAM_GB` = **4.5 GB**. Además, en joyas el modo ligero analiza con el propio, no con m, si está instalado.
3. **`INTERFACES.md` ("Velocidad del análisis")** dice que `prepare` compila los modelos "on a black 960×540 frame". El código usa el tamaño de análisis del primer clip (`cli.rs`, `sampler::scaled_size`); 960×540 es solo el valor si no lo sabe. En Bodas, el hilo de CLIP calcula momentos en vez de verificar.
4. **`INTERFACES.md` ("Fixtures")** dice que `fixtures/` trae JSON de caché por clip. No hay ninguno: solo PNG, el WAV, `python_ref.json` y `make_fixtures.py`.
5. **Comentario de `ShapedSession` en `src/runtime.rs`:** dice que en el detector compartido las 2 sesiones fijas son para "YOLO11s y la pose". `Fabrica::crear` sube `max_fijas` a 2 y eso llega también al decodificador de SAM y a CLIP.
6. **Cabecera de `tests/python_fixtures.rs`:** dice que la pose compara "puntos visibles (conf > 0.5)". La prueba compara **todos** los puntos, y el comentario del bucle lo dice.
7. **Comentario de `crates/camcuts-infer/Cargo.toml`:** dice que basta con añadir al PATH la carpeta de `dlltool.exe`. El método actual es el sustituto con rust-lld de `.cargo\config.toml`, que se ajusta con `scripts\configurar_compilacion.ps1`.
