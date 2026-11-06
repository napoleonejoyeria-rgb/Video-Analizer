# Entrenamiento del detector propio (`jewel_det`)

Este documento explica:

- cómo se hizo el detector propio de CAMCUTS;
- cómo se entrena, se exporta y se evalúa;
- qué pasó en cada ronda;
- cómo se instala un modelo nuevo sin romper nada.

**Qué es el detector propio.** Es un **YOLO11s** de vocabulario cerrado, afinado con material de la empresa. Desde el 2026-11-05 es el detector por defecto de la edición de joyas. Sustituye a YOLO-World x, que sigue disponible con `--detector world` y como respaldo automático. La edición Bodas no lo usa.

**Qué se instala.** Tres archivos que van siempre juntos:

- `models/jewel_det.onnx` (FP32);
- `models/jewel_det_fp16.onnx` (el que usa la app);
- `models/jewel_det.classes.json`.

**Modelo instalado: r6.** El campo `"source"` del JSON apunta a `training\runs\r6\weights\best.pt`.

**Otros documentos que conviene leer:**

| Documento | Qué tiene |
|---|---|
| [`docs/modelos.md`](modelos.md) | Cómo usa la app el modelo en inferencia: umbrales, reescalado, supresión entre clases y modo ligero. |
| [`INTERFACES.md`](../INTERFACES.md) | Las decisiones y mediciones con fecha. Aquí se resumen. |
| [`GPU_TURNOS.md`](../GPU_TURNOS.md) | Las reglas de uso de la GPU. Todos los scripts que la usan intentan respetarlas. |

---

## 1. Clases

`training/scripts/labels.py` (`CLASSES`) define 9 clases. Sus nombres ya son las etiquetas en español que entiende el núcleo:

| Índice | Clase | Notas |
|---|---|---|
| 0 | `anillo` | |
| 1 | `collar` | Incluye rosario. |
| 2 | `cadena` | |
| 3 | `arete` | Incluye candonga (aro). |
| 4 | `topo` | Incluye piercing. |
| 5 | `pulsera` | Incluye tobillera. |
| 6 | `dije` | |
| 7 | `persona` | Sale de YOLO-World (el maestro), sin revisar. |
| 8 | `cara` | Igual que persona. |

- **Tipos que se juntan.** El núcleo no los distingue como joya aparte, así que se suman a la clase más cercana (`TYPE_MAP`): candonga → arete, piercing → topo, tobillera → pulsera, rosario → collar.
- **No hay clase genérica `joya`,** como la que sí tiene YOLO-World. Una propuesta del maestro con etiqueta `joya` toma el tipo que diga CLIP, o `dije` si CLIP no dice nada (`labels.proposals`).
- **Umbrales.** `jewel_det.classes.json` fija un umbral de trabajo por clase de joya: 0.4 en las 7 del modelo instalado. Se eligió en la validación revisada con `choose_thr.py`. Cómo se aplica en Rust está en `docs/modelos.md` §11.
- **Banda débil.** Opcional, con `weak_threshold`. El modelo instalado no la usa.

---

## 2. Los datos

### Qué está en Git y qué no

Los datos **no van en el repositorio** (`.gitignore`: `/training/*`). Solo se suben tres cosas:

- `training/scripts/*.py`;
- `training/e2e/`: el crate `camcuts-e2e` (código fuente; sus registros no);
- `training/bench/*.json`: las detecciones de Rust sobre los cuadros de evaluación (r1 en CPU, r2 en FP16 y FP32, YOLO-World x FP32) y la lista de cuadros.

Todo lo demás vive solo en esta máquina.

### Dónde están en esta máquina

