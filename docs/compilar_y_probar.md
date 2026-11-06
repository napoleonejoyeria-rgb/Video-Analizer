# Compilar y probar CAMCUTS

Cómo preparar un equipo, compilar, pasar las pruebas y comprobar que el repositorio está completo. Todos los comandos son de Windows PowerShell 5.1 y se ejecutan desde la raíz del repositorio.

## Requisitos

| Qué | Para qué | Notas |
|---|---|---|
| Windows 10/11 de 64 bits | Todo | El código usa APIs de Windows (Job Objects, DXGI, prioridad del proceso) |
| Rust con rustup, toolchain `stable-x86_64-pc-windows-gnu` | Compilar | Probada con rustc 1.99.1. **No** hace falta MinGW: ver la sección siguiente. Con MSVC no se ha probado |
| ffmpeg y ffprobe en el PATH | Analizar, pruebas de `robustez`, visor | Aquí: Gyan.FFmpeg 9.0.2 (`winget install Gyan.FFmpeg`). Abre otra terminal después de instalarlo |
| GPU con DirectX 12 | Análisis y pruebas de GPU | Sin GPU se puede analizar en la CPU, muy despacio (ver [operacion.md](operacion.md)) |
| `nvidia-smi` | Medir la VRAM libre de todo el equipo | Opcional. Sin él se usa DXGI, que solo ve este proceso |
| `runtime\onnxruntime.dll` y `runtime\DirectML.dll` | Todo lo que carga modelos, y una prueba de `camcuts-infer` | No van en Git |
| Los modelos `.onnx` (1.6 GB) | Analizar y pruebas de paridad | No van en Git |
| Disco | `target\` en release ocupa unos 1.9 GB | En debug, unos 15 GB: no se compila en debug |
| Conexión a internet | La primera compilación descarga los crates | Las versiones están fijadas en `Cargo.lock` |
| Python (el venv de la app de Python) | Solo para regenerar referencias | No hace falta para compilar ni probar |

## Toolchain y sustituto de dlltool

**El problema.** Varios crates enlazan DLL de Windows con `raw-dylib` (windows-link, windows-sys 0.60 o posterior, libloading 0.9, ort-sys). Para eso rustc crea bibliotecas de importación con `dlltool`. El `dlltool.exe` que trae rustup llama al ensamblador `as` de MinGW, y MinGW no está instalado.

**La solución.** `.cargo/config.toml` le dice a rustc que use `.cargo/dlltool-lld.cmd` en lugar de dlltool:

```toml
[target.x86_64-pc-windows-gnu]
rustflags = ["-C", "dlltool=C:/Users/Usuario/Tools/camcuts_rs/.cargo/dlltool-lld.cmd"]
```

- `dlltool-lld.cmd` recibe los mismos argumentos que dlltool (`-d archivo.def -D nombre.dll -l salida.lib`) y llama a `dlltool-lld.ps1`.
- `dlltool-lld.ps1` crea la biblioteca con `rust-lld -flavor link /lib`, que viene con la toolchain. Lo busca en `%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-gnu\...\bin\rust-lld.exe`. Si rustup está en otra carpeta, falla.
- **Cada biblioteca lleva su propia combinación de mayúsculas en el nombre de la DLL.** Sin eso, dos bibliotecas que importan la misma DLL (por ejemplo dos versiones de windows-sys con `gdi32.dll`) se juntaban en un solo descriptor, y el programa fallaba antes de `main` con `0xC0000005`. Windows no distingue mayúsculas al cargar una DLL, así que el truco es inocuo.
- Por la misma razón, `camcuts-gui` usa `windows-sys` 0.59, que enlaza sin `raw-dylib`. No lo subas de versión sin comprobar que los tres exe arrancan.
- La sección de `config.toml` solo aplica al destino `x86_64-pc-windows-gnu`. Con la toolchain MSVC no se usa (y no se ha probado).

## Después de clonar

cargo pasa la ruta de `-C dlltool=...` tal cual a rustc, así que tiene que ser absoluta. Después de clonar (o de mover la carpeta), ajústala una vez:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\configurar_compilacion.ps1
```

