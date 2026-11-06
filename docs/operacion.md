# Operación de CAMCUTS

Cómo analizar sin ventana, qué variables de entorno cambian el comportamiento, cómo se reparte la GPU, cómo se arman e instalan los paquetes y qué hacer cuando algo falla.

## Línea de comandos

### Quién la entiende

Todos estos programas leen las mismas opciones (`camcuts_core::cli::Args`, en `crates/camcuts-core/src/cli.rs`), que son las de la app de Python.

| Programa | Qué hace | Modelos |
|---|---|---|
| `camcuts.exe --analizar …` | Analiza en la edición de joyas. Es lo que lanza la ventana como proceso hijo | Sí |
| `camcuts-bodas.exe --analizar …` | Lo mismo en la edición Bodas | Sí |
| `camcuts-cli.exe …` | Solo `--rescore` y clips ya analizados. Si hay clips sin analizar, sale con error: "este binario no tiene modelos" | No |
| `cargo run --release -p camcuts-infer --example analizar -- …` | Lo mismo que `camcuts.exe --analizar`, sin depender de la ventana | Sí |

- `--analizar` tiene que ser **el primer argumento**. Sin él, `camcuts.exe` y `camcuts-bodas.exe` **abren la ventana**.
- La ventana lanza: `--analizar --lista <out>\clips.txt --out <out> --no-sheet --preset <criterio> --colores <out>\colores.json`, más `--background` si "Segundo plano" está encendido (lo está por defecto), `--zoom` si se pidió y, en joyas, `--detector`.

### Cómo lanzarla desde PowerShell

`camcuts.exe` y `camcuts-bodas.exe` son del subsistema de ventanas: no escriben en la consola. Redirige la salida a archivos y espera a que terminen:

```powershell
$exe = 'C:\Users\Usuario\Tools\camcuts_rs\dist\CAMCUTS\camcuts.exe'
$p = Start-Process -FilePath $exe -NoNewWindow -Wait -PassThru `
    -ArgumentList '--analizar "D:\CONTENIDO NAPO\2026\Octubre\27\videos" --out "C:\Resultados\Octubre 27" --no-sheet' `
    -RedirectStandardOutput 'C:\Resultados\oct27.log' -RedirectStandardError 'C:\Resultados\oct27.err'
$p.ExitCode
```

