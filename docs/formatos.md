# Formatos de archivo de CAMCUTS

**Qué es este documento.** Cada archivo que CAMCUTS lee o escribe, con sus campos, quién lo escribe y cómo. La tubería que los produce está en [analisis.md](analisis.md); la ventana, en [ventana.md](ventana.md); los modelos, en [modelos.md](modelos.md). Las decisiones y mediciones, en [INTERFACES.md](../INTERFACES.md).

**Regla de compatibilidad** (INTERFACES.md): una carpeta de resultados hecha por la app de Python se abre en Rust y al revés. Por eso los JSON imitan a `json.dumps` de Python (`pyfmt.rs`) y el OTIO a la librería `opentimelineio`. Si el código y este documento no coinciden, **manda el código**.

---

## Resumen

| Archivo | Lo escribe | Lo lee | ¿Atómico? |
|---|---|---|---|
| `cache\<clip>_<hash>.json` | Análisis (`analyse::analyse_prepared`, `cli::respaldar`) | CLI, ventana | **Sí** (`project::write_atomic`) |
| `cache\proxies\<clip>_<hash>.mp4` | ffmpeg del análisis; la ventana (`TrabajoProxies`) | Visor de la ventana; `sampler::proxy_vigente` | Sí (`.part.mp4` + renombrar) |
| `selects_<criterio>.otio` / `.csv` | CLI (`export::write_otio`, `export::write_csv`) | DaVinci Resolve, Excel | **Sí** |
| `CAMCUTS_<criterio>_<modo>.otio` / `.csv` | Ventana, botón Exportar (mismas funciones) | DaVinci Resolve, Excel | **Sí** |
| `<nombre>.camcuts` | Ventana (`proyecto::guardar`) | Ventana | Temporal + renombrar, sin reintentos |
| `colores.json` | Ventana | CLI (`--colores`), ventana | No |
| `clip_types.json` | Ventana | CLI, ventana | No |
| `clips_meta.json` | Ventana | Ventana | No |
| `clips.txt` | Ventana | CLI (`--lista`) | No |
| `necesitan_proxy.txt`, `errores.txt` | CLI | El editor | No |
| `models\calibration.json` | El editor (a mano) | CLI, ventana | — |
| `%APPDATA%\CAMCUTS-rs\settings.json` | Ventana | Ventana | No |
| `<clip>M01.XML`, XML incrustado, `.cube` | La cámara / el editor | Color y timecode | Solo lectura |

