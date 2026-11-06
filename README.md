# CAMCUTS

**Qué es.** CAMCUTS es una app de escritorio para Windows de Napoleone Joyas, escrita en Rust. Revisa el material de cámara y elige los tramos donde la joya se ve **nítida, quieta y bien expuesta**. Luego los exporta a DaVinci Resolve como línea de tiempo **OTIO** y como **CSV**.

**De dónde viene.** Es el port de la app de Python que vive en `C:\Users\Usuario\Tools\camcuts_next\camcuts`. Esa app sigue siendo la referencia: con el mismo material, Rust debe dar los **mismos selects**. Las carpetas de resultados y los proyectos `.camcuts` se abren en las dos.

## Las dos ediciones

| | Joyas | Bodas |
|---|---|---|
| Binarios | `camcuts.exe` (ventana) y `camcuts-cli.exe` (línea de comandos sin modelos) | `camcuts-bodas.exe` |
| Qué elige | La joya nítida | Caras nítidas, cámara estable y el momento de la boda |
| Detector | Propio (YOLO11 entrenado con el material de la marca) o YOLO-World | YOLO-World con 12 sujetos de boda |
| Diseño de la ventana | Marfil | Salvia |
| Resultados por defecto | `<material>\CAMCUTS` | `<material>\CAMCUTS Bodas` |
| Paquete | `dist\CAMCUTS\` | `dist\CAMCUTS Bodas\` |

- **Un solo código.** `camcuts-bodas.exe` es el mismo programa con la edición fijada al arrancar (`camcuts_core::edicion::fijar`, en `crates/camcuts-gui/src/main_bodas.rs`). Toda la separación entre ediciones vive en `crates/camcuts-core/src/edicion.rs`.
- **Cada edición guarda lo suyo:** cachés, nombres de exportación, preferencias. Una caché de la otra edición se vuelve a analizar.
- **La edición Bodas no puede llevar la palabra "Napoleone".** `scripts\armar_dist.ps1` la busca dentro de `camcuts-bodas.exe` y no empaqueta si la encuentra. Como el código es compartido, la regla vale para cualquier texto que acabe dentro del binario.

## Los tres crates

| Crate | Qué hace | Binarios |
|---|---|---|
| `crates/camcuts-core` | Muestrea con ffmpeg, mide nitidez, exposición y movimiento, puntúa, corta tramos y exporta OTIO y CSV. Lee y escribe la caché y el proyecto con el formato de Python. No carga modelos ni abre ventanas. También contiene la línea de comandos (`cli.rs`). | `camcuts-cli.exe` |
| `crates/camcuts-infer` | Los modelos con ONNX Runtime y DirectML: detector propio o YOLO-World, MobileSAM (silueta), yolo11n-pose, CLIP (segunda opinión) y Silero VAD (voz). | solo ejemplos (`examples/`) |
| `crates/camcuts-gui` | La ventana (eframe/egui sobre wgpu). El análisis corre en un proceso hijo: el mismo exe con `--analizar`. | `camcuts.exe`, `camcuts-bodas.exe` |

El contrato entre crates y el registro de decisiones está en [INTERFACES.md](INTERFACES.md). Esta guía no lo repite.

## Qué hay en el repositorio y qué no

| Carpeta | Qué hay | ¿En Git? |
|---|---|---|
| `crates/` | El código de los tres crates, sus pruebas y los datos pequeños de prueba (`tests/data/`) | Sí |
| `.cargo/` | `config.toml` y el sustituto de dlltool (`dlltool-lld.cmd` y `.ps1`) | Sí |
| `scripts/` | `configurar_compilacion.ps1`, `verificar_modelos.ps1`, `armar_dist.ps1` | Sí |
| `models/` | Los `.json` de clases y textos, `MANIFIESTO.sha256`, `README.md` y los scripts de exportación | Sí |
| `models/**/*.onnx` | Los modelos (unos 1.6 GB) | **No**: se copian aparte y se comprueban con el manifiesto |
| `runtime/README.md` | De dónde sacar las bibliotecas y qué versión son | Sí |
| `runtime/*.dll` | `onnxruntime.dll` y `DirectML.dll` | **No**: se copian aparte y se comprueban con el manifiesto |
| `fixtures/` | Salidas de referencia de Python (PNG, JSON, WAV) | Sí |
| `empaque/bodas/` | `README.md` y `CLAUDE.md` que viajan con el paquete Bodas | Sí |
| `design/` | `Marfil.html` y `snapshots/marfil_*.png` (las capturas de joyas que escriben las pruebas de la ventana) | Sí |
| `design/snapshots/bodas_*.png` | Las capturas de Bodas | **No**: muestran caras de personas reales |
| `training/scripts/`, `training/bench/`, `training/e2e/` | Scripts, cuadros de referencia y el crate de evaluación `camcuts-e2e` del detector propio | Sí |
| `training/` (el resto) | Datos, pesos y corridas de entrenamiento | No |
| `bench_*/`, `robustez/` | Bancos de velocidad y de regresión, con material de la empresa | No |
| `target/` | La compilación | No |
| `dist/` | Los paquetes armados | No |
| `docs/` | La documentación | Sí |

**Fuera del repositorio** también quedan:
- la app de Python de referencia y sus cachés (`C:\Users\Usuario\Tools\camcuts_next`);
- el material de cámara (`D:\`, solo lectura).

**El repositorio es público** (desde el 2026-11-06, para que TI lo revise). Por eso no lleva claves, correos ni direcciones, y tampoco las capturas de Bodas. `fixtures\*.png` y `design\snapshots\marfil_*.png` son cuadros del material de la empresa, pero solo muestran joyas y manos. No subas imágenes o audio con personas reconocibles.

## Inicio rápido

Todo con Windows PowerShell 5.1, desde la raíz del repositorio. El detalle está en [docs/compilar_y_probar.md](docs/compilar_y_probar.md).

**1. Requisitos.**
- Toolchain de Rust `stable-x86_64-pc-windows-gnu` (probada con rustc 1.99.1). No hace falta MinGW.
- ffmpeg y ffprobe en el PATH: `winget install Gyan.FFmpeg`.
- Una GPU con DirectX 12.

**2. Después de clonar**, apunta el sustituto de dlltool a la carpeta del clon (una sola vez):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\configurar_compilacion.ps1
```