- Usa rutas absolutas y pon entre comillas las que llevan espacios.
- `camcuts-cli.exe` sí es de consola: se ejecuta normal (`& '...\camcuts-cli.exe' --out … --rescore`).
- Agentes y pruebas: `--out` siempre bajo `C:\Users\Usuario\Tools\camcuts_rs`, **nunca en `D:\`** (ver [CLAUDE.md](../CLAUDE.md)).

### Opciones

| Opción | Valores (por defecto) | Qué hace |
|---|---|---|
| `<entradas>…` | rutas | Clips o carpetas. De una carpeta se toman los videos de dentro, **sin subcarpetas**, en orden alfabético. Se saltan las copias `._*` de macOS |
| `--out <carpeta>` | **obligatoria** | Carpeta de resultados. Se crea con su `cache\` |
| `--lista <archivo>` | — | Texto con una ruta por línea; se suma a las entradas. Acepta BOM, comillas y líneas vacías |
| `--preset <nombre>` | `joyas` · `bodas` · `social` · `todo` (`joyas`) | Criterio: equilibrado, estricto (cortes de hasta 4 s, 3 por clip) o permisivo. En Bodas, `joyas` significa `bodas`; en joyas, `bodas` significa `joyas` |
| `--fps <n>` | `4` | Muestras por segundo |
| `--rescore` | — | No analiza: vuelve a puntuar todas las cachés de esta edición, sean del detector que sean, y exporta. No necesita modelos ni GPU |
| `--modo <modo>` | `aprobados` · `v1v2` · `marcadores` (`v1v2`) | `aprobados`: solo los tramos. `v1v2`: clips completos en V1 y los aprobados en V2. `marcadores`: clips completos con los tramos como marcadores |
| `--solo-aprobados` | — | Igual que `--modo aprobados` |
| `--timeline-fps <fps>` | la del clip más común | fps de la línea de tiempo. Debe coincidir con el proyecto de DaVinci |
| `--tipo <lista>` | todos | Solo estos tipos de plano. Joyas: `producto,modelo,hablando`. Bodas: `hablando,primer_plano,plano_medio,plano_general,detalle` |
| `--joya <lista>` | todas | Solo estas joyas principales: `dije,anillo,arete,topo,pulsera,collar,cadena` |
| `--momento <lista>` | todos | Bodas: solo tramos de estos momentos (`preparativos,ceremonia,votos,beso,primer baile…`). Si también das `--joya`, manda `--joya` |
| `--colores <json>` | `<out>\colores.json` | Conversión de color por clip: `{ruta en minúsculas: {"id": conversión, "cube": ruta .cube}}`. Un clip sin entrada usa "auto". Si la conversión de un clip cambia, se vuelve a analizar |
| `--no-proxy` | — | No escribe el video de vista previa (`<out>\cache\proxies\`). La ventana los crea luego en segundo plano |
| `--proxy-dir <carpeta>` | repetible | Dónde buscar proxies de clips RAW (`.braw`, `.r3d`, `.nev`, `.ari`, `.arx`, `.mxr`, `.nraw`). También se miran carpetas llamadas `Proxy`, `Proxies`, `ProxyMedia` o `Proxy Media` |
| `--no-sam` | — | Mide la nitidez sobre la caja y no sobre la silueta de MobileSAM. Más rápido, menos fino |
| `--zoom` | — | Segunda mirada a 1920 px en orejas, cuello y muñecas, para tomas lejanas. Más lento |
| `--no-sheet` | — | Se acepta por compatibilidad con Python; Rust no hace hoja de contacto |
| `--background` | — | Prioridad baja del proceso (la heredan sus ffmpeg) y menos clips en paralelo. Ver [Clips en paralelo](#clips-en-paralelo) |
| `--workers <n>` | según el equipo | Máximo de clips en paralelo. Si se da, no se aplica el tope de VRAM: es para pruebas |
| `--gpu <modo>` | `auto` · `normal` · `ligero` (`auto`) | Ver [Modo normal y modo ligero](#modo-normal-y-modo-ligero) |
| `--detector <nombre>` | `propio` · `world` | Joyas: el detector propio o YOLO-World ("Clásico" en la ventana). Sin la opción manda `CAMCUTS_DETECTOR` y, si no está, el propio. Bodas usa siempre su YOLO-World |
| `--sin-respaldo` | — | Apaga el respaldo automático del detector propio (ver abajo) |
| `--motor <motor>` | `auto` · `cuda` · `directml` | Motor de la GPU. Sin la opción manda `CAMCUTS_MOTOR` y, si no está, DirectML. CUDA es solo para pruebas y necesita la carpeta `runtime_cuda`, que no está en el repositorio ni en los paquetes |
| `-h`, `--help` | — | La ayuda |

**Caché.** Un clip con caché no se vuelve a analizar si su caché es de esta edición, del mismo detector y con la misma conversión de color, y si el archivo no cambió desde que se marcó como cortado. Si no, se analiza de nuevo y se reescribe la caché.

**Respaldo automático.** Con el detector propio, al terminar se miran todos los clips (nuevos y de caché). Uno se analiza otra vez con YOLO-World si tiene al menos 2 s de joya y el criterio "joyas" aprueba menos del 25 % de ese tiempo (`detector::necesita_respaldo`). Ese análisis sustituye al del propio y su caché queda marcada (`"respaldo": "propio"`) para no repetirlo. También pasa por el respaldo un clip que el propio analizó en lugar de YOLO-World por falta de memoria (`"en_lugar_de": "world"`).

**Reanálisis más rápido.** Si el análisis no escribe proxy (porque ya hay uno al día o por `--no-proxy`) y el clip es H.264, solo se decodifican los cuadros de referencia (`-skip_frame noref`, `sampler::saltar_b`). Los cuadros salen idénticos.

### Qué deja en la carpeta de resultados

| Archivo | Qué es |
|---|---|
| `cache\<clip>_<hash>.json` | Las medidas de cada clip, con el formato de Python. El nombre sale de la ruta absoluta del clip |
| `cache\proxies\*.mp4` | Videos de vista previa: lado largo de 960 px, x264 CRF 20 |
| `selects_<criterio>.otio` y `.csv` | Joyas: `selects_joyas`, `selects_social`, `selects_todo`. Bodas: `selects_bodas`, `selects_bodas_social`, `selects_bodas_todo` |
| `errores.txt` | Clips que no se pudieron analizar, con el motivo. Se borra cuando no falla ninguno |
| `necesitan_proxy.txt` | Clips RAW sin proxy |
| `colores.json`, `clip_types.json` | Los lee: color por clip y tipo fijado por el editor |

La ventana además escribe `clips.txt`, `clips_meta.json` y el proyecto `.camcuts`, y su botón Exportar escribe `CAMCUTS_<criterio>_<modo>.otio` (en Bodas, `CAMCUTS_Bodas_…`). Los formatos están en [formatos.md](formatos.md).

Si un archivo no se puede reemplazar (otro programa lo tiene abierto), el nuevo queda al lado como `<archivo>.tmp` y el error lo dice.

### Qué escribe en la salida

| Línea | Significa |
|---|---|
| `motor de la GPU: DirectML` | Motor con el que se abrió ONNX Runtime |
| `equipo: <GPU> (8 GB, 3.4 GB libres) … -> 2 clips en paralelo · detector propio` | Modo, detector y clips en paralelo de este pase. La ventana la muestra en la barra de estado, en rojo si dice `OCUPADA` |
| `[cache] C9168.MP4` | Clip reutilizado de la caché |
| `[3/19] C9170.MP4: 188 muestras, joya 79%, … 38.9s (GPU, 25% quietas)` | Clip terminado |
| `[3/19] [error] <clip>: <motivo>` | Clip que falló; el resto sigue |
| `[3/19] [proxy?] <clip>: formato RAW sin proxy encontrado` | RAW sin proxy |
| `respaldo: …` | Clips que van al respaldo con YOLO-World, y su tiempo |
| `línea de tiempo a 29.970 fps (debe coincidir con el proyecto de DaVinci)` | fps de la exportación |
| `58 selects -> <ruta>.otio` | Fin de la exportación |
| `aviso: …` (stderr) | Problemas que no paran el análisis |
| `progreso<TAB>37<TAB><ruta>` | Avance de un clip, solo con `CAMCUTS_PROGRESO` |

### Códigos de salida

| Código | Cuándo |
|---|---|
| 0 | Terminó. **También** si algunos clips fallaron (están en `errores.txt`) o si no se pudo reemplazar el CSV o el OTIO (sale un `aviso:`) |
| 1 | Un error que para todo: no se pudo crear `--out`, no se pudo leer `--lista`, no se pudo cargar ONNX Runtime, o hay clips sin analizar y el binario no tiene modelos |
| 2 | Opciones mal escritas |

Con `--motor auto`, si CUDA falla, el programa se relanza a sí mismo con DirectML y devuelve el código de ese segundo proceso.

### Ejemplos

```powershell
# Bodas: solo votos y beso, con la lista de clips de la ventana
camcuts-bodas.exe --analizar --lista "C:\Resultados\Boda\clips.txt" --out "C:\Resultados\Boda" --no-sheet --momento votos,beso

