# CAMCUTS — guía para agentes de IA

**Qué es.** CAMCUTS elige los tramos de video donde la joya está nítida y los exporta a DaVinci Resolve (OTIO y CSV). Es un workspace de Cargo con tres crates: `camcuts-core` (análisis y CLI), `camcuts-infer` (ONNX Runtime con DirectML) y `camcuts-gui` (la ventana y los binarios). Tiene dos ediciones: joyas (`camcuts.exe`, `camcuts-cli.exe`) y Bodas (`camcuts-bodas.exe`). Empieza por [README.md](README.md).

**Antes de tocar nada, lee** [INTERFACES.md](INTERFACES.md) (contrato entre crates y registro de decisiones) y [GPU_TURNOS.md](GPU_TURNOS.md).

## Reglas

1. **Español en todo.** Respuestas, documentación, mensajes de la app y comentarios del código. Comenta cada módulo, cada elemento público y cada bloque que no sea obvio. Para los identificadores, sigue el estilo del módulo que tocas.
2. **Nunca escribas bajo `D:\`.** Es la unidad del material: solo lectura. Las salidas van bajo `C:\Users\Usuario\Tools\camcuts_rs` (por ejemplo `bench_perf\…`).
   - Pasa siempre `--out`: la ventana guarda por defecto en `<material>\CAMCUTS`, que está en `D:\`.
   - No toques `C:\Users\Usuario\Tools\camcuts` (la app de Python instalada) ni `C:\Users\Usuario\Tools\camcuts_next` (la referencia: solo se lee).
3. **Nunca abras ventanas en la pantalla del usuario.**
   - `camcuts.exe` y `camcuts-bodas.exe` **sin `--analizar` abren la ventana**. Para analizar, `--analizar` va primero.
   - La ventana se prueba sin pantalla: `cargo test -p camcuts-gui --lib` y las capturas de egui_kittest (`tests/capturas*.rs`).
4. **Protocolo de GPU** ([GPU_TURNOS.md](GPU_TURNOS.md)). Antes de cualquier cosa que use la GPU (analizar, `python_fixtures`, capturas, `real_clip`, `examples/bench.rs`, entrenar):
   - mira `.gpu_lock`: si tiene menos de 2 h, la GPU es de otro; trabaja en la CPU o pide turno con `.gpu_wait`;
   - toma el turno creando `.gpu_lock` con tu nombre y la hora, y bórralo al terminar;
   - con DaVinci (`Resolve.exe`) abierto, uso moderado (unos 60 %); sin él, a fondo pero sin largos ratos al 99 %;
   - como mucho 2 clips a la vez con YOLO-World x, y mira los eventos `nvlddmkm` después;
   - nunca mates procesos que no lanzaste.
5. **Nada de "Napoleone" en Bodas.** `camcuts-bodas.exe` se compila del mismo código que `camcuts.exe`, así que ningún texto que acabe dentro del binario puede llevar la palabra: literales, recursos incluidos, nombres de archivos. Tampoco los archivos de `empaque\bodas\`. `scripts\armar_dist.ps1` lo comprueba en el exe y no empaqueta si la encuentra.
6. **Paridad con Python.** La edición de joyas debe dar los mismos selects que `C:\Users\Usuario\Tools\camcuts_next\camcuts`.
   - Las pruebas de paridad (`fixtures`, `python_numbers`, `rescore`, `python_fixtures`) siempre tienen que pasar.
   - Bodas entra en el código compartido con una comprobación de una línea (`edicion::es_bodas()`) y le pasa el trabajo a `camcuts_core::bodas`. No cambies el comportamiento de joyas para arreglar Bodas.
   - Un cambio buscado en los selects se mide (regresión) y se apunta en INTERFACES.md con sus cifras.
7. **Release siempre.** `cargo build --release` y `cargo test --release`. En debug, `target\` llega a unos 15 GB y C: tiene unos 20 GB libres. Comparte `target\` y limpia tus archivos temporales.
8. **Toolchain GNU** (`stable-x86_64-pc-windows-gnu`, sin MinGW ni MSVC).
   - No añadas crates que necesiten MSVC o un compilador de C++.
   - No subas `windows-sys` en `camcuts-gui` (0.59) sin comprobar que los tres exe arrancan: ver el sustituto de dlltool en [docs/compilar_y_probar.md](docs/compilar_y_probar.md#toolchain-y-sustituto-de-dlltool).
9. **Decisiones en INTERFACES.md.** Un cambio de contrato o de comportamiento con medidas se apunta allí. La documentación de `docs/` lo enlaza, no lo copia.
10. **Modelos.** Nunca en Git. Si cambias uno a propósito: `scripts\verificar_modelos.ps1 -Generar` y sube `models\MANIFIESTO.sha256`.
11. **Scripts `.ps1` en ASCII**, sin tildes ni eñes: Windows PowerShell 5.1 lee como ANSI los archivos sin BOM.

## Dónde está cada cosa

| Qué | Dónde |
|---|---|
| Opciones de la línea de comandos, bucle de clips, respaldo automático | `crates/camcuts-core/src/cli.rs` (`Args`, `run_con_respaldo`, `analizar_lote`, `respaldar`) |
| Modo de GPU, clips en paralelo, VRAM | `crates/camcuts-core/src/hardware.rs` (`decide_gpu_mode`, `parallel_clips`, `vram_workers`) |
| Detector elegido y reglas de caché | `crates/camcuts-core/src/detector.rs` |
| Edición y archivos de cada edición | `crates/camcuts-core/src/edicion.rs` |
| Muestreo con ffmpeg, proxies, timecode | `crates/camcuts-core/src/sampler.rs` |
| Medida por muestra | `jewel.rs`, `metrics.rs`, `image.rs` (en `camcuts-core/src`) |
| Color (LOG, LUT, perfiles) | `crates/camcuts-core/src/color.rs` |
| Criterios, puntuación, tramos | `crates/camcuts-core/src/scoring.rs` |
| OTIO y CSV | `crates/camcuts-core/src/export.rs` |
| Caché, proyecto `.camcuts`, JSON de resultados | `crates/camcuts-core/src/project.rs` |
| Edición Bodas | `crates/camcuts-core/src/bodas.rs` |
| Contrato `Detector` | `crates/camcuts-core/src/types.rs` |
| ONNX Runtime, motor, sesiones de forma fija | `crates/camcuts-infer/src/runtime.rs` |
| `OnnxDetector`, `InferConfig`, fábrica y detector compartido | `crates/camcuts-infer/src/lib.rs` |
| Detector propio | `crates/camcuts-infer/src/trained.rs` |
| YOLO, SAM, CLIP, voz, momentos, preprocesado | `yolo.rs`, `sam.rs`, `clip.rs`, `vad.rs`, `moments.rs`, `imgops.rs` (en `camcuts-infer/src`) |
| Binarios | `crates/camcuts-gui/src/main.rs`, `main_bodas.rs`; `crates/camcuts-core/src/bin/camcuts-cli.rs` |
| Proceso hijo `--analizar`, Job Object, proxies de la ventana | `crates/camcuts-gui/src/analisis.rs` |
| Estado de la ventana | `crates/camcuts-gui/src/app.rs` |
| Dibujo | `paneles.rs` (Marfil), `bodas_ui.rs` (Salvia), `tema.rs`, `bodas_tema.rs`, `widgets.rs` |
| Visor y miniaturas | `crates/camcuts-gui/src/video.rs` |
| Modelos y su preparación | `models/` y [models/README.md](models/README.md) |
| Referencias de Python | `fixtures/` y `crates/camcuts-core/tests/data/` |
| Compilar, verificar modelos, empaquetar | `scripts/` |
| Entrenamiento del detector propio | `training/scripts/` |
| Documentación | `docs/` (índice en el [README](README.md#documentación)) |

## Cómo validar un cambio

1. **Compila:** `cargo build --release -p camcuts-gui -p camcuts-core`.
2. **Pasa las tres suites** (sin ventanas; solo `robustez` usa NVDEC un momento):

   ```powershell
   cargo test --release -p camcuts-core --lib --tests
   cargo test --release -p camcuts-infer --lib
   cargo test --release -p camcuts-gui --lib
   ```

   Hoy dan 51 + 19 (más `real_clip`, ignorada), 9 y 32. Ojo con las que se saltan solas: ver [docs/compilar_y_probar.md](docs/compilar_y_probar.md#pruebas-que-se-saltan-solas).
3. **Según lo que toques:**

   | Si tocas… | Además |
   |---|---|
   | Modelos o su preprocesado (`camcuts-infer`) | `cargo test --release -p camcuts-infer --test python_fixtures -- --test-threads=1 --nocapture` (GPU, con turno; necesita los modelos FP32) |
   | La ventana | Las capturas con `CAMCUTS_BENCH` o `CAMCUTS_BODAS_BENCH`, y mira los PNG de `design\snapshots\` |
   | Algo que pueda cambiar los selects (muestreo, medida, puntuación, detectores) | La regresión de punta a punta de [docs/compilar_y_probar.md](docs/compilar_y_probar.md#pruebas-de-punta-a-punta-regresión): CSV idéntico, o la diferencia medida y apuntada en INTERFACES.md |
   | Bodas | `bodas::tests` (en la suite del núcleo) y `capturas_bodas`. Joyas no debe cambiar |
   | El empaquetado | `scripts\armar_dist.ps1 -Prueba` |

4. **Documenta:** la decisión en INTERFACES.md y, si cambia el uso, el documento de `docs/` que corresponda.

## Trampas conocidas

- **En release, `camcuts.exe` no escribe en la consola** (subsistema de ventanas). Lánzalo con `Start-Process … -NoNewWindow -Wait -RedirectStandardOutput … -RedirectStandardError …`. Ejemplo en [docs/operacion.md](docs/operacion.md#cómo-lanzarla-desde-powershell).
- **`CAMCUTS_EDICION=bodas` convierte `camcuts.exe` y las pruebas en Bodas.** `CAMCUTS_DETECTOR` cambia el detector por defecto. No las dejes puestas.
- **Las pruebas de `robustez` "pasan" sin ffmpeg**, sin probar nada.
- **`python_fixtures` necesita los modelos FP32**, que no vienen en `dist\`.
- **`.cargo/config.toml` lleva una ruta absoluta.** En otra carpeta, `scripts\configurar_compilacion.ps1`.
- **`rescore_igual_que_python` compara finales de línea `\r\n`** con los archivos de `tests/data/rescore/`. `.gitattributes` los marca `-text` para que Git no los toque; si se quita esa línea, la prueba falla en un clon con LF.
- **Un exe copiado a otra carpeta sin `models\` al lado** usa los del workspace donde se compiló (`runtime::default_models_dir`).

## Discrepancias encontradas

Revisado contra el código el 2026-11-06. La lista completa está en el [README](README.md#discrepancias-encontradas).

- **Detector por defecto.** INTERFACES, en "Detector propio (añadido)", dice que YOLO-World es el predeterminado. El código usa el propio (`DetectorKind`, `#[default] Propio`). La sección "Own detector by default" lo corrige; manda el código.
- **Identificadores en inglés.** INTERFACES lo pide, pero el código reciente usa nombres en español (`cli::analizar_lote`, `camcuts_infer::Compartido`, casi toda `camcuts-gui`). Sigue el estilo del módulo.
- **Clips en paralelo y modo ligero.** El texto antiguo de INTERFACES ("modo ligero = 1 clip, YOLO-World m") solo vale para YOLO-World; con el detector propio manda `hardware::parallel_clips`.
