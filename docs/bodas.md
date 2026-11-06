# CAMCUTS Bodas: resumen para desarrolladores

`camcuts-bodas.exe` es la edición Bodas de CAMCUTS. Es el mismo código que `camcuts.exe`, la app de Napoleone Joyas, con la edición fijada al arrancar. Diseño "Salvia" (verde clarito, modo claro y oscuro).

La documentación de Bodas va **dentro de su paquete** y no se repite aquí:

- [empaque/bodas/README.md](../empaque/bodas/README.md): la guía de uso.
- [empaque/bodas/CLAUDE.md](../empaque/bodas/CLAUDE.md): cómo funciona el análisis de bodas, los modelos, cómo compilar, probar y empaquetar, y lo probado.

`scripts/armar_dist.ps1` copia esos dos archivos al paquete. Por eso en ellos **no puede aparecer la palabra "Napoleone"**. Este archivo sí puede nombrarla: no va en el paquete.

La ventana compartida está documentada en [ventana.md](ventana.md). El contrato entre crates, en [INTERFACES.md](../INTERFACES.md), sección "Edición Bodas (añadido)".

## Cómo se elige la edición

- `crates/camcuts-gui/src/main_bodas.rs` llama a `camcuts_core::edicion::fijar(Edicion::Bodas)` antes que nada. El análisis corre en un hijo `camcuts-bodas.exe --analizar`, que también arranca en Bodas.
- Sin `fijar`, manda la variable `CAMCUTS_EDICION=bodas`; si no está, es joyas (`edicion::actual`).
- En la ventana, `motor::bodas()` dice la edición. `App::dibujar` pasa a `App::dibujar_bodas` cuando es Bodas.

## Qué comparte con joyas

| Qué | Dónde |
|---|---|
| Estado y lógica de la ventana: material, análisis, proyecto, color por clip, visor, exportar | `app.rs` (`App`) |
| Análisis como proceso hijo, Job Object, Detener, avance por clip | `analisis.rs`, `camcuts_core::cli` |
| Visor, miniaturas, color automático por perfil de cámara, proxies | `video.rs`, `motor.rs` |
| Proyectos `.camcuts`, autoguardado, solo lectura, material sin conexión | `proyecto.rs`, `app.rs` |
| Atajos de teclado | `App::atajos` (`paneles.rs`); Bodas añade Ctrl+L (claro/oscuro) |
| Fuentes (DM Sans, DM Serif Display) | `tema.rs` |
| Muestreo, medida de nitidez sobre la silueta, exportación OTIO/CSV | `camcuts-core` |
| Modo ligero, tope de VRAM, motor de GPU | `camcuts_core::hardware`, `camcuts-infer` |

## Qué no comparte

| Qué | Joyas | Bodas |
|---|---|---|
| Dibujo | `paneles.rs` (Marfil) | `bodas_ui.rs` |
| Tema | `tema.rs`, colores fijos | `bodas_tema.rs`, claro u oscuro (`App::modo_claro`), un color por momento |
| Detector | Propio (por defecto) o Clásico (YOLO-World) | Siempre su YOLO-World de boda (`yolo_world_bodas_x` / `_m`): `Args::detector_kind` devuelve `World` |
| Selector de detector, `detector` y `detector_v` en el proyecto | Sí | No (`App::cambiar_detector`, `App::args_detector`, `App::estado_json`) |
| Respaldo automático con el clásico | Sí | No: `detector::necesita_respaldo` devuelve `false` |
| Detector compartido entre hilos (`Compartido`) | Con el propio | No: un detector por hilo |
| Modo ligero (menos de 4.5 GB de VRAM libre) | Con el clásico pedido, analiza el propio | `yolo_world_bodas_m`, 1 clip |
| Clips en paralelo | Propio: hasta 4, hasta 2 en segundo plano | Como mucho 2 (`WORLD_MAX_WORKERS`), 1 en segundo plano |
| Cachés válidas | Sin marca `edition` o `joyas`, y del mismo detector | Con `edition: "bodas"`; el detector no cuenta |
| Criterios | `joyas`, `social`, `todo` (Equilibrado, Redes, Permisivo) | `bodas`, `social`, `todo` (Equilibrado, Highlight, Permisivo) |
| Pesos | 6 (`motor::PESOS`) | 7, con "Cámara estable" (`motor::PESOS_BODAS`) |
| Tipo por clip y filtro de tipos | Joyas (clave `joyas` en el proyecto) | Momentos (clave `momentos`) |
| Planos | Producto, Modelo, Hablando | Hablando, Primer plano, Plano medio, Plano general, Detalle |
| Ctrl+5 (`paneles.mapa`) | Mapa | Panel Foco |
| Carpeta de resultados | `<material>\CAMCUTS` | `<material>\CAMCUTS Bodas` |
| Exportación de la ventana | `CAMCUTS_<criterio>_<modo>.otio` | `CAMCUTS_Bodas_<criterio>_<modo>.otio` |
| Selects de la CLI | `selects_joyas`, `selects_social`… | `selects_bodas`, `selects_bodas_social`… |
| Preferencias | `%APPDATA%\CAMCUTS-rs` | `%APPDATA%\CAMCUTS-Bodas` |
| Campo `app` del proyecto | `CAMCUTS` | `CAMCUTS Bodas` |
| Alto inicial de la línea de tiempo | 200 px | 250 px |