# Volver a puntuar con otro criterio, sin GPU (segundos)
& 'C:\Users\Usuario\Tools\camcuts_rs\dist\CAMCUTS\camcuts-cli.exe' --out "C:\Resultados\Octubre 27" --rescore --preset social --modo aprobados

# Joyas con YOLO-World, sin proxies y con 1 clip a la vez
camcuts.exe --analizar "D:\material" --out "C:\Resultados\material" --detector world --no-proxy --workers 1
```

(Los dos últimos `camcuts*.exe --analizar` se lanzan con `Start-Process`, como arriba.)

## Variables de entorno

Todas las `CAMCUTS_*` del código. La ventana pasa su entorno al proceso hijo del análisis, así que también le afectan.

### Uso y diagnóstico

| Variable | Valores | Efecto | Dónde se lee |
|---|---|---|---|
| `CAMCUTS_DETECTOR` | `propio`, `world` (también `own`, `trained`, `yolo-world`, `yoloworld`) | Detector cuando no se da `--detector`. La ventana siempre pasa `--detector`, así que no le cambia nada | `detector::VARIABLE`, `DetectorKind::from_env` |
| `CAMCUTS_EDICION` | `bodas` | Edición del proceso cuando el binario no la fija: `camcuts.exe` y `camcuts-cli.exe` pasan a comportarse como Bodas. **No la dejes puesta** | `edicion::actual` |
| `CAMCUTS_DISPOSITIVO` | `cpu` | Todos los modelos en la CPU. No se mira la VRAM ni hay guarda de lentitud. Unos 2 s por muestra nueva | `hardware::en_cpu`, `InferConfig::default` |
| `CAMCUTS_DECODE` | `cpu` | Decodifica en la CPU, sin NVDEC | `sampler::attempts` |
| `CAMCUTS_MOTOR` | `directml` (por defecto), `cuda`, `auto` | Motor de la GPU cuando no se da `--motor` | `runtime::requested_engine` |
| `CAMCUTS_RUNTIME_CUDA` | carpeta | Dónde está `runtime_cuda`. Sin ella: junto al exe, en la carpeta de encima o en `dist\runtime_cuda` del workspace | `runtime::cuda_runtime_dir` |
| `CAMCUTS_CALIBRATION` | ruta de un JSON | Calibración del editor sobre el criterio, con los nombres de campo del preset (`score_threshold`, `lap_soft`, `weights`…). Sin ella se usa `models\calibration.json` junto al exe, si existe (hoy no existe) | `cli::load_calibration`, `Preset::apply_calibration` |
| `CAMCUTS_CLI` | ruta de un exe | La ventana analiza con ese programa (por ejemplo la CLI de Python) en vez de `camcuts.exe --analizar`, y no le pasa `--detector` | `analisis::ruta_cli` |
| `CAMCUTS_PROFILE` | cualquier valor | Tiempo por etapa de cada clip en stderr, y una línea `[perfil]` por cada sesión de forma fija que se compila | `analyse::Profile::enabled`, `runtime::ShapedSession::get` |
| `CAMCUTS_PROGRESO` | cualquier valor | Líneas `progreso<TAB>N<TAB>ruta` por clip. La ventana la pone a `1` | `cli::VARIABLE_PROGRESO` |

### Comparaciones de velocidad y calidad

| Variable | Valores | Efecto | Dónde se lee |
|---|---|---|---|
| `CAMCUTS_WORLD_FP16` | `1` | YOLO-World en FP16: 2.7 veces más rápido, pero pierde tiempo aprobado. Necesita los `_fp16` de YOLO-World, que no van en `dist\` | `InferConfig::default` |
| `CAMCUTS_FORMA_DINAMICA` | cualquier valor | Sesiones de forma dinámica con DirectML (sin compilar el grafo entero) | `ShapedSession::fixes_shapes` |
| `CAMCUTS_COMPARTIR` | `0` | Un detector propio por hilo en vez de uno compartido | `camcuts_infer::fabrica` |
| `CAMCUTS_PREFETCH` | `0` | No abre el clip siguiente mientras termina el actual | `cli::analizar_lote` |
| `CAMCUTS_SALTAR_B` | `0` | Decodifica todos los cuadros al reanalizar H.264 | `sampler::saltar_b` |
| `CAMCUTS_LENTO_S` | segundos | Umbral de la guarda de lentitud (por defecto 0.4 s por muestra) | `cli::analizar_lote` |
| `CAMCUTS_TF32` | `1` | CUDA: permite TF32 | `runtime::tf32` |
| `CAMCUTS_CUDNN_BUSQUEDA` | `exhaustive` (por defecto), `heuristic`, `default` | CUDA: cómo busca cuDNN sus algoritmos | `runtime::cuda_ep` |

### Solo pruebas

| Variable | Para |
|---|---|
| `CAMCUTS_BENCH` | Carpeta de resultados de la prueba `capturas` |
| `CAMCUTS_BODAS_BENCH` | Carpeta de resultados de la prueba `capturas_bodas` |
| `CAMCUTS_BODAS_CLIP`, `CAMCUTS_BODAS_VERTICAL` | Clips de `capturas_bodas` (por defecto `C9058` e `IMG_414`) |
| `CAMCUTS_VISOR_CLIP`, `CAMCUTS_VISOR_FUENTE`, `CAMCUTS_VISOR_PNG` | Prueba opcional `video::pruebas::cuadro_real_con_color_automatico`: clip original, lo que se decodifica y PNG de salida |

### Interna

`CAMCUTS_RESPALDO_DIRECTML` la pone `camcuts_infer::motor_listo` al relanzar el programa con DirectML cuando CUDA falla. Obliga a usar DirectML. No la pongas a mano.

### Otras variables que se leen

| Variable | Efecto |
|---|---|
| `ORT_DYLIB_PATH` | Ruta de `onnxruntime.dll`. Gana a la de junto al exe y a la de `runtime\` |
| `RAYON_NUM_THREADS` | Hilos de las medidas de imagen. Por defecto 4 (`hardware::MEASURE_THREADS`): con más iba más lento |
| `APPDATA` | Preferencias de la ventana: `%APPDATA%\CAMCUTS-rs\settings.json` (Bodas: `%APPDATA%\CAMCUTS-Bodas\settings.json`) |
| `PYTHONIOENCODING` | La ventana la pone a `utf-8` en el hijo, por si `CAMCUTS_CLI` es la CLI de Python |

## Turnos de GPU

**La app no necesita turnos.** Mide la VRAM libre de todo el equipo al empezar cada pase y se adapta (modo ligero, menos clips). Con "Segundo plano" encendido, la ventana analiza con prioridad baja para que el editor siga trabajando.

**Quien trabaja en este PC (agentes, desarrollo, pruebas con GPU) sigue [GPU_TURNOS.md](../GPU_TURNOS.md).** En resumen:

- Si existe `C:\Users\Usuario\Tools\camcuts_rs\.gpu_lock` y tiene menos de 2 h, la GPU es de otro. Trabaja en la CPU mientras tanto.
- Si necesitas la GPU, crea `.gpu_wait` con tu nombre. Quien la tiene acaba su paso (como mucho 30 min) y la suelta.
- Si está libre, crea `.gpu_lock` con tu nombre y la hora, y bórralo al terminar.
- Con DaVinci (`Resolve.exe`) abierto: uso moderado, unos 60 % de media, y espera si deja menos de 2 GB de VRAM libre. Sin DaVinci: todo lo que haga falta, pero sin largos ratos al 99 %.
- Como mucho 2 clips a la vez con YOLO-World x. Después de cada corrida, mira si hubo reinicios del driver.
- Nunca abras ventanas, nunca escribas en `D:\` y nunca mates procesos que no lanzaste.

```powershell
$lock = 'C:\Users\Usuario\Tools\camcuts_rs\.gpu_lock'
if ((Test-Path $lock) -and ((Get-Date) - (Get-Item $lock).LastWriteTime).TotalHours -lt 2) {
    "GPU ocupada: $(Get-Content $lock)"
} else {
    "<tu nombre> $(Get-Date -Format s)" | Set-Content $lock
    $inicio = Get-Date
    $davinci = [bool](Get-Process Resolve -ErrorAction SilentlyContinue)   # true: uso moderado
    # ... el trabajo con GPU ...
    Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'nvlddmkm'; StartTime = $inicio } -ErrorAction SilentlyContinue
    Remove-Item $lock
}
```

Los scripts de Python de `training\scripts` piden turno con `GpuTurn` (`gpu_turn.py`).

## Modo normal y modo ligero

**Cuándo.** Cada pase del análisis (el normal y el del respaldo) decide su modo al empezar (`hardware::decide_gpu_mode`):

| `--gpu` | Modo |
|---|---|
| `auto` (por defecto) | Ligero si la VRAM libre es menor de 4.5 GB (`hardware::LOW_VRAM_GB`); normal si no, o si no hay datos de VRAM |
| `normal` | Normal siempre |
| `ligero` | Ligero siempre |
| (con `CAMCUTS_DISPOSITIVO=cpu`) | Normal; no se mira la VRAM |

La VRAM libre sale de `nvidia-smi`, que cuenta lo que ocupan las demás apps. Si no hay `nvidia-smi`, sale de DXGI, que solo ve este proceso.

**Qué cambia.** Solo cambia algo con YOLO-World:

| Detector | Modo normal | Modo ligero |
|---|---|---|
| Propio (joyas, por defecto) | Propio | Propio: no cambia nada. Los clips en paralelo los limita la VRAM |
| Clásico pedido (joyas) | YOLO-World x | El propio en su lugar, si está instalado; la caché lo dice (`"en_lugar_de": "world"`) y vale para el clásico. Si no está, YOLO-World m. 1 clip a la vez |
| Respaldo del propio (joyas) | YOLO-World x | YOLO-World m, 1 clip a la vez |
| Bodas | `yolo_world_bodas_x` | `yolo_world_bodas_m`, 1 clip a la vez |

En joyas, el propio sustituye a m porque m confunde las joyas de la marca: las medidas están en [INTERFACES.md](../INTERFACES.md).

**A mitad del análisis**, con YOLO-World y en la GPU, el modo puede pasar a ligero:
1. **YOLO-World x no cabe.** Si DirectML no puede abrirlo o compilarlo (por ejemplo `DmlGraphFusionHelper … 80070057`), sigue con el propio (si x aún no dio nada en ese clip) o con m. Sale `equipo: GPU OCUPADA — el detector grande no cabe en la memoria libre de la GPU (¿DaVinci?)…`.
2. **Guarda de lentitud.** Si un clip de al menos 80 muestras va a más de 0.4 s por muestra (lo normal es 0.05), Windows está pasando memoria de la GPU a la RAM. Desde el clip siguiente, modo ligero. Sale `equipo: GPU OCUPADA — <clip> fue a X s por muestra…`. El resultado de ese clip vale.

## Clips en paralelo

El número sale de `hardware::parallel_clips` y luego se recorta con `hardware::vram_workers`. La cifra final aparece en la línea `equipo:`.

**1. Lo que pide el modo y el plano** (`clip_workers` = hilos de CPU / 4, entre 1 y 4; con `--workers n`, como mucho n):

| Detector que analiza | `--background` sin `--workers` | Primer plano |
|---|---|---|
| YOLO-World (clásico o Bodas), modo ligero | 1 | 1 |
| YOLO-World, modo normal | 1 | `clip_workers`, como mucho 2 (`WORLD_MAX_WORKERS`) |
| Propio | 2 (`BACKGROUND_WORKERS_PROPIO`) | `clip_workers` |

**2. Lo que cabe en la VRAM libre** (no se aplica con `--workers`):
- YOLO-World: (libre − 0.5 GB) / 2.6 GB por clip.
- Propio: 1 + (libre − 0.5 − 1.8 GB) / 1.0 GB. Los hilos comparten los modelos (`camcuts_infer::Compartido`); cada clip de más solo suma su ffmpeg con NVDEC.
- Siempre al menos 1. Sin datos de VRAM, sin tope.

Ejemplo: con DaVinci abierto quedan 3.3–4 GB libres. Caben 2 clips con el propio y 1 con YOLO-World.

**Por qué el tope de 2 con YOLO-World.** Con 4 clips de YOLO-World x, el driver de NVIDIA se reinició (TDR) en la RTX 5060. Con el propio, el límite es el decodificador de video (NVDEC), no los modelos.

**Orden.** Con 2 o más clips a la vez, los archivos más grandes van primero, para no acabar con uno largo solo. Con 1 clip a la vez se respeta el orden de la lista. Los resultados siempre vuelven en el orden de las entradas.

**Solapamiento.** Cada hilo abre el clip siguiente (ffprobe, NVDEC, primer GOP) mientras termina de decodificar el actual (`CAMCUTS_PREFETCH=0` lo apaga).

## Empaquetado e instalación

### Armar los paquetes

Primero compila (`cargo build --release -p camcuts-gui -p camcuts-core`). Después:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1                  # las dos ediciones, en dist\
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Edicion bodas   # joyas | bodas | ambas
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Prueba          # solo dice qué haría
powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Destino C:\otra # otra carpeta
```

