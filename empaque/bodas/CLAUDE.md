# CAMCUTS Bodas — guía para Claude

**Qué es.** `camcuts-bodas.exe` es la edición Bodas de CAMCUTS, una app de escritorio para Windows escrita en Rust (eframe/egui sobre wgpu, ONNX Runtime con DirectML).

**Qué hace:**
- Analiza el material de cámara de una boda.
- Elige los tramos donde el sujeto principal (sobre todo las caras) está nítido, bien expuesto y con la cámara estable.
- Etiqueta cada muestra con su **momento** de la boda.
- Exporta una línea de tiempo OTIO para DaVinci Resolve:
  - V1: el resto de cada clip.
  - V2: los tramos aprobados.
  - Audio enlazado.
  - Un marcador por tramo con el nombre de su momento.

## Dónde está el código

El código fuente vive en `C:\Users\Usuario\Tools\camcuts_rs` (o en un clon de su repositorio), un workspace de Cargo. Esta carpeta es solo la app empaquetada.

**Las dos ediciones comparten el código.** `camcuts.exe` es la de joyería y `camcuts-bodas.exe` la de bodas. Esta última es el mismo programa con la edición fijada al arrancar:

```rust
camcuts_core::edicion::fijar(Edicion::Bodas);   // crates/camcuts-gui/src/main_bodas.rs
```

Si no se fija, la edición sale de la variable de entorno `CAMCUTS_EDICION=bodas`, y si tampoco está, es joyas.

**Joyas no debe cambiar.** Las funciones compartidas de `scoring`, `analyse`, `export` y `cli` empiezan con una comprobación de una línea y pasan el trabajo a `camcuts_core::bodas` cuando la edición es Bodas. Las pruebas de paridad con Python de la edición de joyas (`cargo test -p camcuts-core`) deben seguir pasando: no cambies el comportamiento de joyas.

**Lo que es solo de joyas.** El código compartido cambió en noviembre de 2026 para la edición de joyas. Bodas queda fuera de todo esto:
- **Detector propio.** Es el predeterminado de joyas. Bodas usa siempre su YOLO-World de boda: `Args::detector_kind` devuelve `World` y la ventana de Bodas no tiene selector de detector.
- **Respaldo automático con el clásico.** No existe en Bodas: `detector::necesita_respaldo` devuelve `false`.
- **Detector compartido entre hilos** (`camcuts_infer::fabrica` → `Compartido`). Solo con el propio. En Bodas, cada hilo de análisis tiene su detector.
- **Propio en lugar del mediano en modo ligero** (`InferConfig::puede_usar_propio`). Solo joyas. Bodas pasa a `yolo_world_bodas_m`.

**Clips en paralelo** (`hardware::parallel_clips`): Bodas cuenta como YOLO-World. Como mucho 2 clips a la vez (`WORLD_MAX_WORKERS`); 1 en segundo plano (`BACKGROUND_WORKERS`, el predeterminado de la ventana) o en modo ligero. Y nunca más de los que caben en la VRAM libre (`hardware::vram_workers`, unos 2.6 GB por clip).

### Archivos de la edición Bodas

| Archivo | Qué hace |
|---|---|
| `crates/camcuts-core/src/edicion.rs` | La edición del proceso |
| `crates/camcuts-core/src/bodas.rs` | Sujetos, medida por muestra, puntuación, tramos, momentos, criterios y pruebas |
| `crates/camcuts-infer/src/moments.rs` | Momento de un cuadro (CLIP zero-shot sobre el cuadro entero rellenado a cuadrado) |
| `crates/camcuts-infer/src/lib.rs` | En Bodas carga `yolo_world_bodas_{x,m}.onnx` y `clip_moments.json`, e implementa `Detector::moments` |
| `crates/camcuts-gui/src/bodas_tema.rs` | Tema "Salvia": verde clarito, modo claro y oscuro, color de cada momento |
| `crates/camcuts-gui/src/bodas_ui.rs` | La ventana de Bodas: barra superior, clips, color, visor con Foco, criterio y línea de tiempo de momentos |
| `crates/camcuts-gui/src/main_bodas.rs` | El binario |
| `models/export_bodas.py` | Exporta los modelos de Bodas |
| `models/check_bodas.py` | Referencia de Python para comprobar los modelos |
| `crates/camcuts-infer/examples/bodas_check.rs` | Lo mismo en Rust |