| Ruta | Qué hay | Tamaño |
|---|---|---|
| `D:\CONTENIDO NAPO\2026\` | El material original. **Solo lectura: nunca escribas bajo `D:\`.** | — |
| `training\data\img1920\`, `img960\` | 4425 cuadros en JPEG, a 1920 px de lado largo y a 960 px (lo que ve el detector). | 937 MB en total (`data\`) |
| `training\data\manos960\` | 114 cuadros de "Videos Yo Manos Detalle" (ronda de r6). | |
| `training\data\plan.json`, `frames.json` | De qué clip y en qué segundo sale cada cuadro. | |
| `training\data\split.json` | El reparto actual: 3834 de entrenamiento, 215 de validación y 398 de evaluación. | |
| `training\data\priority.txt`, `extra_*.txt`, `manos_r6.json` | Orden del maestro, cuadros añadidos después y manifiesto de Manos. | |
| `training\teacher\teacher.jsonl` | Las pseudoetiquetas del maestro: una línea por cuadro, 4425. | 12 MB (con `scores.jsonl`) |
| `training\teacher\scores.jsonl` | Puntuaciones CLIP de "macro", "cristal" y "recipiente", y las detecciones de r2, para el minado. | |
| `training\review\` | Veredictos de la revisión (`<conjunto>_decisions*.jsonl`), imágenes de revisión, listas de ids, hojas de contacto y métricas (`metricas_r2_r6_r7.json`). | 180 MB |
| `training\runs\r1`…`r7` | `weights\best.pt`, `last.pt`, `args.yaml` y `results.csv` de cada ronda. **r4 ya no existe.** | 220 MB |
| `training\runs_r*_log.txt` | El registro completo de cada entrenamiento. | |
| `training\weights_dl\yolo11s.pt` | El modelo base (COCO) de r1. | 18 MB |
| `training\weights\clip\ViT-B-32.pt` | CLIP del maestro y de las simulaciones. Algunos scripts lo buscan en `%USERPROFILE%\.cache\clip`. | 338 MB |
| `training\staging_r6\` | El r6 exportado. Es idéntico al instalado: se comprobó el SHA-256. | 54 MB |
| `training\e2e_r6\bin_r6\`, `training\e2e_r5bin\` | Copias de prueba de la app con su `models\`. Los de `bin_r6` son enlaces duros. | |
| `training\e2e_runs\` | Salidas de las pruebas de punta a punta. `r6cmp\` es la comparación r2/r6. | 146 MB |
| `training\e2e\` | El crate `camcuts-e2e`, la CLI de pruebas con `bench`. Va en el repositorio. | |
| `bench_perf\modelos_r2\` | La copia de seguridad de r2, el modelo anterior a r6. | |
| `bench_perf\vista\carpeta_propio\cache\` | Las cachés de Manos que lee `manos_sheets.py`. | |

**Los conjuntos YOLO `training\dataset_r*` ya no existen.** Se borraron por espacio en disco y se rehacen con `build_dataset.py` y `build_manos.py`.

**Dependencias fuera del repositorio.** Varios scripts usan rutas absolutas de esta máquina:

- **La app de referencia en Python, `C:\Users\Usuario\Tools\camcuts_next`:**
  - `teacher.py`, `camcuts_metrics.py`, `sim_weak.py` y `e2e_diff.py` importan su código;
  - `eval_r6.py` y `r5_quick.py` llaman a `tools\parity.py`;
  - ahí están los `.pt` de YOLO-World, de la pose y de SAM.

  Se lee, nunca se edita.
- **El material de `D:`.**
- **El candado de GPU:** `gpu_turn.py` usa `C:\Users\Usuario\Tools\camcuts_rs\.gpu_lock`.

**Entorno de Python.** Los registros muestran Ultralytics 8.4.192, PyTorch 2.11.0+cu128 y Python 3.12.10. En esta máquina, el venv de la app de Python está en `C:\Users\Usuario\Tools\camcuts\.venv`. No instales nada en él.

### De dónde salen las etiquetas

El proceso tiene tres fuentes: un **maestro** que propone, una **revisión a mano** que decide y un **minado** que elige qué revisar.

#### 1. Muestreo (`sample_frames.py`)

- Se recorrió `D:\CONTENIDO NAPO\2026`: 1951 clips en 132 carpetas.
- Se eligieron 801 clips repartidos por carpeta, para que ninguna sesión dominara. De ellos, 795 eran válidos: 10,1 h.
- Se planearon 3607 cuadros y se extrajeron 3585 a 1920 px (`data\sample_log.txt`).
- Más tarde, `extract_more.py`, `mine_macro.py` y la ronda de Manos llevaron el total a 4425.

#### 2. Reparto (`split.py`)

| Parte | Qué contiene | Para qué |
|---|---|---|
| Evaluación (`eval`) | 13 carpetas enteras apartadas (Octubre 27, Candongas, Anillo, Pulseras, Dijes…) y el clip difícil `julio\28\C0200` | No entra nunca en el entrenamiento. |
| Validación (`val`) | 45 clips sueltos | Sigue el entrenamiento y elige el umbral. |
| Entrenamiento | El resto | |

#### 3. Maestro (`teacher.py`)

El pipeline de Python a umbrales bajos sobre cada cuadro de 960 px. Si el cuadro es plano o LOG, se estira el contraste, como hace el análisis.

1. YOLO-World x con los prompts de `jewel.py`, a conf ≥ 0.02.
2. CLIP: probabilidad de joya y los tres tipos más probables de cada candidato.
3. Pose y una segunda mirada a 1920 px en orejas, cuello y muñecas, como `--zoom`.
4. MobileSAM: una caja ajustada a la silueta.

Todo se guarda en `teacher.jsonl`, también lo débil. `labels.proposals` decide qué se propone.

#### 4. Revisión a mano

- **`make_review.py`** dibuja cada cuadro con sus cajas numeradas y recortes ampliados (`render.py`).
- **Veredictos:** se escriben en `review/<conjunto>_decisions*.jsonl`. Por caja: `ok`, otra clase o `no`. Además: `fix` (caja corregida), `add` (pieza añadida) y `unsure` (el cuadro se descarta).
- **`decisions.gt`** convierte los veredictos en cajas.
- **Auditoría de piezas largas.** Con `mosaic_review.py audit` se extendieron a toda la pieza las cajas parciales de pulsera, cadena y collar (`audit_decisions*.jsonl`). `decisions.apply_audit` la aplica a los conjuntos `gold`, `more`, `r2` y `r3`.
- **`class_sheets.py`** hace hojas por clase para comprobar a ojo que cada clase es coherente.

Conjuntos revisados:

| Conjunto | Veredictos (descartados) | Cómo se eligieron |
|---|---|---|
| `eval` | 327 (26) → **301 cuadros de evaluación** | Toda la parte apartada. |
| `gold` | 300 (68) | `select_gold.py`: Julio 28, orejas, cajas enormes, desacuerdo de etiquetas y al azar. |
| `more` | 500 (36) | 500 cuadros más de entrenamiento. La lista está en `review/more_ids.txt`; no queda el script que la eligió. |
| `r2` | 300 (48) | `mine.py`: donde r1 y el maestro no coinciden, y clases escasas. |
| `r3` | 484 (76) | `select_r3.py`: macro, recipientes, cadena frente a pulsera, cristal, dijes y cadenas. |
| `evm` | 70 (2) | Evaluación macro: cuadros nuevos de clips macro apartados. |
| `r5` | 163 (15) | `mosaic_review.py new`: macro de piezas largas y negativos. |
| `audit` | 472 | Correcciones de piezas largas. |

**Personas y caras no se revisan.** Salen del maestro: persona ≥ 0.35 y cara ≥ 0.25.

#### 5. Minado

Cada ronda busca dónde se equivoca el detector anterior:

- **`mine.py`:** desacuerdos con el maestro.
- **`score_frames.py`:** puntúa todos los cuadros con CLIP (macro, cristal, recipiente) y con el detector.
- **`select_r3.py` y `mine_macro.py`:** eligen y extraen cuadros macro.
- **`manos_sheets.py` y `build_manos.py`:** la ronda de anillos sueltos.

---

## 3. Qué hace cada script de `training/scripts`

Ejecuta los scripts **desde `training\`**: las rutas relativas (`runs\…`, `dataset_…`) son relativas a esa carpeta.

### Turnos de GPU

Los que usan la GPU piden turno con `gpu_turn.GpuTurn`:

- toman `.gpu_lock`;
- si está ocupado, esperan dejando `.gpu_wait`;
- con DaVinci abierto, esperan a que la GPU baje del 60 % y queden 2 GB libres.

**`evaluate.py`, `choose_thr.py` y `mine.py` no piden turno por sí solos.** Toma el turno tú o llámalos desde un script que lo haga.

### Muestreo y reparto

| Script | Qué hace | GPU |
|---|---|---|
| `sample_frames.py` | Elige clips de `D:` y extrae cuadros a 1920 px → `data\img1920`, `plan.json`, `frames.json`. | No (ffmpeg) |
| `split.py` | Reparto eval/val/train y orden del maestro → `split.json`, `priority.txt`. | No |
| `extract_more.py` | Cuadros extra de clips concretos, con ids nuevos desde 04000. | No |
| `mine_macro.py` | Cuadros nuevos de clips con planos macro de piezas largas. | No |

### Maestro y puntuación

| Script | Qué hace | GPU |
|---|---|---|
| `teacher.py` | Pseudoetiquetas → `teacher\teacher.jsonl` y `data\img960`. | Sí (turno, freno 0.8) |
| `score_frames.py` | CLIP de cuadro entero (macro, cristal, recipiente) y detecciones de un modelo propio → `teacher\scores.jsonl`. | Sí (turno, freno 0.6) |

### Revisión

| Script | Qué hace | GPU |
|---|---|---|
| `make_review.py` | Imágenes y JSON de revisión de un conjunto, con propuestas del maestro o de `--props`. | No |
| `render.py` | Dibujo de las imágenes de revisión (módulo). | No |
| `select_gold.py` | Elige los cuadros "oro". | No |
| `mine.py` | Elige cuadros para la ronda 2 y sus propuestas. | Sí (sin turno) |
| `select_r3.py` | Elige cuadros para la ronda 3 y la evaluación macro. | No |
| `mosaic_review.py` | Revisión rápida en mosaicos de 4: `audit` o `new`. | No |
| `mosaic.py` | Un mosaico de cuadros para echar un vistazo. | No |
| `class_sheets.py` | Hojas de contacto por clase. | No |
| `manos_sheets.py` | Hojas de contacto de Manos con las cajas de la caché del detector instalado. | No (ffmpeg) |

### Conjunto y entrenamiento

| Script | Qué hace | GPU |
|---|---|---|
| `build_dataset.py` | Conjunto YOLO desde los veredictos, con aumentos → `dataset_<x>\data.yaml`. | No |
| `build_manos.py` | La ronda de Manos en tres pasos: `extraer`, `hojas` y `escribir`. Añade cuadros al conjunto. | No (r2 en la CPU) |
| `train.py` | Entrenamiento con turno y freno. | Sí |
| `export_onnx.py` | Exporta `best.pt` a ONNX FP32 o FP16 y escribe `jewel_det.classes.json`. | FP16: CUDA |

### Evaluación

| Script | Qué hace | GPU |
|---|---|---|
| `evaluate.py` | Métricas en la evaluación apartada: propio frente a YOLO-World x + CLIP. Define `evaluate`, `cross_class` y `student_preds`. | Sí (sin turno) |
| `evaluate_r3.py` | Lo mismo con dos modelos propios y el corte macro. | Sí |
| `choose_thr.py` | El umbral de trabajo que maximiza el F1 de "joya" en la validación. | Sí (sin turno) |
| `eval_r6.py` | Comparación completa de rondas: `metricas`, `exportar`, `e2e`, `manos` y `paridad`. | Sí |
| `e2e_run.py` | Análisis de punta a punta con `camcuts-e2e.exe` (el método antiguo). | Sí |
| `e2e_diff.py` | Imágenes lado a lado de los momentos que aprueba solo uno de dos análisis. | No (ffmpeg) |
| `sim_weak.py` | Simula la banda débil con CLIP sin reentrenar y elige `lo`. | Sí |
| `sim_merge.py` | Simula unir las cajas partidas de piezas largas. | Sí |
| `r5_quick.py` | En un turno: umbral, exportación y prueba rápida con C9181. | Sí |
| `dump_frames.py` | Escribe los cuadros de evaluación en BGR crudo (`bench\frames.bin` y `frames.json`). | No |
| `bench_eval.py` | Cronometra y puntúa en Rust el propio y YOLO-World (`camcuts-e2e bench`). | Sí |

### Módulos comunes

| Script | Qué hace | GPU |
|---|---|---|
| `labels.py` | Clases, IoU, lectura del maestro, `baseline` (lo que aprueba la app con YOLO-World x + CLIP) y `proposals`. | — |
| `decisions.py` | Veredictos → cajas (`gt`) y auditoría (`apply_audit`). | — |
| `camcuts_metrics.py` | Importa `is_flat` y `normalize_contrast` de la app de referencia. | — |
| `gpu_turn.py` | `GpuTurn`, `Throttle`, `duty` y `someone_waiting`. | — |

**Ojo: "ronda" no es lo mismo que "r".** Los docstrings numeran rondas de datos, y las ejecuciones se llaman r1…r7:

| Ronda | Ejecuciones |
|---|---|
| 2 | r2 |
| 3 | r3, r4, r5 y la banda débil |
| 4 | r6 y r7 (Manos) |

---

## 4. Entrenar

### Construir el conjunto

```powershell
cd C:\Users\Usuario\Tools\camcuts_rs\training
python scripts/build_dataset.py --out dataset_r8 --sets gold,more,r2 [--aug 0.35] [--dup arete,topo] [--no_audit]
```

`build_dataset.py`:

- **Qué cuadros usa:** los revisados de los conjuntos de `--sets`, sin los `unsure`, de las partes train y val. La evaluación nunca entra.
- **Qué imagen guarda:** la de 960 px como la ve el detector, con el contraste estirado si es plana.
- **Aumentos fuera de línea** (`--aug` es la probabilidad de cada uno; 0,35 por defecto):
  - LOG simulado y, en los clips LOG, la versión sin estirar;
  - desenfoque gaussiano o de movimiento;
  - un recorte vertical 9:16 alrededor de una joya.
- **`--dup`:** repite los cuadros de esas clases con volteo.
- **`--no_audit`:** reproduce las etiquetas sin la auditoría de piezas largas, que son las de r2.

Para la ronda de Manos se añaden cuadros al conjunto ya construido:

```powershell
python scripts/build_manos.py extraer            # cuadros exactos de D: + cajas de r2 → data\manos960, data\manos_r6.json
python scripts/build_manos.py hojas              # review\manos_r6\hoja_*.jpg para revisar
python scripts/build_manos.py escribir --out dataset_r6 [--excluir C9282,C9284]
```

**En Manos no se etiqueta caja por caja.**

- Las cajas son las del detector r2.
- La clase se corrige por clip: todas las piezas de un clip son del mismo tipo (`TRAIN_CLIPS`).
- `review\manos_r6_drop.txt` quita los cuadros malos: 11.
- Los cuadros difíciles, en los que r2 se equivocaba de clase, llevan una copia volteada.
- Los clips de `APARTADOS` no entran: sirven para medir si el arreglo generaliza.

### Entrenar

```powershell
python scripts/train.py --data dataset_r8\data.yaml --name r8 --model runs\r6\weights\best.pt `
    --epochs 10 --batch 4 --lr0 0.0005 --optimizer AdamW --warmup 1 --close_mosaic 3
```