| Paquete | Contiene |
|---|---|
| `dist\CAMCUTS\` | `camcuts.exe`, `camcuts-cli.exe`; `runtime\` con las dos DLL; `models\` con `clip_text.json`, `clip_visual_fp16.onnx`, `jewel_det.classes.json`, `jewel_det_fp16.onnx`, `mobile_sam_{encoder,decoder}_fp16.onnx`, `silero_vad.onnx`, `yolo11n_pose_fp16.onnx`, `yolo_world_{m,x}.onnx` y sus `.classes.json` |
| `dist\CAMCUTS Bodas\` | `camcuts-bodas.exe`; `runtime\`; `models\` con `clip_moments.json`, `clip_text.json`, `clip_visual_fp16.onnx`, `mobile_sam_*_fp16.onnx`, `silero_vad.onnx`, `yolo11n_pose_fp16.onnx`, `yolo_world_bodas_{m,x}.onnx` y sus `.classes.json` (todo en `models\`, sin subcarpeta); `README.md` y `CLAUDE.md` de `empaque\bodas\` |

**Reglas del script:**
- Copia un archivo solo si cambió (tamaño o SHA-256). Los modelos iguales no se vuelven a copiar: dice `igual`.
- Después de copiar, comprueba el SHA-256 de cada archivo.
- **Bodas sin "Napoleone".** Antes de copiar nada de Bodas, busca la palabra en `camcuts-bodas.exe` (como texto ASCII y UTF-16) y se detiene si aparece. Lo hace también con `-Prueba`. Solo mira el exe, no los `.md` del paquete.
- `-Destino` nunca en `D:\` (el material es de solo lectura), aunque el ejemplo de la cabecera del script use `D:\otra`.

### Si la app está abierta

Windows no deja sobrescribir un `.exe` en uso, pero sí renombrarlo. El script:
1. renombra el viejo a `<nombre>_anterior.exe` (por ejemplo `camcuts_anterior.exe`);
2. copia el nuevo con el nombre de siempre y dice `(estaba abierto: el anterior quedo como …)`.

La app abierta sigue funcionando con el viejo; la próxima vez que se abra, arranca el nuevo. Si el `_anterior.exe` también está abierto, el script para y pide cerrar la app. El `_anterior.exe` se puede borrar a mano con la app cerrada.

Esto vale solo para los `.exe`. Si cambió una DLL de `runtime\` (o un modelo) mientras la app la tiene cargada, la copia puede fallar y el script se detiene: cierra la app y repite.

### Instalar en otro PC

1. Copia la carpeta entera del paquete (`dist\CAMCUTS\` o `dist\CAMCUTS Bodas\`). `models\` y `runtime\` tienen que quedar junto al exe.
2. Instala ffmpeg y ffprobe en el PATH: `winget install Gyan.FFmpeg`. Abre la app después, para que vea el PATH nuevo.
3. Hace falta una GPU con DirectX 12. `nvidia-smi` es opcional (mejora la medida de VRAM libre).
4. Abre `camcuts.exe` (o `camcuts-bodas.exe`). Las preferencias se guardan en `%APPDATA%\CAMCUTS-rs\` (Bodas: `%APPDATA%\CAMCUTS-Bodas\`).
5. Por defecto la ventana guarda los resultados junto al material, en `<material>\CAMCUTS` (Bodas: `<material>\CAMCUTS Bodas`).
6. En DaVinci: *File ▸ Import ▸ Timeline…* y pega la ruta del `.otio`, que la ventana deja copiada. Los fps de la línea de tiempo deben coincidir con los del proyecto.

**Para actualizar**, vuelve a correr `armar_dist.ps1` (con `-Destino` si la app está en otra carpeta). Solo copia lo que cambió.

## Solución de problemas

### Sin GPU o sin DirectX 12

- **Síntoma:** según el equipo, los clips salen en `errores.txt` con un error al abrir un modelo (`abriendo …\models\….onnx: …`), o el análisis va muchísimo más lento de lo normal.
- **Si falta el runtime**, el análisis entero para con código 1 y `error: no encuentro onnxruntime.dll (ni en ORT_DYLIB_PATH, ni junto al ejecutable, ni en …)`. Comprueba que `runtime\` está junto al exe.
- **La decodificación** pasa sola a la CPU cuando NVDEC no da cuadros.
- **Para analizar sin GPU a propósito**, define `CAMCUTS_DISPOSITIVO=cpu` y `CAMCUTS_DECODE=cpu` antes de abrir la app o de lanzar `--analizar`. Así no se mira la VRAM ni salta la guarda de lentitud. Va a unos 2 s por muestra nueva: 9 clips de Bodas tardaron unos 29 min, frente a 52 s en la GPU.

### Poca VRAM

- **Síntoma:** la barra de estado dice en rojo `equipo: … GPU OCUPADA por otras apps: X GB libres (cierra DaVinci/Lightroom/Cutdown para ir más rápido); modo ligero: …`. O el análisis va mucho más lento de lo normal.
- **Causa:** con menos de 4.5 GB libres los modelos grandes no caben, y Windows pasa memoria de la GPU a la RAM.
- **Qué hacer:**
  - Nada obligatorio: la app ya pasó a modo ligero y sigue.
  - Para ir más rápido, cierra DaVinci, Lightroom o Cutdown y vuelve a analizar.
  - Las cachés hechas en modo ligero valen: no se repiten al tener memoria otra vez.
  - No fuerces `--gpu normal` ni `--workers` con poca VRAM: es la situación en la que el driver se reinicia.

### DaVinci abierto

- DaVinci ocupa unos 4 GB de los 8 de la RTX 5060. Con él abierto, `auto` suele dar modo ligero.
- Con el detector propio (el de por defecto) caben 2 clips y el análisis casi no cambia. En joyas con el clásico pedido, analiza el propio en su lugar. En Bodas, 1 clip con el modelo mediano.
- "Segundo plano" (encendido por defecto) baja la prioridad del análisis. En lo medido, la GPU queda por debajo del 60 % de media, para seguir editando.
- Para importar: *File ▸ Import ▸ Timeline…*. Los fps deben coincidir (`--timeline-fps` si hace falta).

### ffmpeg ausente

- **Síntoma:** todos los clips fallan y el motivo dice `no se pudo ejecutar ffprobe` (o ffmpeg). El visor y las miniaturas no muestran nada. En desarrollo, las 8 pruebas de `robustez` "pasan" sin probar nada.
- **Qué hacer:** `winget install Gyan.FFmpeg`, abre una terminal nueva y comprueba `ffmpeg -version` y `ffprobe -version`. Cierra y vuelve a abrir la app para que vea el PATH nuevo.

### Unidad desconectada

- **En la ventana:** la barra de estado dice en rojo `Material sin conexión: <carpeta>. Conecta la unidad; el proyecto lo conserva.`
  - Los clips ya analizados siguen en la lista (salen de la caché): se pueden revisar y exportar.
  - El visor usa el proxy si existe; si no, avisa de que no pudo leer el cuadro.
  - El proyecto conserva las rutas y no se reescribe hasta que cambies algo.
- **En la línea de comandos:**
  - Una carpeta de entrada que no existe no da error: no aporta clips y la línea de tiempo sale vacía. Revisa el número de clips en la salida.
  - Un archivo que no existe: si ya tiene caché, se usa la caché; si no, sale en `errores.txt`.
  - El respaldo se salta los clips que ya no están.
- **En DaVinci:** el OTIO apunta al material original. Para importarlo, la unidad tiene que estar conectada.

### CSV abierto en Excel

- **Causa:** Excel bloquea el CSV y no deja reemplazarlo.
- **En la línea de comandos:** sale `aviso: no se pudo escribir el CSV (¿abierto en Excel?): …`. El análisis termina con código 0, el OTIO se escribe y el CSV nuevo queda como `selects_<criterio>.csv.tmp`.
- **En la ventana:** al exportar sale "El CSV no se pudo reemplazar (¿está abierto en Excel?)". El OTIO sí se exporta.
- **Qué hacer:** cierra Excel y vuelve a exportar desde la ventana, o corre `--rescore` (segundos, sin GPU). También puedes renombrar el `.tmp`.
- Lo mismo pasa con el OTIO si otro programa lo tiene abierto: `aviso: no se pudo escribir la línea de tiempo`.

### Cachés viejas de Python

Una carpeta de resultados hecha por la app de Python se abre en Rust, pero algunas cachés se vuelven a analizar:

| Caso | Qué pasa | Qué hacer |
|---|---|---|
| Caché sin clave `detector` (todas las de Python) | Cuenta como YOLO-World. Con el detector propio (el de por defecto) se vuelve a analizar; la ventana la marca "Reanalizar · detector" | Para usarla tal cual: `--detector world` o "Detector: Clásico" en la ventana. O `--rescore`, que usa todas |
| Caché sin clave `edition` | Vale para joyas; Bodas la vuelve a analizar | Nada |
| Conversión de color distinta de la de `colores.json` | Se vuelve a analizar | Nada, o deja el color como estaba |
| JSON dañado o cortado | `aviso: …; se vuelve a analizar`. En `--rescore` se salta con un aviso | Nada |
| Proxy antiguo (CRF 28, de Python o de Rust antes del 2026-11-05) | Se rehace al reanalizar el clip (`sampler::proxy_vigente`) | Nada |

Los nombres de las cachés son los de Python (`<clip>_<hash de la ruta>.json`) y se comparan sin distinguir mayúsculas. Si el material cambió de letra de unidad o de carpeta, las cachés no se encuentran y todo se analiza de nuevo.

Los `__pycache__` de los scripts de Python no van a Git. Ejecuta los scripts con `python -B` (o `PYTHONDONTWRITEBYTECODE=1`) para no crearlos.

## Discrepancias encontradas

Revisado contra el código el 2026-11-06.

**INTERFACES.md frente al código:**
- **Detector por defecto.** "Detector propio (añadido)" dice que YOLO-World es el predeterminado y que sin `--detector` ni `CAMCUTS_DETECTOR` se usa `world`. El código usa el propio (`DetectorKind`, `#[default] Propio`). La sección "Own detector by default" ya lo corrige.
- **Clips en paralelo.** La misma sección dice "modo ligero = 1, `--background` = 1 salvo `--workers`". Con el propio, `hardware::parallel_clips` no limita en modo ligero y da 2 en segundo plano. Solo vale para YOLO-World.
- **Modo ligero.** "Poca VRAM con DaVinci abierto" dice "YOLO-World m, un clip" y que el fallo de x pasa a m. En joyas, con el modelo propio instalado, analiza el propio (`InferConfig::puede_usar_propio`). La sección siguiente de INTERFACES ya lo recoge.
- **VRAM por clip de YOLO-World.** "Poca VRAM" mide unos 3.3 GB por clip de YOLO-World x, pero `vram_workers` sigue contando 2.6 GB (`WORLD_GB_PER_WORKER`). Con 5.7 GB libres el código deja 2 clips; con 3.3 GB por clip harían falta 7.1 GB.
- **`runtime_cuda`.** INTERFACES la sitúa en `dist\runtime_cuda`, que el código busca, pero hoy no existe.

**Otros textos desactualizados:**
- `models/README.md` dice que el modelo mediano se usa con menos de 3 GB libres; el código usa 4.5 GB (`hardware::LOW_VRAM_GB`).
- La cabecera de `scripts/armar_dist.ps1` pone de ejemplo `-Destino D:\otra`, en la unidad del material, donde no se debe escribir.