**Nunca se escribe bajo `D:\`** (el disco del material). Todo lo que escribe CAMCUTS va a la carpeta de resultados o a `%APPDATA%`.

---

## Carpeta de resultados

Por defecto está junto al material (`edicion::carpeta_resultados`): `<material>\CAMCUTS` en joyas y `<material>\CAMCUTS Bodas` en Bodas. **Cada edición tiene la suya**: nunca reutiliza ni pisa lo de la otra.

```text
<carpeta de resultados>\
├── cache\
│   ├── C0001_1a2b3c4d.json          caché de un clip
│   ├── C0001_1a2b3c4d.json.tmp      solo si una escritura no pudo reemplazar el .json
│   └── proxies\
│       ├── C0001_1a2b3c4d.mp4       proxy de vista previa
│       └── C0001_1a2b3c4d.part.mp4  solo mientras se escribe
├── Material.camcuts                 proyecto de la ventana
├── colores.json
├── clip_types.json
├── clips_meta.json
├── clips.txt
├── selects_joyas.otio / .csv        de la CLI
├── CAMCUTS_Equilibrado_v1v2.otio / .csv   de la ventana
├── necesitan_proxy.txt
└── errores.txt
```

---

## 1. Caché por clip (`cache/*.json`)

**Código:** `project::ClipEntry`, `jewel::Sample`, `jewel::Jewel`, `project::cache_file`, `project::write_entry`, `project::read_entry`.

Una caché por clip con **todas las medidas por muestra**. Puntuar y exportar solo leen esto: no se vuelve a abrir el video.

### 1.1 Nombre del archivo

`project::cache_file`, igual que `cache_file` de Python:
- `<nombre del clip>_<8 primeros hex del SHA-1 de la ruta absoluta en minúsculas>.json`. El hash separa clips con el mismo nombre de carpetas distintas.
- **Cachés antiguas sin hash** (`<nombre>.json`): se siguen usando si su `path` es este clip. La comparación es por componentes y sin mayúsculas, como `WindowsPath ==` (`project::path_key`): `D:\Contenido\…` y `D:\CONTENIDO\…` son el mismo clip.
- Prueba: `project::tests::nombre_de_cache_como_python`.

### 1.2 Campos del clip

En este orden en el archivo (serde_json con `preserve_order`):

| Clave | Tipo | Qué es | Si falta |
|---|---|---|---|
| `path` | texto | Ruta del medio original (a la que apunta el OTIO). | obligatoria |
| `decode_path` | texto o `null` | Proxy de cámara decodificado en lugar de un RAW. | `null` |
| `start_tc` | texto | Timecode de inicio; `;` antes de los cuadros = drop-frame. Puede ser `""`. | Se sondea al cargar si el archivo existe (cachés anteriores al timecode). |
| `fps` | `[num, den]` | Frecuencia exacta, p. ej. `[30000, 1001]`. | obligatoria |
| `width`, `height` | enteros | Tamaño **mostrado** (ya girado). | obligatorias |
| `duration` | segundos | Duración nominal del archivo. | obligatoria |
| `per_second` | número | Muestras por segundo (4.0). | obligatoria |
| `speech` | `[[ini, fin], …]` | Intervalos de voz en segundos (2 decimales). Vacío si nadie aparece o no hay audio. | `[]` |
| `samples` | lista | Una entrada por muestra (1.4). | obligatoria |
| `proxy` | texto o `null` | Proxy de vista previa, si existía al terminar. | `null` |
| `decode` | `"gpu"` / `"cpu"` | Dónde se decodificó (del último intento). | `"cpu"` |
| `lap_scale` | número | Corrección de nitidez aplicada: 1/1.103 en GPU, 1.0 en CPU. | 1.0 |
| `reused` | entero | Muestras quietas que reutilizaron la detección anterior. | 0 |
| `color` | `{"id", "cube"}` | Conversión de color con la que se analizó. | Automática (`{"id": "auto"}`) |

### 1.3 Claves de estado (al final del objeto)

Van después de los campos fijos, en el orden en que se añaden (`ClipEntry::extra`).

| Clave | Valor | Quién la escribe | Qué significa |
|---|---|---|---|
| `detector` | `"world"` / `"propio"` | `analyse_prepared` (solo joyas) | Detector que analizó (`detector::CLAVE_CACHE`). **Sin clave = YOLO-World** (Python y cachés antiguas), aunque hoy el predeterminado sea el propio. |
| `en_lugar_de` | `"world"` | `analyse_prepared`, si `Detector::en_lugar_de()` lo dice | El propio analizó porque YOLO-World no cabía (modo ligero). **Vale también para YOLO-World**: no se rehace al volver a tener memoria (`detector::CLAVE_EN_LUGAR`). |
| `respaldo` | `"propio"` | `cli::respaldar` | Análisis de YOLO-World de respaldo de un clip donde el propio casi nunca daba la joya nítida. **Vale para los dos detectores** (`detector::CLAVE_RESPALDO`). |
| `edition` | `"bodas"` | `analyse_prepared` (Bodas) | **Sin clave = joyas.** Una caché de la otra edición cuenta como vieja. |
| `sam` | `false` | `analyse_prepared`, si SAM falló en alguna caja | Esas cajas se midieron enteras, sin silueta (`analyse::CLAVE_SAM`). |
| `cortado` | `{"hasta": s, "bytes": n, "mtime": s}` | `analyse_prepared` | El archivo se acaba antes de su duración: solo se analizó hasta `hasta`. `bytes` y `mtime` son su huella (`analyse::CLAVE_CORTADO`). |

**Claves desconocidas se conservan** al leer y reescribir (`#[serde(flatten)]`), igual en `Sample` y `Jewel`. Así una caché de otra versión no pierde datos.

### 1.4 Campos de cada muestra (`Sample`)

Mismo orden de campos que la dataclass `Sample` de Python.

| Clave | Qué es |
|---|---|
| `t` | Segundos desde el inicio del clip (`i / per_second`). |
| `edge_density` | Fracción de píxeles de borde nítido del cuadro entero. |
| `lap_var` | Varianza del Laplaciano del cuadro × `lap_scale²`. |
| `mean_luma` | Luma media (0–1). |
| `clip_black`, `clip_white` | Fracción de píxeles ≤ 2 y ≥ 253. |
| `cam_dx`, `cam_dy` | Desplazamiento de cámara desde la muestra anterior, en píxeles de análisis. |
| `jewel` | ¿Hay joya? Si es `false`, los campos de joya valen 0. |
| `label` | Tipo de la joya protagonista (`anillo`, `collar`, `cadena`, `arete`, `topo`, `pulsera`, `dije`, `joya`). |
| `conf` | Confianza de la protagonista. |
| `box` | Caja de la protagonista `[x0, y0, x1, y1]` en píxeles de análisis (lado largo 960). |
| `area_frac` | Área de la caja / cuadro. |
| `touches_edge` | La caja toca el borde (≤ 2 px). |
| `jewel_edge_density` | Densidad de bordes nítidos de la protagonista. |
| `bg_edge_density` | Lo mismo fuera de todas las cajas agrandadas un 25 %. |
| `focus_ratio` | `jewel_edge_density / max(bg, 0.001)`. |
| `jewel_lap_var` | Varianza del Laplaciano de la protagonista × `lap_scale²`. |
| `jewel_lap_p99` | **Nitidez principal:** p99 de \|Laplaciano\| × `lap_scale`. |
| `anisotropy` | 1 = nítido; hacia 0 = barrido. Por defecto 1.0. |
| `jewel_speed` | Velocidad de la protagonista en diagonales de caja por segundo. |
| `n_jewels` | Grupos de joyas medidos. |
| `masked` | Medida sobre la silueta de SAM (no sobre la caja). |
| `mask_frac` | Área de la silueta dilatada / área de la caja. |
| `person`, `face` | Hay persona (conf ≥ 0.25) / se ve una cara. |
| `person_frac` | Caja de persona más grande / cuadro. |
| `worn` | El centro de la joya está dentro de una persona. |
| `with_labels` | Tipos de las piezas solapadas con la protagonista (dije → cadena). |
| `jewels` | Cada grupo de joyas, medido por separado (1.5). |
| `n_sharp` | Joyas con `lap_p99` ≥ 90. |
| `log` | Se trató como LOG (contraste estirado o color convertido). Puede faltar. |
| `speaking` | Cara visible y voz en ese instante. Puede faltar. |

Los campos que faltan en cachés antiguas toman su valor por defecto (0, `false`, lista vacía; `anisotropy` 1.0).

**En Bodas**, "jewel" significa **sujeto principal** y la estructura es la misma. Los campos propios de Bodas (momento, voz, cara…) van como claves extra de la muestra: ver [empaque/bodas/CLAUDE.md](../empaque/bodas/CLAUDE.md).

### 1.5 Campos de cada joya (`Jewel`, dentro de `jewels`)

| Clave | Qué es |
|---|---|
| `label` | Tipo de la pieza principal del grupo. |
| `with` | Tipos de las piezas solapadas. **Ojo:** en la joya se llama `with`; en la muestra, `with_labels`. |
| `conf`, `box` | Confianza y caja. |
| `zoom` | La encontró la segunda mirada a 1920 px (`--zoom`). |
| `area_frac`, `edge`, `ratio` | Área / cuadro, densidad de bordes y razón con el fondo. |
| `masked`, `mask_frac` | Silueta usada y su fracción de la caja. |
| `lap_var`, `lap_p99` | Nitidez de esta pieza (con `lap_scale`). |
| `anisotropy`, `speed` | Barrido y velocidad de esta pieza. |
| `touches_edge`, `worn` | Toca el borde / la lleva una persona. |
| `raw_label` | Tipo antes de estabilizar. Solo existe en memoria al puntuar; el análisis no lo escribe. |

Todos salvo `label`, `conf` y `box` son opcionales y se omiten si faltan. La verificación CLIP (`verified`) **no** se guarda.

### 1.6 Ejemplo abreviado

```jsonc
{"path": "D:\\Material\\C0001.MP4", "decode_path": null, "start_tc": "17:42:22;22",
 "fps": [30000, 1001], "width": 3840, "height": 2160, "duration": 35.035, "per_second": 4.0,
 "speech": [[1.25, 4.8]],
 "samples": [
   {"t": 0.0, "edge_density": 0.031, …, "jewel": true, "label": "anillo", "conf": 0.82,
    "box": [402, 210, 488, 291], …, "jewel_lap_p99": 141.3, …,
    "jewels": [{"label": "anillo", "with": [], "conf": 0.82, "box": [402, 210, 488, 291],
                "zoom": false, "area_frac": 0.013, …, "lap_p99": 141.3, …, "worn": true}],
    "n_sharp": 1, "log": true, "speaking": false},
   …],
 "proxy": "E:\\Resultados\\cache\\proxies\\C0001_1a2b3c4d.mp4", "decode": "gpu",
 "lap_scale": 0.9066…, "reused": 37, "color": {"id": "auto"}, "detector": "propio"}
```

Formato real: una sola línea, separadores `", "` y `": "`, caracteres no ASCII como `\uXXXX` y números como `repr(float)` de Python (`pyfmt::dumps`, `pyfmt::float_repr`).

### 1.7 ¿Cuándo vale una caché?

**En el análisis** (`cli::run_con_respaldo`), las cuatro a la vez:

| Regla | Función | Detalle |
|---|---|---|
| Mismo color | `project::same_color` | Igual `id` e igual `cube` (vacío = sin LUT). Sin `color` o `null` = `auto`. |
| Misma edición | `edicion::cache_vigente` | Joyas acepta sin `edition` o `"joyas"`; Bodas solo `"bodas"`. |
| Mismo detector | `detector::cache_vigente` | Mismo `detector`; o `respaldo: "propio"` cuando se pide el propio; o `en_lugar_de` igual al pedido. En Bodas no se mira. |
| Archivo igual | `analyse::cortado_vigente` | Solo con `cortado`: mismo tamaño y fecha que al analizar. |

- **Una caché ilegible** (cortada por un análisis matado antes de que la escritura fuera atómica) cuenta como clip sin analizar: aviso y se rehace.
- **`--rescore`** usa todas las cachés legibles de esta edición, sin mirar color ni detector (`project::read_entries_lenient`).
- **La ventana** carga las de su edición (`motor::cargar_cache`). Si dos archivos son del mismo clip, gana el más reciente.

---

## 2. Proxies de vista previa

**Código:** `sampler::PROXY_ARGS`, `sampler::proxy_vigente`, `sampler::sample_with`, `analyse::prepare_clip`.

| Dato | Valor |
|---|---|
| Ruta | `cache\proxies\<nombre de la caché>.mp4`. Usa el nombre con hash: dos clips iguales de carpetas distintas no chocan. |
| Video | H.264 (libx264 `veryfast`, CRF 20), `yuv420p`, GOP de 8 sin B, `+faststart`, **sin audio**. |
| Tamaño | El de análisis (lado largo 960), todos los cuadros, ya girado y **sin matriz de rotación**. |
| Peso | ~7 MB por minuto. |
| Mientras se escribe | `<nombre>.part.mp4`. Se renombra al final solo si ffmpeg terminó bien y el archivo no estaba cortado. Si no, se borra. |

**Quién los escribe:**
- El análisis, en la misma pasada que muestrea, si no hay uno al día y no se pasó `--no-proxy`.
- La ventana (`TrabajoProxies`), para clips analizados que no tienen ninguno. Mismos `PROXY_ARGS`, escalador `area`, prioridad baja.

### 2.1 La firma (`proxy_vigente`)

Un proxy "al día" se reconoce por la firma que x264 escribe en el primer cuadro:
1. Se recorren las cajas de primer nivel del MP4 (tamaños de 32 o 64 bits, como mucho 64 cajas) hasta `mdat`.
2. Se leen 64 KB desde el principio de `mdat` y se busca `x264 - core`. En los 2000 bytes siguientes deben estar `crf=20.0` **y** `deblock=1`.
3. Si el archivo no tiene esa estructura o no aparece, se repite la búsqueda en los primeros 512 KB del archivo.

**Por qué así:**
- Con `+faststart` el `moov` va delante y crece ~4.5 bytes por cuadro. En un proxy de más de ~32 min a 59.94 la firma ya no cabía en los primeros 512 KB y el proxy se reescribía en cada reanálisis.
- Se comparan **bytes**, no texto. Una versión anterior cortaba un texto "con pérdidas" a mitad de un carácter, entraba en pánico y el clip salía "fallo interno al abrir el clip".

**Proxies antiguos** (`ultrafast`, CRF 28, `deblock=0`, con bloques): no pasan la firma y **se rehacen la próxima vez que se analice ese clip**. Un clip con caché válida conserva su proxy viejo hasta entonces.

**Para qué sirve no repetirlo:** sin proxy que escribir, el análisis puede decodificar solo los cuadros I/P (`-skip_frame noref`, ver [analisis.md](analisis.md#55-decodificar-solo-ip--skip_frame-noref)).

Prueba: `sampler::tests::proxy_vigente_por_su_firma`.

---

## 3. CSV de tramos (`selects_*.csv`, `CAMCUTS_*.csv`)

**Código:** `export::write_csv`. Mismas columnas y formato que Python.

| Columna (joyas) | Contenido | Formato |
|---|---|---|
| `clip` | Nombre del archivo | texto |
| `inicio_s`, `fin_s`, `duracion_s` | Tramo en segundos del clip, con asas | 2 decimales |
| `score_medio`, `score_pico` | Media del núcleo y mejor muestra | 3 decimales |
| `pico_s` | Segundo de la mejor muestra | 2 decimales |
| `joya` | Tipo en el pico, p. ej. `dije con cadena` | texto |
| `joyas_en_corte` | Joyas vistas de forma estable, separadas por `, ` | texto |
| `tipo` | `producto`, `modelo` o `hablando` | texto |
| `motivo` | `Segment::describe` | texto |

**En Bodas:** `clip, inicio_s, fin_s, duracion_s, score_medio, score_pico, pico_s, momento, sujeto, sujetos_en_corte, plano, motivo`.

**Formato:**
- Separador `,`, líneas `\r\n`, UTF-8 sin BOM.
- Comillas solo si el campo tiene `,`, `"` o saltos de línea (como el módulo `csv` de Python).
- Los redondeos imitan a Python (`pyfmt::fixed`: valor binario exacto, empates al par).

Ejemplo:

```text
clip,inicio_s,fin_s,duracion_s,score_medio,score_pico,pico_s,joya,joyas_en_corte,tipo,motivo
C0001.MP4,12.25,18.50,6.25,0.712,0.884,15.00,anillo,"anillo, pulsera",modelo,"[modelo] joyas: anillo, pulsera · joya nítida, quieta, más nítida que el fondo"
```

`motivo` = `[plano] joyas: … · ` + nitidez (`joya nítida` / `algo blanda` / `blanda`) + quietud (`quieta` / `algo de movimiento` / `movida`) + `más nítida que el fondo` (si aplica) + `joya pequeña en cuadro` (si aplica) + avisos con su cuenta (`joya_cortada x3`).

---

## 4. Línea de tiempo OTIO (`*.otio`)

**Código:** `export::build_otio`, `export::write_otio`, `pyfmt::dumps_otio`.

**Formato:** JSON con sangría de 4, saltos `\r\n`, caracteres no ASCII sin escapar y las claves en el orden de la librería `opentimelineio`. Así Resolve lo importa igual que el de Python y los archivos se pueden comparar.

```text
Timeline.1   name, global_start_time = 0
└─ tracks: Stack.1
   ├─ markers: [Marker.2, …]                uno por tramo
   └─ children:
      ├─ Track.1 "V1 · resto del clip"  (Video)   Clip.2 y Gap.1
      ├─ Track.1 "V2 · aprobados"       (Video)
      ├─ Track.1 "A1"                   (Audio)   mismos elementos que la primera de video
      └─ Track.1 "A2"                   (Audio)   mismos elementos que la segunda de video
```

**Pistas según el modo:**

| Modo | Pistas de video |
|---|---|
| `aprobados` | `V1 · aprobados` |
| `v1v2` | `V1 · resto del clip` + `V2 · aprobados` |
| `marcadores` | `V1 · clips completos` |

**Cada `Clip.2`:**

| Campo | Valor |
|---|---|
| `name` | Nombre del archivo. |
| `media_references.DEFAULT_MEDIA.target_url` | **Ruta del medio original** (el RAW, aunque se analizara su proxy). |
| `available_range` | Desde el timecode de inicio, la duración del clip. |
| `source_range` | Timecode de inicio + inicio del trozo, y su duración. |
| `metadata.Resolve_OTIO` | `{"Link Group ID": n}`: Resolve enlaza el video y el audio que lo comparten. |
| `metadata.camcuts` | Solo en tramos aprobados: `{"jewels", "label", "mean_score", "peak_score"}`, claves ordenadas. En Bodas: `label, mean_score, moment, moments, peak_score, shot, subjects`. |

**Cada `Marker.2`** (en la regla de la línea de tiempo):
- `name`: `<joyas presentes, o tipo, o "joya"> · NN%`. En Bodas, el momento.
- `color`: `GREEN` si la media ≥ 0.8, `YELLOW` ≥ 0.65, `RED` el resto.
- `marked_range`: posición y duración del tramo en la línea de tiempo.
- `comment`: el mismo `motivo` del CSV.

**Tiempos:**
- **Todo a una sola frecuencia**, la de la línea de tiempo: Resolve los lee así diga lo que diga el archivo. CLI: `--timeline-fps` o la más común de los clips. Ventana: la cadencia elegida. Debe coincidir con el proyecto de Resolve.
- Los segundos se redondean al cuadro con empates al par.
- El tiempo de medio es el **timecode de cámara** (con drop-frame cuando toca). Detalle en [analisis.md](analisis.md#103-timecode-y-drop-frame).

**Nombres:**

| Quién | Archivo | Nombre de la línea de tiempo |
|---|---|---|
| CLI, joyas | `selects_joyas.otio`, `selects_social.otio`, `selects_todo.otio` | `CAMCUTS joyas` |
| CLI, Bodas | `selects_bodas.otio`, `selects_bodas_social.otio`… | `CAMCUTS Bodas bodas` |
| Ventana, joyas | `CAMCUTS_<Equilibrado/Redes/Permisivo>_<modo>.otio` | `CAMCUTS · <criterio> · <modo>` |
| Ventana, Bodas | `CAMCUTS_Bodas_<criterio>_<modo>.otio` | `CAMCUTS Bodas · <criterio> · <modo>` |

El CSV se llama igual con `.csv` (`edicion::nombre_selects`, `edicion::prefijo_export`).

---

## 5. Proyecto (`*.camcuts`)

**Código:** `camcuts-gui/src/proyecto.rs` (`guardar`, `abrir`, `app_ajena`) y `App::estado_json` / `App::aplicar_estado`. Formato `camcuts-project`, versión 1, el de `project.py` de Python.

**Formato:** JSON con sangría de 1 y sin escapar acentos (`json.dumps(indent=1, ensure_ascii=False)`). Al abrir se acepta un BOM al principio.

**Dónde:** por defecto `<carpeta de resultados>\<nombre de la carpeta del material>.camcuts` (`proyecto::ruta_por_defecto`).

| Clave | Tipo | Qué es |
|---|---|---|
| `format` | `"camcuts-project"` | Obligatoria: sin ella, "no es un proyecto de CAMCUTS". |
| `version` | 1 | |
| `app` | `"CAMCUTS"` / `"CAMCUTS Bodas"` | Edición del proyecto (`Edicion::de_app`; sin campo = joyas). |
| `saved` | `"AAAA-MM-DD HH:MM:SS"` | Hora local. |
| `sources` | `[{"abs", "rel"}, …]` | Material. `rel` es relativa al proyecto, o `null` si está en otra unidad. |
| `results_dir` | `{"abs", "rel"}` | Carpeta de resultados, igual. |
| `out`, `out_auto` | texto, bool | Carpeta de resultados tal cual; si es automática, se recalcula del material al abrir. |
| `criterion` | texto | `joyas`, `social`, `todo` (o los de Bodas). |
| `thr`, `min`, `max`, `per`, `gap`, `smooth` | números | Umbral, tramo mín./máx., tramos por clip, tolerancia de hueco y suavizado. |
| `weights` | objeto | `sharp`, `size`, `exposure`, `still`, `ratio`, `aniso`. |
| `joyas` | lista | Filtro de joyas (en Bodas, la clave es `momentos`). |
| `tipos` | lista | Filtro de planos. |
| `clip_types` | objeto | Tipo fijado por clip (ver 7). |
| `colors` | objeto | Color por clip (ver 6). |
| `fps` | texto | Cadencia de la línea de tiempo para exportar (no las muestras por segundo). |
| `recursive`, `background`, `zoom` | bool | Opciones del material y del análisis. |
| `detector`, `detector_v` | `"propio"`/`"world"`, 2 | Solo joyas. **Sin `detector_v` ≥ 2 el detector guardado no cuenta** y se usa el propio: antes casi todos guardaban `world` sin elegirlo. |
| `export_mode` | texto | `aprobados`, `v1v2` o `marcadores`. Un proyecto antiguo con `all_clips: false` abre en `aprobados`. |
| `ui` | objeto | Clip y segundo actual, zoom del mapa, paneles, alto de la línea de tiempo, modo claro… |
| `analysis` | objeto | Resumen: `clips`, `analysed` y hasta 2000 rutas `pending`. |

**Rutas al abrir** (`proyecto::abrir`): la absoluta si existe; si no, la relativa al proyecto; si tampoco, la absoluta. Las rutas que no existen (unidad desconectada) se conservan.

**Ediciones:** un proyecto de la otra edición se abre **solo lectura**. El autoguardado nunca lo pisa (`proyecto::app_ajena`) y sus `clip_types` no se copian a esta carpeta.

**Python** ignora las claves que no conoce (`detector`, `detector_v`, los extras de `ui`).

---

## 6. `colores.json`

Conversión de color por clip. **Cambiarla obliga a reanalizar ese clip** (la detección y la nitidez se miden sobre el cuadro convertido).

```json
{
 "d:\\material\\c0001.mp4": {
  "id": "slog3_sgamut3cine"
 },
 "d:\\material\\c0002.mp4": {
  "id": "cube",
  "cube": "C:\\LUTs\\mi_lut.cube"
 }
}
```

- **Clave:** ruta del clip en minúsculas. La CLI pasa a minúsculas también al leer (`project::read_colors`).
- **`id`:** uno de `color::TRANSFORMS` (`none`, `auto`, `slog3_sgamut3cine`, `slog3_sgamut3`, `slog2_sgamut`, `hlg`, `vlog`, `clog3`, `applelog`, `cube`).
- **`cube`:** solo con `id: "cube"`.
- **Sin entrada = `auto`**. La ventana borra la entrada cuando se elige automático.
- La ventana lo escribe antes de cada análisis y se lo pasa a la CLI con `--colores`. La CLI, sin `--colores`, lee `<out>\colores.json`.
- Formato: sangría de 1, sin escapar acentos.

---

## 7. `clip_types.json`

Tipo de joya que el editor fijó para un clip ("Joya de este clip" en la ventana). También queda como ejemplo para entrenar.

```json
{
 "d:\\material\\c0001.mp4": "anillo"
}
```

- **Clave:** ruta en minúsculas. **Valor:** un tipo de joya. En Bodas, un momento (`votos`, `beso`…).
- Se aplica **al puntuar**, nunca al analizar: todas las joyas del clip toman ese tipo (`scoring::force_type`; en Bodas `force_moment`). La caché no cambia.
- Lo leen la CLI al final de cada ejecución y la ventana al cargar.
- La ventana lo escribe al cambiar un tipo y al abrir un proyecto con `clip_types` (los copia aquí para que la CLI los vea).
- Formato: sangría de 1, sin escapar acentos.

---

## 8. `clips_meta.json` y metadatos

### 8.1 `clips_meta.json`

Datos para la tabla de clips de la ventana, para no lanzar ffprobe cada vez (`datos::sondear_meta`, `datos::escribir_metas`).

```json
{"d:\\material\\c0001.mp4":{"codec":"H.264","container":"MP4","res":"3840×2160","fps":29.97002997002997,"duration":35.035}}
```

| Campo | Qué es |
|---|---|
| `codec` | `H.264`, `H.265`, `ProRes` o el nombre de ffprobe; `?` si no hay. |
| `container` | Extensión en mayúsculas. |
| `res` | `ancho×alto` mostrado (girado si el clip es vertical). |
| `fps` | `r_frame_rate` como número. |
| `duration` | Duración del contenedor. |
| `error` | `true` si ffprobe falló o no hay pista de video (y no hay más campos). |

- **Clave:** ruta en minúsculas. Todos los campos son opcionales.
- La ventana lo escribe compacto (`serde_json::to_string`), sin los espacios de `json.dumps`. Python lo lee igual.
- Si la carpeta no deja escribir, la ventana lo avisa una vez y guarda los datos solo en la sesión.

### 8.2 Metadatos de cámara que se leen (nunca se escriben)

| Fuente | Para qué | Código |
|---|---|---|
| ffprobe (`-show_streams -show_format`) | Tamaño, rotación, fps, duración, códec, timecode, rango y matriz de color | `sampler::info_from_json`, `color::ffprobe_color` |
| `<nombre>M01.XML` al lado del clip | Perfil (`CaptureGammaEquation`, `CaptureColorPrimaries`), `CodingEquations`, drop-frame (`LtcChange`) | `color::texto_xml_sony` |
| XML `NonRealTimeMeta` dentro del MP4 | Lo mismo si falta el `M01.XML` (últimos o primeros 256 KB) | `color::xml_incrustado` |
| Átomo `moov` | Marca de Apple Log | `color::detect_profile` |
| `.cube` | LUT del usuario | `color::read_cube` |

---

## 9. Otros archivos

| Archivo | Contenido |
|---|---|
| `clips.txt` | Una ruta por línea. La ventana lo escribe en cada análisis y lo pasa con `--lista`. La CLI quita BOM, espacios y comillas de cada línea. |
| `necesitan_proxy.txt` | Una ruta por línea: RAW sin proxy encontrado. |
| `errores.txt` | `ruta<TAB>motivo` por línea: clips que fallaron o están cortados en la última pasada que analizó algo. Se borra si esa pasada no tuvo ninguno; si todo salió de la caché, no se toca. |
| `models\calibration.json` | Objeto con nombres de campo de `scoring::Preset` (`lap_soft`, `score_threshold`, …). `weights` reemplaza todos los pesos. Se busca junto al exe o en `CAMCUTS_CALIBRATION`. |
| `%APPDATA%\CAMCUTS-rs\settings.json` | Preferencias de la ventana (último proyecto y estado). En Bodas, `%APPDATA%\CAMCUTS-Bodas\settings.json`. |

---

## 10. Compatibilidad con las cachés de Python

| Tema | Python → Rust | Rust → Python |
|---|---|---|
| Nombre del archivo | Mismo algoritmo (SHA-1 de la ruta resuelta en minúsculas). Las antiguas `<nombre>.json` se encuentran sin distinguir mayúsculas. | Igual. |
| JSON | Se leen con valores por defecto para lo que falte; lo desconocido se conserva. | Mismos campos y formato (`json.dumps`: separadores, `\uXXXX`, `repr(float)`). Python carga diccionarios e ignora las claves nuevas. |
| Detector | **Sin `detector` = YOLO-World.** Con el propio por defecto, **analizar vuelve a analizar los clips de Python**; con `--detector world` se reutilizan. La ventana los marca "Reanalizar · detector". `--rescore` los usa siempre. | **Python solo compara el color**: reutiliza una caché de Rust aunque sea del propio. |
| Edición | Sin `edition` = joyas: valen en joyas, no en Bodas. | Python no mira `edition`. Bodas usa otra carpeta de resultados, así que no se mezclan. |
| `start_tc` | Si falta y el clip existe, se sondea al cargar. | Siempre se escribe. |
| `color` | Si falta, `auto`. | Siempre se escribe. |
| Etiquetas y protagonista | Se recalculan al puntuar (`refine_jewels`, `stabilize_labels`): no dependen de quién escribió la caché. | Igual. |
| OTIO y CSV | Re-puntuar una carpeta de Python da el mismo OTIO y CSV (prueba `tests/rescore.rs`). | Mismo esquema y formato. |

**Lo que se comprueba:** `cargo test -p camcuts-core` (paridad con `fixtures/` y con carpetas de Python). Tolerancias en [fixtures/README.md](../fixtures/README.md).

---

## 11. Escritura atómica y archivos bloqueados

### 11.1 `project::write_atomic`

La usan **las cachés, el OTIO y el CSV**.

1. Escribe `<archivo>.tmp`.
2. Lo renombra encima del destino (en Windows, `rename` reemplaza).
3. Si el proceso muere a mitad (Detener usa `taskkill /F`), queda el archivo anterior entero, **nunca uno cortado**.
4. Si el destino está abierto por otro programa, `rename` falla: **reintenta 10 veces con 100 ms entre ellas** (~1 s).
5. Si sigue sin poder, **conserva el `.tmp`** con el trabajo nuevo y el error dice dónde quedó. Antes se borraba y se perdía el análisis.

Prueba: `project::tests::destino_bloqueado_no_pierde_el_resultado`.

**Ojo con los `.tmp`:** nadie los lee solos. `cache_files` solo mira `*.json`, así que un `C0001_….json.tmp` no cuenta como caché. Se puede renombrar a mano o volver a analizar.

### 11.2 Qué pasa si Excel tiene el CSV abierto

Excel abre el CSV sin compartir: no se puede reemplazar.

| Dónde | Qué pasa |
|---|---|
| CLI | Tras ~1 s de reintentos: `aviso: no se pudo escribir el CSV (¿abierto en Excel?): …`. El OTIO sí se escribe. **Termina con código 0** (antes, código 1 y la ventana decía "El análisis se detuvo" con todo analizado). El CSV nuevo queda en `selects_<criterio>.csv.tmp`; el viejo sigue intacto. |
| Ventana, Exportar | Muestra los tramos exportados y añade "El CSV no se pudo reemplazar (¿está abierto en Excel?)". Si el que falla es el OTIO, "No se pudo exportar". |
| Cachés | No les afecta: se escriben antes de exportar. |

**Solución:** cerrar el CSV en Excel y volver a exportar (o `--rescore`). El `.tmp` se sobrescribe la próxima vez.

Prueba: `cli::pruebas::csv_bloqueado_no_tumba_el_analisis`.

Si lo bloqueado es una **caché** (un antivirus, la app de Python leyéndola), ese clip falla con "no se pudo reemplazar … (el resultado nuevo queda en ….tmp)" y aparece en `errores.txt`.

### 11.3 Lo que no es atómico

| Archivo | Cómo se escribe |
|---|---|
| `.camcuts` | Temporal + `rename`, pero **sin reintentos**: si está bloqueado, falla el guardado. |
| Proxies | ffmpeg escribe `.part.mp4` y se renombra al final; si el renombrado falla, no se avisa. |
| `colores.json`, `clip_types.json`, `clips_meta.json`, `clips.txt`, `settings.json`, `necesitan_proxy.txt`, `errores.txt` | Escritura directa (`std::fs::write`). Un corte a mitad puede dejarlos incompletos; quien los lee trata un JSON roto como vacío. |

---

## Discrepancias encontradas

Revisado contra el código del 2026-11-06. En todos los casos **manda el código**.

1. **Firma del proxy.** INTERFACES.md dice que `sampler::proxy_vigente` busca la firma "en los primeros 512 KB". El código la busca primero al principio de `mdat` (64 KB) y después en los primeros 512 KB (sección 2.1).
2. **`project.rs` del núcleo frente al de la ventana.** INTERFACES.md dice que cada edición guarda su nombre de app en el proyecto (`edicion::nombre_app`). Lo hace la ventana (`camcuts-gui/src/proyecto.rs::guardar`). En cambio `camcuts_core::project::save` escribe siempre `"app": "CAMCUTS"` (`APP_NAME`). Ningún binario la llama hoy, igual que `project::load`, `write_dict_indented`, `write_meta` y `probe_meta`: **usarlas desde Bodas marcaría el proyecto como de joyas**.
3. **Escrituras sin las ayudas atómicas del núcleo.** El núcleo tiene `write_dict_indented` y `write_meta` (atómicas y con el formato exacto de Python), pero la ventana escribe `colores.json`, `clip_types.json` y `clips_meta.json` con `std::fs::write`. Además `clips_meta.json` sale compacto (`{"k":v}`), no como `json.dumps` (`{"k": v}`). El contenido es el mismo JSON y Python lo lee, pero no es idéntico byte a byte, como pide la regla de compatibilidad de INTERFACES.md.
4. **"Y al revés" con Python.** INTERFACES.md pide que una carpeta de Rust abra en Python. Abre, pero Python solo compara el color para reutilizar una caché: reutiliza cachés del detector propio sin saberlo, y Rust en cambio rehace por defecto las cachés de Python (sin clave `detector` = YOLO-World).