**Opciones de `train.py`:**

| Opción | Por defecto | Notas |
|---|---|---|
| `--data`, `--name` | — | Obligatorias. Los resultados van a `runs\<name>`. |
| `--model` | `weights_dl\yolo11s.pt` | Para afinar, pasa el `best.pt` del modelo instalado. |
| `--epochs` | 30 | |
| `--imgsz` | 960 | |
| `--batch` | 12 | r6 y r7 usaron 4. |
| `--lr0` | 0.01 | Con `--optimizer auto`, Ultralytics ignora `lr0` y elige AdamW 0.000769. Así se entrenaron r1–r5. |
| `--optimizer` | `auto` | Para afinar, fíjalo (`AdamW`) con un `lr0` bajo. |
| `--warmup` | 3.0 | Épocas de calentamiento. |
| `--close_mosaic` | 6 | Últimas épocas sin mosaico. |
| `--freeze`, `--workers` | 0, 4 | |
| `--duty` | 0.55 | Freno de la GPU. Se recalcula cada época (ver abajo). |
| `--resume` | — | `--model` es un `last.pt` que se continúa. |

**Hiperparámetros fijos en `train.py`:**

- `cache="ram"`, `cos_lr=True`, `patience=100`, `amp=True`, `seed=1`;
- aumentos en línea: `mosaic=1.0`, `fliplr=0.5`, `scale=0.5`, `translate=0.1`, `degrees=0`, `mixup=0`, `hsv_h=0.015`, `hsv_s=0.6`, `hsv_v=0.4`;
- `check_amp` se da por bueno sin descargar nada.