- Reescribe la ruta de `.cargo/config.toml` con la de la carpeta actual y lo dice.
- Si ya apuntaba allí, dice "config.toml ya apunta a esta carpeta".
- Cambiar las opciones de rustc obliga a recompilar todo la primera vez.
- **Git verá `.cargo/config.toml` modificado.** Es lo esperado en un clon en otra carpeta; no subas ese cambio.

## Modelos y runtime

Los `.onnx` y las `.dll` no van en Git (`.gitignore`). Se copian aparte y se comprueban contra `models\MANIFIESTO.sha256`, que lista el SHA-256, el tamaño y la ruta de cada archivo.

### Qué pide el manifiesto

19 archivos: 17 modelos y 2 DLL. La app instalada (`dist\`) solo trae los que usa al analizar; los demás sirven a las pruebas de paridad y a las medidas.

| Archivo | MB | Para qué | En `dist\` |
|---|---|---|---|
| `models\jewel_det_fp16.onnx` | 19 | Joyas: detector propio (el predeterminado) | Joyas |
| `models\jewel_det.onnx` | 38 | Solo comparaciones FP32 de `training\scripts`; el código Rust siempre usa el FP16 | No |
| `models\yolo_world_x.onnx` | 292 | Joyas: detector clásico y respaldo automático. Paridad | Joyas |
| `models\yolo_world_m.onnx` | 114 | Joyas: clásico en modo ligero. Paridad | Joyas |
| `models\yolo_world_x_fp16.onnx` | 146 | Paridad, `examples/bench.rs` y `CAMCUTS_WORLD_FP16=1` | No |
| `models\yolo_world_m_fp16.onnx` | 57 | Igual | No |
| `models\mobile_sam_encoder_fp16.onnx` | 14 | Siluetas (las dos ediciones) | Las dos |
| `models\mobile_sam_decoder_fp16.onnx` | 8 | Siluetas (las dos ediciones) | Las dos |
| `models\mobile_sam_encoder.onnx` | 28 | Paridad y `bench` en FP32 | No |
| `models\mobile_sam_decoder.onnx` | 17 | Paridad y `bench` en FP32 | No |
| `models\yolo11n_pose_fp16.onnx` | 6 | Pose (las dos ediciones) | Las dos |
| `models\yolo11n_pose.onnx` | 13 | Paridad y `bench` en FP32 | No |
| `models\clip_visual_fp16.onnx` | 176 | Verificador CLIP; en Bodas también los momentos | Las dos |
| `models\clip_visual.onnx` | 352 | Paridad y `bench` en FP32 | No |
| `models\silero_vad.onnx` | 2 | Voz (las dos ediciones) | Las dos |
| `models\bodas\yolo_world_bodas_x.onnx` | 292 | Bodas: detector | Bodas, en `models\` sin subcarpeta |
| `models\bodas\yolo_world_bodas_m.onnx` | 114 | Bodas: modo ligero | Bodas, en `models\` sin subcarpeta |
| `runtime\onnxruntime.dll` | 21 | ONNX Runtime 1.24.4 (compilación DirectML) | Las dos, en `runtime\` |
| `runtime\DirectML.dll` | 19 | DirectML | Las dos, en `runtime\` |

**Los `.json` sí van en Git:** `clip_text.json`, los `*.classes.json` y `bodas\clip_moments.json`.

**Por qué hay FP16 y FP32.** La app usa YOLO-World en FP32 y el resto en FP16 (`InferConfig::default`). Las pruebas de paridad con Python (`crates/camcuts-infer/tests/python_fixtures.rs`) miden **las dos** precisiones de YOLO-World x y m, MobileSAM, pose y CLIP, y fallan si falta cualquiera. Las tolerancias medidas están en [fixtures/README.md](../fixtures/README.md).

### De dónde sacarlos

| Fuente | Trae | Sirve para |
|---|---|---|
| El workspace original, `C:\Users\Usuario\Tools\camcuts_rs\models` y `runtime` | Los 19 | Todo |
| La carpeta compartida de la empresa | Los 19 (es la copia de referencia) | Todo |
| `dist\CAMCUTS\` y `dist\CAMCUTS Bodas\` | 12 de 19 (faltan unos 650 MB: los 5 FP32 y los 2 FP16 de YOLO-World) | Compilar, analizar, empaquetar y las pruebas `--lib`. **No** las de paridad |

### Copiar y comprobar

Desde una fuente completa:

```powershell
$origen = 'C:\Users\Usuario\Tools\camcuts_rs'      # o la carpeta compartida
New-Item -ItemType Directory -Force runtime, models\bodas | Out-Null
Copy-Item "$origen\runtime\*.dll" runtime\
Copy-Item "$origen\models\*.onnx" models\
Copy-Item "$origen\models\bodas\*.onnx" models\bodas\
```

Desde los paquetes de `dist\` (fíjate en que los de Bodas van a `models\bodas\`):

```powershell
$joyas = 'C:\Users\Usuario\Tools\camcuts_rs\dist\CAMCUTS'
$bodas = 'C:\Users\Usuario\Tools\camcuts_rs\dist\CAMCUTS Bodas'
New-Item -ItemType Directory -Force runtime, models\bodas | Out-Null
Copy-Item "$joyas\runtime\*.dll" runtime\
Copy-Item "$joyas\models\*.onnx" models\
Copy-Item "$bodas\models\yolo_world_bodas_*.onnx" models\bodas\
```

Después, siempre:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1
```

- Con una fuente completa: `19 correctos, 0 con problemas` y código de salida 0.
- Solo con `dist\`: `12 correctos, 7 con problemas`, con siete líneas `FALTA` y código 1. Es lo esperado. Lo que **no** debe salir nunca es `HASH MAL` ni `TAMANO MAL`.
- `-Generar` reescribe el manifiesto. Úsalo solo cuando cambies un modelo a propósito, y sube el manifiesto nuevo.

La app busca `models\` y `runtime\` junto al exe y, si no están, en el workspace donde se compiló (`runtime::default_models_dir`). Por eso `target\release\camcuts.exe` usa los del repositorio.

## Compilar

```powershell
cargo build --release -p camcuts-gui -p camcuts-core
```

- Deja en `target\release\`: `camcuts.exe` y `camcuts-bodas.exe` (de `camcuts-gui`) y `camcuts-cli.exe` (de `camcuts-core`).
- **Siempre en release.** El perfil de desarrollo compila las dependencias optimizadas y aun así `target\` llega a unos 15 GB. Este PC tiene unos 20 GB libres en C:.
- Para una comprobación de integridad, añade `--locked`: falla si `Cargo.lock` no cuadra con los `Cargo.toml`.
- `camcuts.exe` y `camcuts-bodas.exe` son del subsistema de ventanas en release: no abren consola. **Sin `--analizar` abren la ventana.**

## Pruebas

### Resumen

| Comando | Pruebas | Necesita | GPU |
|---|---|---|---|
| `cargo test --release -p camcuts-core --lib --tests` | 51 de la librería + 19 de integración (+1 ignorada) | ffmpeg; las cachés de `camcuts_next` para `rescore` | No (`robustez` usa NVDEC un momento si lo hay) |
| `cargo test --release -p camcuts-infer --lib` | 9 | `runtime\*.dll` | No |
| `cargo test --release -p camcuts-gui --lib` | 32 | Nada externo (dos lanzan ffmpeg o ffprobe sobre un archivo que no existe y esperan el fallo) | No |
| `cargo test --release -p camcuts-gui --test capturas` | 1 | `CAMCUTS_BENCH` | wgpu fuera de pantalla |
| `cargo test --release -p camcuts-gui --test capturas_bodas` | 1 | `CAMCUTS_BODAS_BENCH` | wgpu fuera de pantalla |
| `cargo test --release -p camcuts-infer --test python_fixtures -- --test-threads=1 --nocapture` | 5 | Los 17 modelos y el runtime | **Sí** (DirectML) |
| `cargo test --release -p camcuts-core --test real_clip -- --ignored --nocapture` | 1 | Un clip de `D:\` y una caché de `camcuts_next` | NVDEC |

Las tres primeras pasaron hoy (2026-11-06) y son las que hay que pasar siempre. **No uses `cargo test -p camcuts-infer` sin `--lib`:** también corre `python_fixtures`, que usa la GPU y falla si falta un modelo.

### camcuts-core

**Librería (51).** Formatos de Python (`round`, `repr`, `json.dumps`), nombres de caché como Python, escritura atómica de cachés, aritmética de imagen de 8 bits, agrupación de joyas, timecode y drop-frame, firma del proxy, regla de caché y respaldo del detector, ediciones, decisión del modo de GPU y de los clips en paralelo, criterios de Bodas, y el CSV bloqueado por Excel (`cli::pruebas::csv_bloqueado_no_tumba_el_analisis`).

**Integración:**

| Archivo | Pruebas | Qué cubre | Datos |
|---|---|---|---|
| `tests/detector_cache.rs` | 4 | Solo se reutilizan las cachés del detector elegido; `--detector`; sin opción manda el propio; `--rescore` usa todas | Ninguno (sin modelos) |
| `tests/edicion_cache.rs` | 2 | Una caché de la otra edición se vuelve a analizar | Ninguno |
| `tests/fixtures.rs` | 2 | `jewel::measure` y la voz frente a Python | `fixtures/` (en el repositorio) |
| `tests/python_numbers.rs` | 2 | Medidas de imagen y conversiones de color frente a Python | `tests/data/numbers/` (en el repositorio) |
| `tests/rescore.rs` | 1 | `--rescore` sobre 19 cachés de Python da el mismo OTIO y CSV que Python, con 5 combinaciones de opciones | `tests/data/rescore/` y `C:\Users\Usuario\Tools\camcuts_next\bench_oct27\cache` (fuera del repositorio) |
| `tests/robustez.rs` | 8 | Clips sintéticos con ffmpeg `lavfi`: proxies verticales, archivos rotos y cortados, `-skip_frame noref`, clips cortos y frecuencias raras, timecode 23.976, marca del detector en la caché, un lote que sigue tras un archivo dañado | ffmpeg; escribe en `target\tmp` |
| `tests/real_clip.rs` | 1, ignorada | Muestreador con C9182.MP4 frente a la caché de Python | `D:\` y `camcuts_next` |

### camcuts-infer

**Librería (9).** Letterbox y redondeos como OpenCV y Ultralytics, la caché de sesiones de forma fija (`runtime::lru_tomar`), el relleno de los momentos de Bodas, las bandas de confianza del detector propio, `DetectorKind::take_arg`, el cambio de YOLO-World x al propio, la fábrica de detectores compartidos y el recuerdo de un fallo de SAM.

**Paridad (`tests/python_fixtures.rs`, 5).** Detecciones de YOLO-World x y m, siluetas de MobileSAM, pose, CLIP y Silero frente a `fixtures/python_ref.json`. Cada prueba mide FP16 y FP32, imprime las cifras y exige las tolerancias a la configuración por defecto. Usa la GPU: pide turno ([GPU_TURNOS.md](../GPU_TURNOS.md)).

### camcuts-gui

**Librería (32).** Sin ventana: lectura de las líneas de avance, el Job Object que mata al análisis con la ventana, material sin conexión, proyectos de la otra edición, detector guardado en el proyecto, tiempos como Python, el visor (búsqueda fallida, perfil de cámara), proyectos de ida y vuelta.

**Capturas (`tests/capturas.rs`, `tests/capturas_bodas.rs`).** Dibujan la ventana con egui_kittest sobre wgpu fuera de pantalla y guardan PNG en `design\snapshots\`. **Nunca abren una ventana**, pero sobrescriben esos PNG. Los de joyas (`marfil_*.png`) están en Git; los de Bodas no, porque muestran personas.

```powershell
# una carpeta de resultados del material de Octubre 27, con sus 19 clips analizados
$env:CAMCUTS_BENCH = 'C:\Users\Usuario\Tools\camcuts_rs\bench_perf\regresion\oct27'
cargo test --release -p camcuts-gui --test capturas -- --nocapture
Remove-Item Env:CAMCUTS_BENCH

# una carpeta analizada con camcuts-bodas.exe
$env:CAMCUTS_BODAS_BENCH = 'C:\Users\Usuario\Tools\camcuts_rs\bench_bodas'
cargo test --release -p camcuts-gui --test capturas_bodas -- --nocapture
Remove-Item Env:CAMCUTS_BODAS_BENCH
```

`capturas` exige al menos 19 clips analizados y que el título diga "Octubre 27". `capturas_bodas` usa los clips `CAMCUTS_BODAS_CLIP` (C9058 por defecto) y `CAMCUTS_BODAS_VERTICAL` (IMG_414).

### Pruebas que se saltan solas

Una prueba que se salta **cuenta como superada** en el resumen de cargo. Antes de fiarte de un "ok", mira esta tabla.

| Prueba | Se salta si… | ¿Avisa? |
|---|---|---|
| `robustez::*` (las 8) | ffmpeg no está en el PATH | **No**: pasa sin probar nada. Comprueba antes `ffmpeg -version` |
| `rescore::rescore_igual_que_python` | No existe `C:\Users\Usuario\Tools\camcuts_next\bench_oct27\cache` | Sí: "sin cachés de bench_oct27: prueba saltada" |
| `fixtures::*` | No está `fixtures\python_ref.json` (está en el repositorio, así que corre) | Sí |
| `real_clip::clip_real_como_python` | Siempre (`#[ignore]`); con `--ignored`, si falta el clip o la caché | Sí |
| `detector_cache::*`, `robustez::cortado_*`, `robustez::caches_marcan_su_detector` | `CAMCUTS_EDICION=bodas` | No |
| `detector_cache::sin_opcion_manda_el_propio` | `CAMCUTS_DETECTOR` está definida | No |
| `hardware::tests::gpu_info_coincide_con_nvidia_smi` | No hay `nvidia-smi` | No |
| `camcuts_infer::pruebas::fallo_de_sam_se_recuerda` | No se puede cargar ONNX Runtime | No |
| `camcuts_infer::pruebas::fabrica_no_retiene_el_compartido` | `CAMCUTS_EDICION=bodas` o `CAMCUTS_COMPARTIR=0`. **Sin `runtime\*.dll` no se salta: falla** | — |
| `video::pruebas::cuadro_real_con_color_automatico` | Falta alguna de `CAMCUTS_VISOR_CLIP`, `CAMCUTS_VISOR_FUENTE`, `CAMCUTS_VISOR_PNG` | No |
| `capturas`, `capturas_bodas` | Falta `CAMCUTS_BENCH` o `CAMCUTS_BODAS_BENCH` | Sí |

Corre las suites sin `CAMCUTS_EDICION`, `CAMCUTS_DETECTOR` ni `CAMCUTS_COMPARTIR` en el entorno.

## Pruebas de punta a punta (regresión)

Las pruebas unitarias no ven el análisis completo con modelos y material real. Para eso se analiza material de verdad con el exe nuevo y con el de antes, y se comparan los selects.

### Qué se hizo hoy

**2026-11-06, `bench_perf\regresion\`.** Cada carpeta es una carpeta de resultados (`--out`); a su lado quedan `<nombre>.log` (stdout) y `<nombre>.err` (stderr).

| Conjunto | Material (solo lectura) | Carpetas | Qué se comprobó |
|---|---|---|---|
| Octubre 27 | `D:\CONTENIDO NAPO\2026\Octubre\27\videos` (19 clips) | `oct27`, `oct27_reuso.log`, `oct27_selects_1.csv` | 58 selects. Una segunda corrida reutiliza las 19 cachés (`[cache]`) y da el CSV idéntico |
| Manos | `D:\CONTENIDO NAPO\2026\Noviembre\3\Videos Yo Manos Detalle` (33 clips) | `manos` | 75 selects, 2 clips en paralelo con DaVinci abierto |
| Bodas | Los 9 clips de `bodas_lista.txt` (Octubre 5 y 16) | `bodas`, `bodas_antes`, `bodas_x`, `bodas_x2` | `camcuts-bodas.exe` nuevo frente al anterior (`bench_perf\exe_antes_bugs`), con `--gpu normal`: mismo CSV |
| Proxies | C9264 y C9266 de Manos | `proxy`, `proxy_antes`, `proxy1..3`, `proxy_antes1..2` | Con proxy (sin `--no-proxy`), tres corridas: mismo CSV que el exe anterior |

Además, `t_core`, `t_infer`, `t_gui` y `t_capturas` son las salidas de las cuatro suites, y `bench.log` la de `cargo run --release -p camcuts-infer --example bench`.

**2026-11-05, `training\e2e_runs\r6cmp\`.** Comparación del detector propio r2 con el r6 sobre Manos y Octubre 27. La hizo `training\scripts\eval_r6.py e2e` con `camcuts.exe --analizar <carpeta> --out <salida> --detector propio --no-proxy --no-sheet --sin-respaldo`, sin ventana y con prioridad baja. `eval_r6.py paridad` comparó los segundos aprobados con `parity.py` de la app de Python.

### Las bases no van en el repositorio

`bench_*/`, `robustez/` y `training/` (salvo `scripts/` y `bench/`) están en `.gitignore`. Las bases:
- contienen proxies y medidas del material de la empresa;
- dependen de las rutas de `D:\`: el nombre de cada caché sale de la ruta absoluta del clip;
- ocupan mucho y se rehacen en minutos.

Tampoco va `bench_perf\compare_selects.py`, el script que calcula la concordancia de dos análisis. Si se quiere versionar, hay que moverlo a `scripts\`.

### Cómo rehacer una base y comparar

**1. Guarda los exe de la versión de referencia antes de cambiar nada.** Así se hicieron las de hoy:

```powershell
$antes = 'C:\Users\Usuario\Tools\camcuts_rs\bench_perf\exe_antes_<cambio>'
New-Item -ItemType Directory -Force $antes | Out-Null
Copy-Item target\release\camcuts.exe, target\release\camcuts-bodas.exe, target\release\camcuts-cli.exe $antes
```

Sin `models\` al lado, esos exe usan los modelos del workspace donde se compilaron. Con Git, otra opción es compilar la etiqueta de referencia en otra carpeta.

**2. Pide turno de GPU** ([GPU_TURNOS.md](../GPU_TURNOS.md)) y mira si DaVinci está abierto.

**3. Analiza con los dos exe, con el mismo material y las mismas opciones.** Siempre con `--out` bajo `camcuts_rs`, **nunca en `D:\`**. `camcuts.exe` no escribe en la consola: redirige la salida a archivos.

```powershell
function Analizar([string]$exe, [string]$nombre, [string]$opciones) {
    $out = "C:\Users\Usuario\Tools\camcuts_rs\bench_perf\regresion\$nombre"
    $p = Start-Process -FilePath $exe -ArgumentList "--analizar $opciones --out `"$out`" --no-sheet" `
        -NoNewWindow -Wait -PassThru -RedirectStandardOutput "$out.log" -RedirectStandardError "$out.err"
    "$nombre -> código $($p.ExitCode)"
}
$material = '"D:\CONTENIDO NAPO\2026\Octubre\27\videos"'
$nuevo = 'C:\Users\Usuario\Tools\camcuts_rs\target\release'
Analizar "$antes\camcuts.exe" 'oct27_antes' "$material --detector propio"
Analizar "$nuevo\camcuts.exe" 'oct27'       "$material --detector propio"
```

- Usa rutas absolutas en todo (exe, material, `--out`, `--lista`).
- Para Bodas usa `camcuts-bodas.exe` y `--lista C:\Users\Usuario\Tools\camcuts_rs\bench_perf\regresion\bodas_lista.txt`.
- Si comparas YOLO-World, fija `--gpu normal` en las dos corridas.

**4. Comprueba que las dos corridas trabajaron igual.** La línea `equipo:` de cada `.log` dice el modo y el detector. Si en una sale `GPU OCUPADA — … fue a X s por muestra`, la guarda de lentitud pasó a modo ligero a mitad: repite con la GPU libre.

**5. Compara los selects.**

```powershell
$r = 'C:\Users\Usuario\Tools\camcuts_rs\bench_perf\regresion'
(Get-FileHash "$r\oct27_antes\selects_joyas.csv").Hash -eq (Get-FileHash "$r\oct27\selects_joyas.csv").Hash
```

- `True`: los mismos selects. Es lo que se exige a un arreglo que no debe cambiar resultados.
- `False`: mide la diferencia. Para joyas: `python -B bench_perf\compare_selects.py <referencia> <nuevo>` da el tiempo aprobado de cada uno y la concordancia. Un cambio buscado se apunta en [INTERFACES.md](../INTERFACES.md) con sus cifras.

**6. Comprueba la reutilización.** Repite el mismo comando sobre la misma `--out`. Todas las líneas deben ser `[cache] …` y el CSV debe salir idéntico.

## Lista de comprobación de integridad para TI

El 2026-11-06 se instaló Git 2.56 en este PC y se hicieron los pasos 0 a 2: el repositorio tiene el primer commit en `main`, con la etiqueta `base-2026-11-06`, y está publicado en <https://github.com/napoleonejoyeria-rgb/Video-Analizer> (público). Los pasos 0 a 2 quedan como referencia y para repetir la comprobación. A TI le toca el paso 4 (y el 3 si quiere los modelos en Git; en un repositorio público, mejor no).

### 0. Lo que se decidió antes del primer commit

- **Finales de línea de los datos de referencia.** `.gitattributes` tiene `* text=auto`. Git guardará con LF los `.otio`, `.csv`, `.json` y `.cube` de `crates\camcuts-core\tests\data\` y `fixtures\`, que hoy tienen CRLF. En un clon de Windows con la configuración por defecto vuelven a salir con CRLF. Con `core.autocrlf=input`, o fuera de Windows, salen con LF y `rescore_igual_que_python` falla, porque compara los `\r\n` con los de Python. Por eso `.gitattributes` ya trae estas dos líneas, que los guardan byte a byte:

  ```gitattributes
  crates/camcuts-core/tests/data/** -text
  fixtures/** -text
  ```

- **`design\snapshots\`.** Son 14 PNG que reescriben las pruebas de capturas. Los 8 de joyas (`marfil_*.png`) van en el repositorio: sirven para ver en un diff cómo cambió la ventana. Los 6 de Bodas no van, porque muestran caras de personas reales; el `.gitignore` los excluye.
- **Repositorio público.** Está en GitHub como público para que TI lo revise. Antes de subirlo se revisó que no lleve claves, tokens, contraseñas, correos, IP, direcciones ni cadenas de conexión, y que los commits usen el correo privado `noreply` de GitHub. `fixtures\*.png` y `design\snapshots\marfil_*.png` son cuadros del material de la empresa, pero solo muestran joyas y manos.

### 1. Primer commit

```powershell
cd C:\Users\Usuario\Tools\camcuts_rs
git init -b main
git add .
git status --short          # repasa la lista antes de seguir
git commit -m "CAMCUTS en Rust: código, pruebas y documentación"
git tag base-2026-11-06
```

Es normal que `git add` avise de finales de línea ("LF will be replaced by CRLF the next time Git touches it"): en Windows, los archivos de texto salen con CRLF al clonar. Los datos de prueba marcados `-text` no avisan porque Git no los toca.

### 2. Comprueba que el .gitignore deja fuera lo que debe

```powershell
git check-ignore -v target dist bench_perf robustez training/runs models/yolo_world_x.onnx models/bodas/yolo_world_bodas_x.onnx runtime/onnxruntime.dll
git ls-files | Select-String -Pattern '\.(onnx|dll|pt|exe)$', '^(target|dist|robustez|bench_[^/]+)/', '^training/(?!scripts/|bench/)'
git ls-files models
git count-objects -vH
```

| Comando | Resultado esperado |
|---|---|
| `git check-ignore -v …` | Una línea por ruta, con la regla de `.gitignore` que la excluye |
| `git ls-files \| Select-String …` | Nada |
| `git ls-files models` | 12 archivos: `MANIFIESTO.sha256`, `README.md`, `clip_text.json`, los `*.classes.json`, `bodas/clip_moments.json` y los `.py`. Ningún `.onnx` |
| `git count-objects -vH` | `size-pack` de unos 20 MB o menos (el primer commit tiene 231 archivos y 19,4 MB sin comprimir) |

### 3. Opcional: modelos en Git LFS

Solo si TI prefiere tener los modelos en Git en vez de copiarlos aparte. El manifiesto sigue sirviendo para comprobarlos.

```powershell
git lfs install
# 1) quita del .gitignore las líneas /models/**/*.onnx y /runtime/*.dll
# 2) sigue los binarios con LFS (añade reglas a .gitattributes)
git lfs track "models/**/*.onnx" "runtime/*.dll"
git add .gitignore .gitattributes models runtime
git commit -m "Modelos y runtime en Git LFS"
git lfs ls-files            # deben salir 19 archivos
```

Ocupa unos 1.6 GB de almacenamiento LFS, y cada versión nueva de un modelo suma su tamaño.

### 4. Clona y valida en una carpeta limpia

```powershell
git clone https://github.com/napoleonejoyeria-rgb/Video-Analizer C:\Validacion\camcuts_rs
cd C:\Validacion\camcuts_rs
powershell -ExecutionPolicy Bypass -File scripts\configurar_compilacion.ps1
# copia los modelos y el runtime (sección "Modelos y runtime")
powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1
ffmpeg -version
cargo build --release --locked -p camcuts-gui -p camcuts-core
cargo test --release --locked -p camcuts-core --lib --tests
cargo test --release --locked -p camcuts-infer --lib
cargo test --release --locked -p camcuts-gui --lib
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Prueba
git status --short
```

| Paso | Resultado esperado |
|---|---|
| `verificar_modelos.ps1` | `19 correctos, 0 con problemas`. Solo con `dist\`: 7 `FALTA`, ningún `HASH MAL` ni `TAMANO MAL` |
| `ffmpeg -version` | Responde. Si no, `robustez` "pasa" sin probar nada |
| `cargo build` | `camcuts.exe`, `camcuts-bodas.exe` y `camcuts-cli.exe` en `target\release\` |
| `camcuts-core` | 51 de la librería; `detector_cache` 4, `edicion_cache` 2, `fixtures` 2, `python_numbers` 2, `rescore` 1, `robustez` 8; `real_clip` ignorada. Sin `camcuts_next`, `rescore` imprime "prueba saltada" |
| `camcuts-infer --lib` | 9 |
| `camcuts-gui --lib` | 32 |
| `armar_dist.ps1 -Prueba` | Una línea `copiaria` o `igual` por archivo y `Listo.`, sin el error de "Napoleone" |
| `git status --short` | Solo ` M .cargo/config.toml` (lo cambió `configurar_compilacion.ps1`) |

**Los exe no tendrán el mismo SHA-256 que los de `dist\`.** Llevan dentro rutas de la carpeta donde se compilaron (por ejemplo, la de `models\` del workspace). Se valida el comportamiento (pruebas y regresión), no el hash del exe.

## Discrepancias encontradas

Revisado contra el código el 2026-11-06.

**INTERFACES.md frente al código y al repositorio:**
- La sección "Fixtures" dice que `fixtures/` trae cachés por clip de Python. No hay ninguna. La única prueba con cachés de Python (`tests/rescore.rs`) las lee de `C:\Users\Usuario\Tools\camcuts_next\bench_oct27\cache`, fuera del repositorio. Sin esa carpeta, la prueba se salta.
- INTERFACES cita `training\runs\r6`, `training\e2e_runs\r6cmp` y `bench_perf\modelos_r2` como detalle del modelo r6. Ninguna va al repositorio. También cita `dist\runtime_cuda`, que hoy no existe.

**Otros textos desactualizados:**
- `crates/camcuts-infer/Cargo.toml` dice que basta con poner en el PATH la carpeta `self-contained` de la toolchain (su `dlltool.exe`). Ese dlltool necesita `as`, que no está; lo que funciona es `.cargo/config.toml` con `dlltool-lld.cmd`.
- `training\scripts\e2e_run.py` y `r5_quick.py` llaman a `target\release\camcuts-e2e.exe`. Ese binario sale del crate `training\e2e`, que está fuera del workspace a propósito: `cargo build --release --manifest-path training/e2e/Cargo.toml --target-dir target`. El 2026-11-06 compila con el código actual. Para comparar contra las bases es más directo `camcuts.exe --analizar`, como hace `eval_r6.py`.