**3. Modelos y runtime.** Cópialos desde el workspace original o desde la carpeta compartida, y compruébalos:

```powershell
$origen = 'C:\Users\Usuario\Tools\camcuts_rs'      # o la carpeta compartida
New-Item -ItemType Directory -Force runtime, models\bodas | Out-Null
Copy-Item "$origen\runtime\*.dll" runtime\
Copy-Item "$origen\models\*.onnx" models\
Copy-Item "$origen\models\bodas\*.onnx" models\bodas\
powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1   # debe decir: 19 correctos, 0 con problemas
```

Ojo: los paquetes de `dist\` solo traen 12 de los 19 archivos del manifiesto. Faltan los FP32 que usan las pruebas de paridad. Ver [docs/compilar_y_probar.md](docs/compilar_y_probar.md#modelos-y-runtime).

**4. Compilar.** Siempre en release (en debug, `target\` ocupa unos 15 GB):

```powershell
cargo build --release -p camcuts-gui -p camcuts-core
```

Deja `camcuts.exe`, `camcuts-bodas.exe` y `camcuts-cli.exe` en `target\release\`.

**5. Probar** (no abren ventanas y casi no usan la GPU):

```powershell
cargo test --release -p camcuts-core --lib --tests
cargo test --release -p camcuts-infer --lib
cargo test --release -p camcuts-gui --lib
```

**6. Empaquetar** las dos ediciones en `dist\`:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Prueba   # solo dice qué haría
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1
```

**Nunca** ejecutes `camcuts.exe` sin `--analizar` en un equipo ajeno o desde un agente: abre la ventana. El uso por línea de comandos está en [docs/operacion.md](docs/operacion.md).

## Git

El repositorio se inició el 2026-11-06: rama `main`, primer commit con la etiqueta `base-2026-11-06`. Está publicado en GitHub, público: <https://github.com/napoleonejoyeria-rgb/Video-Analizer>. Cómo comprobar el `.gitignore`, cómo validar un clon desde cero y la opción de Git LFS para los modelos están en [docs/compilar_y_probar.md](docs/compilar_y_probar.md#lista-de-comprobación-de-integridad-para-ti).

## Documentación