**Turnos y freno de GPU:**

- Toma `GpuTurn` al empezar.
- Tras cada lote duerme lo necesario para no pasar del ciclo útil.
- Cada época recalcula el ciclo con `gpu_turn.duty()`: 0.6 con DaVinci abierto y 0.88 sin él.
- Si otro agente espera (`.gpu_wait`) desde hace más de 25 minutos, suelta el candado y la VRAM, deja pasar al otro y vuelve a pedir turno sin parar el proceso.

---

## 5. Exportar (`export_onnx.py`)

```powershell
python scripts/export_onnx.py --model runs\r8\weights\best.pt --out staging_r8\jewel_det.onnx --thr 0.4
python scripts/export_onnx.py --model runs\r8\weights\best.pt --out staging_r8\jewel_det_fp16.onnx --thr 0.4 --fp16
```

### Opciones

| Opción | Qué hace |
|---|---|
| `--model` | El `best.pt`. Sus clases deben ser exactamente `labels.CLASSES` (hay un `assert`). |
| `--out` | El `.onnx` de salida. **`jewel_det.classes.json` se escribe en la misma carpeta.** |
| `--fp16` | Pesos y cálculo en FP16, exportado en CUDA, con entrada y salida en f32. |
| `--thr` | Umbral de trabajo de las 7 joyas. **Por defecto 0.3: pasa siempre `--thr 0.4`**, el umbral del modelo instalado. |
| `--thr_json` | JSON `{clase: umbral}` que sobrescribe clases concretas. |
| `--weak` | Escribe `weak_threshold` (la banda débil para CLIP). Sin la opción no hay banda débil. |