**El estado de la ventana es compartido** (`crates/camcuts-gui/src/app.rs`): material, análisis, proyecto `.camcuts`, color por clip, visor, exportar. Solo cambia el dibujo: `App::dibujar` llama a `dibujar_bodas` en esta edición.

## Cómo funciona

**1. Sujetos.** YOLO-World detecta 12 clases, todas con prompts en inglés:

| Prompt | Etiqueta |
|---|---|
| face | cara |
| person | persona |
| bride in a white wedding dress | novia |
| groom in a suit | novio |
| wedding dress | vestido |
| wedding rings | anillos |
| bridal bouquet of flowers | ramo |
| wedding cake | pastel |
| champagne glass | copas |
| microphone | micrófono |
| high heel shoes | zapatos |
| sparkler | bengalas |

**Caras desde la pose.** El prompt "face" casi no encuentra caras en material real. Por eso `bodas::pose_faces` crea cajas de cara con la nariz, los ojos y las orejas de la pose (yolo11n-pose).

**Agrupación.** Solo se juntan cajas de lo mismo (IoU ≥ 0.5, por ejemplo persona + novia sobre un cuerpo); una cara dentro de un cuerpo es un sujeto aparte.

**2. Medida** (`bodas::measure`): igual que la de joyas (nitidez p99 del Laplaciano sobre la silueta de MobileSAM o la caja). Además guarda en el mapa `extra` de la caché:

| Dónde | Campos |
|---|---|
| Muestra | `frame_lap_p99`, `jewel_luma`, `jewel_clip`, `faced`, `face_frac`, `voice` |
| Muestra, momento | `moment_raw`, `moment_conf_raw`, `moment_top`, `moment`, `moment_conf` |
| Muestra, al cargar | `shake` |
| Cada sujeto | `luma`, `clip`, `faced` |

En la caché "jewel" significa **sujeto principal**. No se renombran campos: la estructura `Sample` es la de joyas.

**3. Sujeto principal** (`bodas::hero_priority`):

```
(0.15 + nitidez) × (0.25 + centro) × (0.5 + conf) × (0.35 + 0.65 × tamaño) × peso_tipo
```

- `peso_tipo`: cara 1, objetos 0.8, novia/novio 0.75, vestido 0.7, persona 0.6.
- Una persona de espaldas vale ×0.8.

**4. Momento:**
- **Clasificación:** CLIP clasifica el cuadro entero (rellenado a cuadrado) contra 16 momentos. Cada momento es la media de 3-6 frases en inglés. Solo se hace en cuadros nuevos; los quietos heredan el momento.
- **Estabilización** (`bodas::stabilize_moments`): mayoría ponderada en unos 3 s. La voz (Silero VAD) multiplica "votos" y "discursos" ×1.6, y ×0.7 en silencio. Si la probabilidad media del ganador es menor que 0.18, el momento queda "otro".
- **Corrección manual:** el editor puede fijar el momento de un clip (`clip_types.json`, en la ventana "Momento de este clip").

**5. Puntuación** (`bodas::score_sample`):
- La nitidez es la puerta:
  - `lap_soft` 35 → `lap_sharp` 90 (las caras tienen detalle más suave que las joyas).
  - El resto la modula hasta el 60 %.
- Pesos por defecto:

  | Criterio | Peso |
  |---|---|
  | exposición del sujeto | 0.15 |
  | cámara estable | 0.10 |
  | tamaño | 0.10 |
  | barrido | 0.05 |
  | quietud | 0 |
  | ratio | 0 |