| Documento | De qué trata |
|---|---|
| [docs/analisis.md](docs/analisis.md) | Cómo se analiza un clip: muestreo, medidas, puntuación y tramos |
| [docs/formatos.md](docs/formatos.md) | Caché por clip, proyecto `.camcuts`, `colores.json`, OTIO y CSV |
| [docs/modelos.md](docs/modelos.md) | Los modelos ONNX y su preparación |
| [docs/entrenamiento.md](docs/entrenamiento.md) | El detector propio: datos, entrenamiento y evaluación |
| [docs/ventana.md](docs/ventana.md) | La ventana: paneles, atajos y exportación |
| [docs/bodas.md](docs/bodas.md) | La edición Bodas |
| [docs/compilar_y_probar.md](docs/compilar_y_probar.md) | Requisitos, toolchain, compilación, pruebas, regresión y comprobación para TI |
| [docs/operacion.md](docs/operacion.md) | Línea de comandos, variables `CAMCUTS_*`, GPU, paquetes y solución de problemas |
| [INTERFACES.md](INTERFACES.md) | Contrato entre crates y registro de decisiones con sus medidas |
| [GPU_TURNOS.md](GPU_TURNOS.md) | Turnos de GPU para quien trabaja en este PC |
| [models/README.md](models/README.md) | Cada modelo: entradas, salidas y pre y posprocesado exactos |
| [fixtures/README.md](fixtures/README.md) | Referencias de Python y tolerancias de paridad |
| [empaque/bodas/](empaque/bodas/) | Guía de usuario y guía para Claude del paquete Bodas |
| [CLAUDE.md](CLAUDE.md) | Reglas para agentes de IA que trabajen en el repositorio |

## Discrepancias encontradas

Revisado contra el código el 2026-11-06.

**INTERFACES.md frente al código:**
- **Detector por defecto.** La sección "Detector propio (añadido)" dice que YOLO-World es el predeterminado y que la CLI usa `world` si no hay `--detector` ni `CAMCUTS_DETECTOR`. El código usa el propio (`DetectorKind`, con `#[default] Propio`, y `DetectorKind::resolver`). La sección "Own detector by default" ya lo corrige; el texto anterior quedó viejo.
- **Clips en paralelo.** La misma sección dice "modo ligero = 1, `--background` = 1". Eso solo vale para YOLO-World. Con el propio, `hardware::parallel_clips` no limita en modo ligero y da 2 en segundo plano (`BACKGROUND_WORKERS_PROPIO`).
- **Modo ligero.** "Poca VRAM con DaVinci abierto" dice "YOLO-World m, un clip". En joyas, si el modelo propio está instalado, analiza el propio (`InferConfig::puede_usar_propio`). La sección siguiente de INTERFACES ya lo dice.
- **VRAM por clip.** INTERFACES mide unos 3.3 GB por clip de YOLO-World x, pero `hardware::WORLD_GB_PER_WORKER` sigue en 2.6 GB.
- **Fixtures.** INTERFACES dice que `fixtures/` trae cachés por clip. No hay ninguna. `tests/rescore.rs` las lee de `camcuts_next`, fuera del repositorio.
- **Identificadores en inglés.** Es una regla de INTERFACES, pero el código reciente usa nombres en español (`cli::analizar_lote`, `camcuts_infer::Compartido`, casi toda `camcuts-gui`).
- **Rutas que no van a Git.** INTERFACES cita `training\runs\r6`, `training\e2e_runs\r6cmp` y `bench_perf\modelos_r2`. Ninguna va al repositorio. También cita `dist\runtime_cuda`, que hoy no existe.

**Otros comentarios desactualizados:**
- El comentario de módulo de `crates/camcuts-core/src/lib.rs` dice "YOLO-World por defecto".
- `crates/camcuts-infer/Cargo.toml` dice que basta con poner el `dlltool.exe` de la toolchain en el PATH. Ese dlltool necesita el ensamblador `as`, que no está; por eso existe `.cargo/dlltool-lld.cmd`.
- `models/README.md` dice que el modelo mediano se usa con menos de 3 GB libres; el código usa 4.5 GB (`hardware::LOW_VRAM_GB`). También dice que SAM guarda el embedding del último cuadro; guarda 4 (`sam::CACHE_CUADROS`).