### Qué sale

- **El modelo:** `images` f32 `[lote, 3, alto, ancho]` dinámica → `output0` `[lote, 13, anclas]` (4 coordenadas + 9 clases, ya con sigmoide). Es el formato que lee `yolo.rs`.
- **Cómo se exporta:** con `torch.onnx.export` (TorchScript, opset 17) y la cabeza `Detect` en modo exportación. No necesita el paquete `onnx`: se salta el único paso que lo pide.
- **El JSON:** `names`, `imgsz`, `thresholds`, `default_threshold`, `source` y, opcionalmente, `weak_threshold`.

### Exporta siempre a una carpeta de staging

No exportes directamente a `models\`. Machacarías el modelo instalado y su JSON a medias.

`eval_r6.py exportar` hace las dos exportaciones a `training\staging_<tag>\` y prepara la copia de prueba de la app:

```powershell
python scripts/eval_r6.py exportar --tag r8 --thr 0.4
```

---

## 6. Evaluar

Hay dos niveles. **Los dos hacen falta antes de instalar.**

### a) En cuadros, contra la verdad revisada

**Conjuntos de referencia:**

- **Evaluación apartada:** los 301 cuadros revisados de `eval`, con 725 joyas.
- **Validación revisada:** los cuadros de `val` de `gold`, `more`, `r2`, `r3` y `r5`, con 130 joyas.

**Métricas** (`evaluate.evaluate`):

- AP50 por clase, con interpolación de todos los puntos;
- precisión y exhaustividad en el umbral de trabajo;
- **"joya" sin clase**: ¿hay una joya ahí? Es lo que más pesa en CAMCUTS.

Las predicciones pasan antes por `cross_class`, la misma supresión entre clases de `trained.rs`.

**La base de comparación** es la app con YOLO-World x + CLIP (`labels.baseline`): fuertes ≥ 0.15, más hasta 4 débiles confirmadas por CLIP (p ≥ 0.6).

| Script | Uso |
|---|---|
| `evaluate.py --model runs\rX\weights\best.pt --thr 0.4` | El propio frente a la base, en el apartado, con el mejor umbral de F1 a título informativo. |
| `evaluate_r3.py --old … --new …` | Dos modelos propios y la base, en el apartado y en el corte macro (`evm` + `scores.macro` ≥ 0.5). |
| `choose_thr.py --model …` | Umbral que maximiza el F1 de "joya" en la **validación** (nunca en la evaluación). |
| `eval_r6.py metricas --modelos r2,r6,r7` | Lo anterior para varios modelos, con F1 por umbral y matriz de confusión anillo/arete/topo. Guarda `review\metricas_<modelos>.json`. |
| `sim_weak.py --tune` / `--eval --lo 0.25` | La banda débil simulada. |
| `sim_merge.py` | La unión simulada de piezas largas. |
| `dump_frames.py` + `bench_eval.py <models_dir>` | Las detecciones que devuelve **Rust** (con reescalado incluido), puntuadas contra la verdad. |

**`bench_eval.py` necesita cosas fuera del repositorio:**

- `camcuts-e2e.exe`, del crate `training\e2e`. Se compila con `cargo build --release --manifest-path training/e2e/Cargo.toml --target-dir target`.
- `bench\frames.bin`. Ya no existe y se rehace con `dump_frames.py`.

### b) De punta a punta, contra las bases

Las carpetas de referencia:

| Carpeta | Clips | Notas |
|---|---|---|
| `D:\CONTENIDO NAPO\2026\Octubre\27\videos` | 19 | Apartada del entrenamiento. Incluye C9181, una pulsera macro con poca profundidad de campo. |
| `D:\CONTENIDO NAPO\2026\Octubre\8\Candongas` | 9 | Apartada. |
| `D:\CONTENIDO NAPO\2026\septiembre\26\Anillo` | 9 | Apartada. |
| `D:\CONTENIDO NAPO\2026\Noviembre\3\Videos Yo Manos Detalle` | 33 | Anillos y topos. Algunos clips entrenaron r6; los de `APARTADOS` no. |

**Qué se compara:** los segundos aprobados (la suma de `duracion_s` de `selects_joyas.csv`, criterio "joyas"), el número de tramos y las etiquetas. Las bases son el modelo instalado y YOLO-World x.

```powershell
python scripts/eval_r6.py e2e --bins r2,r8 --folders manos,oct27     # camcuts.exe --analizar … --detector propio --no-proxy --no-sheet --sin-respaldo
python scripts/eval_r6.py manos --tags r2,r8                         # % de cada etiqueta en Manos, por grupo de clips
python scripts/eval_r6.py paridad --tags r2,r8                       # parity.py de la app de referencia: aprobados por clip
```

**Cuidados al comparar:**

- **`--sin-respaldo`:** se compara el detector, no el respaldo con YOLO-World.
- **La etiqueta `r2` no es r2.** `eval_r6.py` (`bin_of`) usa con ella el `camcuts.exe` de `target\release` con los modelos de `models\` del workspace, es decir, **el instalado**: hoy r6. Las demás etiquetas usan `training\e2e_r6\bin_<tag>\`.
- **La copia de prueba usa el exe que ya tenga.** `exportar` copia `camcuts.exe` en `bin_<tag>\` solo si no hay uno. Para comparar con el mismo código, borra antes ese exe.
- **Usa carpetas de salida nuevas.** Las salidas van a `training\e2e_runs\r6cmp\<carpeta>_<tag>`. Si ya existen, CAMCUTS **reutiliza sus cachés**, porque la caché guarda qué detector se usó pero no qué versión del modelo. Borra esas carpetas o usa una etiqueta nueva.
- **`e2e_run.py`, `e2e_diff.py` y `r5_quick.py`** son el método de las primeras rondas, con `camcuts-e2e.exe`.

---

## 7. Historial de rondas

### Resumen

"Val" es la validación de Ultralytics al final del entrenamiento: 44 cuadros y 179 cajas, salvo r5 (53 y 197). No sirve para comparar entre rondas con datos distintos; las decisiones se tomaron con las pruebas de los apartados a) y b).

"Optimizador" es el que se usó de verdad. De r1 a r5 se pasó `--optimizer auto`, así que Ultralytics ignoró el `lr0` pedido y eligió AdamW 0.000769: lo dicen los registros (`runs_r*_log.txt`). `args.yaml` guarda el `lr0` pedido.

| Ronda | Fecha | Parte de | Imágenes de entrenamiento | Épocas, lote, optimizador | Val P / R / mAP50 | Resultado |
|---|---|---|---|---|---|---|
| r1 | 2026-10-30 | `yolo11s.pt` (COCO) | 1116 (`gold` + `more`) | 30, 12, AdamW 0.000769 (pedido 0.01) | 0,763 / 0,594 / 0,736 | Primera versión. |
| r2 | 2026-10-30 | r1 | 1532 (+ `r2`) | 18, 12, AdamW 0.000769 (pedido 0.003) | 0,682 / 0,699 / 0,727 | **Instalada el 30/10**, umbral 0.4. |
| r3 | 2026-10-30 | r2 | 2271 (+ `r3`: macro, recipientes…) | 20, 12, AdamW 0.000769 (pedido 0.003) | 0,746 / 0,657 / 0,748 | No se instaló. |
| r4 | 2026-10-30 | r3 | 2788 | 8, 12, AdamW 0.000769 (pedido 0.002) | 0,784 / 0,653 / 0,769 | No se instaló. No quedan pesos. |
| r5 | 2026-11-03 | r2 | 2494 (+ auditoría + `r5`) | 10, 12, AdamW 0.000769 (pedido 0.002) | 0,646 / 0,693 / 0,727 | No se instaló. |
| r6 | 2026-11-05 | r2 | 1748 (conjunto de r2 + Manos) | 10, 4, AdamW 0.0005 (fijado) | 0,763 / 0,679 / 0,742 | **Instalada el 05/11.** |
| r7 | 2026-11-05 | r2 | 1703 (r6 sin los topos de maniquí) | 10, 4, AdamW 0.0005 (fijado) | 0,764 / 0,577 / 0,677 | Descartada. |

### r1 y r2: el primer modelo útil (2026-10-30)

**Qué se hizo:**

- **r1:** sale de YOLO11s de COCO con los cuadros `gold` y `more`. Tardó 23 min.
- **r2:** afina r1 con los 300 cuadros minados donde r1 y el maestro no coincidían.
- **Umbral:** se eligió 0.4 en la validación.
- **Instalación:** el 30/10.

**Resultado de punta a punta**, con el código de ese día. Esta tabla solo se puede comparar consigo misma:

| Carpeta | YOLO-World x | r2 |
|---|---|---|
| Octubre 27 | 709,8 s | 727,3 s |
| Candongas | 538,3 s | 539,0 s |
| Anillo | 412,8 s | 426,3 s |

**El punto débil: C9181**, la pulsera macro. r2 aprobaba 1,75 s y YOLO-World x 61 s.

### Banda débil sin reentrenar (2026-10-30)

**Qué se hizo:**

- **`sim_weak.py`:** pasa por CLIP las detecciones entre `lo` y 0.4, como hace el núcleo con YOLO-World.
- **Rust:** se añadió `weak_threshold` a `trained.rs` y `--weak` a `export_onnx.py`.

**Resultado:** con r2, Octubre 27 pasó de 727,3 a 733,0 s, y C9181 de 1,75 a 3,75 s.

**Decisión: no se activó.** Ni r2 ni r6 llevan `weak_threshold` en su JSON. El código sigue disponible.

### r3 y r4: los puntos débiles (2026-10-30)

**Qué se hizo:**

- **r3** añadió 484 cuadros revisados de macro, recipientes, cristal, dijes y cadenas.
- **r4** continuó r3 con 8 épocas y 2788 imágenes. Se pidió un `lr0` menor, pero con `auto` no se aplicó.

**Resultado de r3 de punta a punta:**

- Octubre 27: 710,0 s.
- C9181: 0 s aprobados (joya en el 3 % de las muestras).
- Candongas: 530,5 s; Anillo: 428,8 s.

**Decisión:** no mejoraban a r2 y no se instalaron. De r4 no quedan pesos ni prueba de punta a punta.

### r5: las piezas largas en macro (2026-11-03)

**Qué se hizo:**

- Se auditaron las cajas parciales de pulsera, cadena y collar (472 cuadros).
- Se añadieron 163 cuadros de macro y negativos.
- El entrenamiento se cortó tres veces por turnos de GPU y se reanudó desde la época 5 (`runs_r5b_log.txt`).

**Resultado de `r5_quick.py`:** C9181 sacó **0 tramos**, con y sin la banda débil (`weak_threshold` 0.25, en `e2e_r5bin`).

**Decisión:** no se instaló. `sim_merge.py` (unir cajas partidas) tampoco se adoptó: no está en `trained.rs`. La solución para C9181 fue el **respaldo automático con YOLO-World** (`INTERFACES.md`, 2026-11-03).

### r6: anillos sueltos y topos (2026-11-05), la instalada

**El problema.** En "Videos Yo Manos Detalle", r2 llamaba "arete" a muchos anillos sueltos (C9271 y C9278 salían "arete" al 100 %) y "anillo" a topos en una oreja de maniquí (C9282).

**Qué se hizo:**

- Se usó el conjunto de r2 (1532 imágenes; `build_dataset.py --no_audit` reproduce sus etiquetas).
- Se añadieron 103 cuadros de Manos con sus aumentos: 216 imágenes.
- Se afinó desde r2 con AdamW, `lr0` 0.0005, lote 4 y 10 épocas. Tardó 13 min.

**Resultados en los clips apartados de Manos** (proporción de la etiqueta correcta en las cajas de la caché; recalculado de `e2e_runs\r6cmp`):

| Grupo | r2 | r6 |
|---|---|---|
| Anillos sueltos (C9265, C9270, C9271, C9273, C9275, C9278) | 66,0 % anillo (34,0 % arete) | **99,9 %** anillo |
| Topos (C9280, C9281, C9283, C9285, C9286) | 71,2 % topo | **95,3 %** topo |
| Anillos puestos (C9255, C9257, C9259, C9261) | 99,9 % | 100 % |

**Resultados de punta a punta**, con `--sin-respaldo`:

| Carpeta | r2 | r6 |
|---|---|---|
| Manos | 2127,0 s (77 tramos) | 2137,8 s (75 tramos) |
| Octubre 27 | 725,5 s (50 tramos) | 827,5 s (58 tramos) |

En Octubre 27 la subida viene sobre todo de **C9181, que pasa de 0 s a 83 s** sin respaldo. YOLO-World x da 61 s. La regla del respaldo automático se calibró con r2.

**Resultados en cuadros** (`review\metricas_r2_r6_r7.json`, umbral 0.4):

| Medida | r2 | r6 |
|---|---|---|
| mAP50 en el apartado | 0,497 | 0,497 |
| AP50 de "joya" | 0,701 | 0,701 |
| AP50 de topo | 0,714 | 0,750 |
| AP50 de anillo | 0,633 | 0,619 |
| AP50 de "joya" en la validación | 0,776 | 0,801 |

`review\r6_metricas.json`, de una ejecución anterior el mismo día, da cifras algo distintas para r6. Usa la de r2/r6/r7.

**Coste conocido.** Un sello en un puño (**C8726**, en la evaluación apartada) sale "topo". Según `eval_r6.py` y `build_manos.py`, los topos de la oreja de maniquí llevaron sellos y anillos de cabeza redonda a "topo".

**Instalación:** el 05/11. r2 quedó en `bench_perf\modelos_r2`. El 2026-11-06 se comprobó que `staging_r6` es idéntico a lo instalado (SHA-256) y que `dist\CAMCUTS` lleva r6.

### r7: r6 sin los topos de maniquí (2026-11-05), descartada

**La idea:** quitar C9282 y C9284 (`build_manos.py escribir --excluir C9282,C9284`) para arreglar el sello.

**Resultado:**

| Medida | r6 | r7 |
|---|---|---|
| Exhaustividad de topo en el apartado | 0,77 | 0,64 |
| AP50 de topo en el apartado | 0,750 | 0,705 |
| mAP50 en la validación | 0,584 | 0,544 |

La prueba de punta a punta no llegó a terminarse.

**Decisión:** se quedó r6.

---

## 8. Instalar un modelo nuevo de forma segura

### Reglas

- **Los tres archivos van juntos:** `jewel_det.onnx`, `jewel_det_fp16.onnx` y `jewel_det.classes.json`.
- **Antes de instalar, compara contra el instalado** en cuadros y de punta a punta.
- **Siempre deja una copia del anterior.**
- **Nunca escribas bajo `D:\`:** las salidas van a `C:`.
- **Sigue `GPU_TURNOS.md`.**

### Pasos

1. **Entrena** (§4) → `training\runs\<tag>\weights\best.pt`.

2. **Comprueba el umbral en la validación.** Si no hay pruebas de que otro valor sea mejor, quédate con 0.4.

   ```powershell
   python scripts/choose_thr.py --model runs\<tag>\weights\best.pt
   ```

3. **Mide en cuadros** contra el instalado. Revisa la matriz anillo/arete/topo y el F1 de "joya".

   ```powershell
   python scripts/eval_r6.py metricas --modelos r6,<tag>
   ```

4. **Exporta a staging.** No a `models\`.

   ```powershell
   python scripts/eval_r6.py exportar --tag <tag> --thr 0.4      # → training\staging_<tag>\ y training\e2e_r6\bin_<tag>\
   ```

5. **Prueba de punta a punta** contra el instalado, con carpetas de salida nuevas (§6b):

   ```powershell
   python scripts/eval_r6.py e2e --bins r2,<tag>      # "r2" = exe del workspace con el modelo instalado
   python scripts/eval_r6.py manos --tags r2,<tag>
   python scripts/eval_r6.py paridad --tags r2,<tag>
   ```

   **Criterio:**
   - el tiempo aprobado no baja en Manos ni en Octubre 27;
   - las etiquetas de los clips apartados mejoran o se mantienen;
   - no hay confusiones nuevas en la evaluación apartada (por ejemplo, C8726).

   Anota las cifras.

6. **Cierra cualquier análisis en marcha.** `ShapedSession` vuelve a leer el `.onnx` del disco cada vez que compila una forma nueva. Si cambias el archivo a mitad de un análisis, un mismo análisis mezclaría dos modelos.

7. **Copia de seguridad del instalado**, como se hizo con r2:

   ```powershell
   $r = 'C:\Users\Usuario\Tools\camcuts_rs'
   New-Item -ItemType Directory -Force "$r\bench_perf\modelos_r6" | Out-Null
   Copy-Item "$r\models\jewel_det.onnx", "$r\models\jewel_det_fp16.onnx", "$r\models\jewel_det.classes.json" "$r\bench_perf\modelos_r6\"
   ```

8. **Instala** los tres archivos desde staging:

   ```powershell
   Copy-Item "$r\training\staging_<tag>\jewel_det*" "$r\models\"
   ```

9. **Regenera y comprueba el manifiesto:**

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1 -Generar
   powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1          # 19 correctos, 0 con problemas
   ```