- Ajustes y casos especiales:
  - De espaldas: ×0.75.
  - Sujeto cortado por el borde (no los cuerpos): ×0.8.
  - En *lugar* y *detalles* el cuadro entero también puntúa como `plano_de_ambiente` (×0.85).
- **Cámara estable:** cuánto se aparta el desplazamiento de cámara de su mediana de 5 muestras, por cada 1/4 s. `SHAKE_REF` = 15 px.

**6. Tramos** (`bodas::build_segments`): como en joyas, pero cortan donde cambia el momento y cada uno lleva `moment` y `moments`. El filtro de tipos (`jewel_types`) filtra por momento. Planos: hablando, primer_plano, plano_medio, plano_general, detalle.

**7. Criterios:**

| Criterio | Umbral | Tramos |
|---|---|---|
| `bodas` | 0.5 | hasta 60 s, todos |
| `social` (Highlight) | 0.6 | hasta 4 s, 3 por clip |
| `todo` | 0.4 | — |

## Modelos

Están en `models\` de esta carpeta (en el workspace, en `models\` y `models\bodas\`):

| Modelo | Uso |
|---|---|
| `yolo_world_bodas_x.onnx` | Detector (FP32) |
| `yolo_world_bodas_m.onnx` | Detector en modo ligero: menos de 4.5 GB de VRAM libre (`hardware::LOW_VRAM_GB`), o si el x no se puede abrir o va lento. Un clip a la vez |
| `clip_moments.json` | Textos de los momentos |
| `clip_visual_fp16.onnx` + `clip_text.json` | CLIP |
| `mobile_sam_*_fp16.onnx` | Siluetas |
| `yolo11n_pose_fp16.onnx` | Pose |
| `silero_vad.onnx` | Voz |

`runtime\` trae `onnxruntime.dll` y `DirectML.dll`. Se comprobaron contra Python en CPU: mismas cajas y confianzas a 3 decimales, y momentos a ±0.001.

## Compilar, probar y empaquetar

**Antes de empezar:**
- Toolchain `stable-x86_64-pc-windows-gnu`.
- Tras clonar el repositorio, ejecuta `scripts\configurar_compilacion.ps1`: apunta `.cargo\config.toml` (el sustituto de dlltool) a la carpeta del clon.
- Los modelos `.onnx` y las DLL de `runtime\` no van en Git. Cópialos aparte y compruébalos con `scripts\verificar_modelos.ps1`.
- Lee `GPU_TURNOS.md` antes de usar la GPU.
- Nunca escribas bajo `D:\`.
- Nunca abras ventanas en la pantalla del usuario.

**Compilar:**

```powershell
cd C:\Users\Usuario\Tools\camcuts_rs
cargo build --release -p camcuts-gui --bin camcuts-bodas
```

**Pruebas:**

```powershell
cargo test -p camcuts-core                 # paridad de joyas + pruebas de bodas (bodas::tests)
cargo test -p camcuts-gui --lib            # incluye ancla de selección, material sin conexión, visor
$env:CAMCUTS_BODAS_BENCH="C:\Users\Usuario\Tools\camcuts_rs\bench_bodas"
cargo test -p camcuts-gui --test capturas_bodas -- --nocapture   # capturas sin pantalla: design\snapshots\bodas_*.png
```

Las carpetas `bench_*` son locales: no van en el repositorio. Sin `CAMCUTS_BODAS_BENCH`, la prueba de capturas se salta.

**Analizar sin ventana:**

```powershell
camcuts-bodas.exe --analizar --lista clips.txt --out <resultados> --no-sheet [--momento votos,beso] [--preset social]
```

Para hacerlo sin GPU, define `CAMCUTS_DISPOSITIVO=cpu` y `CAMCUTS_DECODE=cpu`. En CPU va a unos 2 s por muestra nueva, y eso es lo normal: con `CAMCUTS_DISPOSITIVO=cpu` no se mira la VRAM, no hay modo ligero y la guarda de lentitud no salta (`hardware::en_cpu`).

**Empaquetar:**

```powershell
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Edicion bodas
```

Copia `camcuts-bodas.exe`, los modelos y el runtime de la tabla de arriba, este `CLAUDE.md` y el `README.md` (de `empaque\bodas\`) a `dist\CAMCUTS Bodas\`. Solo copia lo que cambió y comprueba el SHA-256 de cada archivo. Se detiene si `camcuts-bodas.exe` lleva el nombre de la marca de joyería.

## Probado

Material de prueba con personas, de octubre de 2026 (unidad `D:`, solo lectura). No hay material de bodas: se usaron una pareja, una persona hablando a cámara y una modelo, y todos los clips son verticales.

Se analizaron 9 clips en CPU con `CAMCUTS_DISPOSITIVO=cpu` (la GPU estaba ocupada por otros trabajos). Salen 23 tramos aprobados, con 1:54 aprobados de 2:48 de material.

| Clip | Qué se ve | Momentos |
|---|---|---|
| IMG_4362 | persona hablando a cámara | discursos 88 %, votos 12 % (la voz empuja "discursos") |
| IMG_4357 | persona hablando en una mesa | banquete 52 %, votos 48 % |
| C8909 | modelo, primer plano de cara | preparativos 100 % |
| IMG_4141 | chica en una joyería | preparativos 100 % (la cara de frente es el sujeto principal; la persona de espaldas no) |
| IMG_4142 | — | detalles 100 % |
| C9057, C9058, C9061, C9062 | manos con pulseras, anillos y cadenas | anillos y detalles (algo de fiesta en C9061) |

**Resultados y capturas:**
- Los resultados están en `camcuts_rs\bench_bodas`, que es la carpeta de la prueba de capturas.
- Las capturas sin pantalla están en `camcuts_rs\design\snapshots\bodas_*.png`.

**Validación del detector:** el detector de bodas exportado coincide con Python en CPU (`models/check_bodas.py` frente a `bodas_check`).

**En la GPU (DirectML, `bench_bodas_gpu`):** los mismos 9 clips tardan 52 s, frente a unos 29 min en CPU. Comparado con la ejecución en CPU:
- se aprueban 116,7 s frente a 114,0 s (+2,4 %), con 23 tramos en ambos;
- el momento coincide en el 95,7 % de las muestras;
- el sujeto principal coincide en el 95,1 %.

**Ajuste de nitidez en GPU:** la ruta de GPU (NVDEC + lanczos) corrige la nitidez con `GPU_LAP_SCALE` = 1/1.103, medido en joyería. En este material el lanczos sale solo ~5 % más nítido, así que esta edición lo ajusta ×1.05 (`bodas::GPU_AJUSTE`, en `bodas::measure`). Sin el ajuste la GPU aprobaba un 12 % menos.

**Lo que queda de diferencia** es la nitidez del cuadro entero en clips LOG (C9062, IMG_4142). Los dos decodificadores la dan algo distinta y solo cuenta en planos sin sujeto.

## Pendiente e ideas

- **Calibrar con bodas reales:**
  - `lap_soft` / `lap_sharp` de las caras
  - `SHAKE_REF`
  - `MOMENT_MIN`
  - las frases de cada momento: `models/export_bodas.py`, y luego volver a generar `clip_moments.json`
- **Detección de sujetos:** "novia" y "novio" salen de cualquier persona con vestido o traje: son pistas, no identidades.
- **Momentos confundibles:** retratos y preparativos, cóctel y banquete, anillos y detalles.
- **Ideas:**
  - ojos cerrados (puntos faciales)
  - emoción y reacciones
  - móviles de invitados delante del sujeto
  - frases de votos y brindis (transcripción)
  - "first look"
  - armar un highlight con orden narrativo
  - cortar al ritmo de la música