Cada edición abre los proyectos de la otra en solo lectura y no carga sus cachés. Ver [ventana.md](ventana.md#modo-solo-lectura).

## La regla: Joyas no debe cambiar

Bodas se añadió sin tocar el comportamiento de joyas. Al cambiar código compartido:

1. **Una comprobación de una línea.** Las funciones compartidas de `scoring`, `analyse`, `export` y `cli` empiezan con `if crate::edicion::es_bodas() { … }` y pasan el trabajo a `camcuts_core::bodas`. El camino de joyas queda igual.
2. **Paridad con Python.** `cargo test -p camcuts-core` debe seguir pasando.
3. **Lo nuevo de joyas, fuera de Bodas a propósito.** El detector propio, su respaldo, el detector compartido y el propio en modo ligero excluyen Bodas de forma explícita: `Args::detector_kind`, `InferConfig::puede_usar_propio`, `detector::cache_vigente`, `detector::necesita_respaldo`, `OnnxDetector::kind`. Haz lo mismo con lo próximo.
4. **En la ventana**, separa con `motor::bodas()`. Las pruebas de `app.rs` hacen lo mismo para las comprobaciones de una sola edición.
5. **Capturas de las dos.** Si tocas la ventana compartida, corre `capturas` y `capturas_bodas` y compara `design/snapshots/`. Los `bodas_*.png` no van a Git, porque muestran personas: compáralos en local. Cómo, en [ventana.md](ventana.md#capturas-sin-pantalla).
6. **Sin "Napoleone" en Bodas.** Los dos binarios enlazan la misma librería: cualquier texto en código compartido acaba también en `camcuts-bodas.exe`. Deja el nombre de la marca fuera de los textos del programa (en los comentarios no llega al binario). `scripts/armar_dist.ps1` no empaqueta Bodas si el exe lo contiene. Tampoco puede aparecer en `empaque/bodas/`.

## Correcciones en `empaque/bodas` (2026-11-06)

Se revisaron los dos archivos contra el código. Se corrigió solo lo desactualizado:

- **README.md:** avance por clip en la lista; segundo plano y clips en paralelo (hasta 2); color automático por perfil en el visor; etiquetas del visor (solo el sujeto principal, el resto al pasar el ratón; interruptores Caras y Color); tramos aprobados con el color de su momento; atajo Inicio; proyectos de joyas en solo lectura.
- **CLAUDE.md:** qué es solo de joyas (detector propio, respaldo, detector compartido, propio en modo ligero); clips en paralelo de Bodas; cuándo entra `yolo_world_bodas_m`; scripts tras clonar (`configurar_compilacion.ps1`, `verificar_modelos.ps1`); `bench_*` fuera del repositorio; CPU sin modo ligero (`hardware::en_cpu`); empaquetar con `scripts/armar_dist.ps1`.

La copia de `dist\CAMCUTS Bodas\` sigue con la versión anterior hasta el próximo `armar_dist.ps1 -Edicion bodas`.

## Discrepancias encontradas

Manda el código.

1. **Comentario de `bodas_ui.rs` (cabecera del módulo).** Dice que Foco tiene "tres medidores (nitidez, exposición, cámara estable)". `App::panel_foco` dibuja cuatro: también la puntuación. El README ya decía cuatro.
2. **`empaque/bodas/CLAUDE.md`, sección "Probado".** No dice "Napoleone", pero cita la ruta del material de prueba, cuyo nombre de carpeta abrevia la marca. No se cambió porque no está desactualizada. Conviene decidir si se deja.