10. **Regresión** con el código actual:

    ```powershell
    cargo test -p camcuts-infer --lib
    cargo test -p camcuts-core
    cargo run -p camcuts-infer --release --example analizar -- "D:\CONTENIDO NAPO\2026\Noviembre\3\Videos Yo Manos Detalle" --out bench_perf\regresion\manos_<tag> --no-proxy --no-sheet
    cargo run -p camcuts-infer --release --example analizar -- "D:\CONTENIDO NAPO\2026\Octubre\27\videos" --out bench_perf\regresion\oct27_<tag> --no-proxy --no-sheet
    ```

    `D:` solo se lee: las salidas van a `C:`. Compara con la regresión anterior en `bench_perf\regresion\manos` y `oct27`: el 2026-11-06, con r6, salieron 75 y 58 tramos. Esta vez el respaldo automático va activado, así que comprueba también qué clips lo disparan.

11. **Empaqueta:**

    ```powershell
    powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Edicion joyas
    ```

    Copia solo lo que cambió y comprueba el SHA-256.

12. **Deja constancia:**
    - sube en un mismo commit `models\MANIFIESTO.sha256` y `models\jewel_det.classes.json`;
    - copia los `.onnx` a la carpeta compartida de modelos;
    - añade una entrada a `INTERFACES.md` como la de "Own model r6 installed": de dónde parte, qué datos añade, resultados y coste conocido.

### Las cachés no se invalidan solas

`detector::cache_vigente` solo mira si la caché es de `"propio"` o de `"world"`, no qué versión del modelo la hizo. **Un análisis ya hecho conserva los resultados del modelo anterior.** Para aplicar el modelo nuevo:

- borra las cachés del clip (`<resultados>\cache\*.json`), o
- analiza en una carpeta de resultados nueva.

### Volver atrás

1. Copia los tres archivos de la copia de seguridad (`bench_perf\modelos_<anterior>\`) a `models\`.
2. Regenera el manifiesto (`-Generar`) y compruébalo.
3. Vuelve a empaquetar.

`bench_perf\` no va en Git: las copias de seguridad son locales.

---

## Discrepancias encontradas

En todos los casos manda el código o los archivos actuales.

1. **`INTERFACES.md` dice que r6 se afinó "on 103 frames"** de Manos. El conjunto de r6 fueron 1748 imágenes: las 1532 de r2 más 103 cuadros de Manos y sus aumentos, 216 imágenes en total.
2. **`build_manos.py` llama a r2 "el mismo `best.pt` que `models/jewel_det.onnx`"**, y `eval_r6.py` (`bin_of`) llama "el desplegado (r2)" al exe del workspace. Desde el 05/11 el instalado es r6: con `--bins r2` se mide r6.
3. **El docstring de `train.py`** habla de un freno de "~55 %" y de soltar el candado "si el turno pasa de 27 min". El código recalcula el freno en cada época (0.6 con DaVinci, 0.88 sin él) y solo suelta el candado si otro espera desde hace más de 25 minutos. `TURN_S` no se usa.
4. **El docstring de `export_onnx.py`** propone exportar directamente a `../models/jewel_det.onnx` con `--thr 0.3`. El instalado usa 0.4 (`eval_r6.py exportar` lo pasa), y exportar a `models\` sobrescribe también el `classes.json` instalado.
5. **`bench_eval.py` necesita `bench\frames.bin`, que ya no existe.** (El crate `training/e2e`, que estos scripts usan para `camcuts-e2e.exe`, sí va en el repositorio desde el 2026-11-06 y compila con el código actual.)
6. **La cabecera de `trained.rs` describe la banda débil como parte del funcionamiento.** El modelo instalado (r6) no trae `weak_threshold`, así que en la práctica está apagada. No es un error, pero `INTERFACES.md` no lo dice.
7. **`args.yaml` de r1–r4 guarda `lr0` 0.01, 0.003 y 0.002, pero no se aplicaron.** Con `optimizer: auto`, los registros muestran que se usó AdamW 0.000769 en todos.
