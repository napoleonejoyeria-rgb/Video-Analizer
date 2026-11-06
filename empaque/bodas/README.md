# CAMCUTS Bodas

CAMCUTS Bodas revisa todo el material de una boda y te deja los **tramos utilizables** en DaVinci Resolve. Elige los tramos donde las **caras se ven nítidas**, bien expuestas y con la **cámara estable**. Además, le pone a cada tramo su **momento**:

- preparativos, detalles, lugar
- ceremonia, votos, anillos, beso
- cóctel, retratos
- entrada, primer baile, discursos, banquete, pastel, fiesta, salida

## Empezar

1. Abre **`camcuts-bodas.exe`**.
2. En **Proyecto ▸ + Carpeta…**, elige la carpeta con el material de la boda. También puedes usar el **+** de la lista de clips.
3. Pulsa **Analizar**. Los clips ya analizados se saltan. La lista muestra el avance de cada clip.
   - **Analizar en segundo plano** (en **Proyecto**, encendido por defecto) analiza un clip a la vez y con prioridad baja, para que sigas editando. Apagado, analiza hasta 2 clips a la vez si la GPU tiene memoria libre.
   - Con poca memoria de GPU libre (por ejemplo, con DaVinci abierto), usa un detector más ligero y un clip a la vez.
4. Revisa el material:
   - Mira cada clip en el visor.
   - En la línea de tiempo **Momentos**, cada momento de la boda tiene su carril de color.
   - Los tramos aprobados van en la fila **Aprobados**, cada uno con el color de su momento. En el visor, la marca verde **APROBADO** dice que estás dentro de uno.
5. Pulsa **Exportar**. En DaVinci, ve a *File ▸ Import ▸ Timeline…* y pega la ruta, que queda copiada.

## La ventana

- **Barra de arriba:** nombre del material, estado del análisis, botones para mostrar u ocultar cada panel, modo claro u oscuro, **Proyecto** (también el material y las opciones del análisis), **Analizar** y **Exportar**.
- **Clips** (Ctrl+1):
  - Cada clip con su miniatura, duración, número de tramos, puntuación y los puntos de color de sus momentos.
  - Mientras se analiza, en lugar de la puntuación sale el avance del clip: `0%` si está en cola, `error` si falló.
  - Ctrl+clic o Mayús+clic elige varios clips.
- **Color** (Ctrl+3): la conversión de color del clip, de los clips elegidos o de todos (por ejemplo, S-Log3 → Rec.709 o una LUT `.cube`). **Detectar** (la lupa) lee el perfil de la cámara.
  - Con **Automático**, el visor ya aplica el perfil que dice la cámara. La píldora de color, bajo el cuadro, dice cuál: por ejemplo, `S-Log3 → 709 (auto)`.
- **Visor:**
  - El cuadro se ve limpio: esquinas en las caras y, solo en el sujeto principal, su etiqueta y su confianza. La de otra caja sale al pasar el ratón por encima.
  - Arriba van las marcas de **APROBADO** y del momento de ese instante.
  - Los interruptores **Caras** y **Color** ocultan las esquinas o la conversión de color.
  - Los controles van debajo del cuadro, nunca encima.
  - **Momento: automático ▾** fija el momento de un clip si la app se equivoca.
- **Foco** (Ctrl+5):
  - La cara principal ampliada y el momento de ese instante.
  - Cuatro medidores: nitidez, exposición, cámara estable y puntuación.
- **Criterio** (Ctrl+2):
  - **Criterios:** Equilibrado, Highlight (cortes de hasta 4 s, los 3 mejores por clip) o Permisivo.
  - **Pesos** y ajustes de los tramos.
  - **Filtros:** por **momentos** (chips de color) y por **planos** (hablando, primer plano, plano medio, plano general, detalle).
  - **Exportación:** fps de la línea de tiempo y modo.
- **Momentos** (Ctrl+4):
  - La línea de tiempo de todo el material: un carril por momento y los tramos aprobados.
  - Rueda: zoom. Mayús+rueda: desplazar. Arrastra el borde de arriba para cambiar su alto.

**Atajos:**

| Tecla | Acción |
|---|---|
| Espacio | Reproducir o pausar |
| ← / → | ±2 s |
| Inicio | Al principio del clip |
| N | Siguiente tramo |
| ↑ / ↓ | Clip anterior o siguiente |
| Ctrl+E | Exportar |
| Ctrl+L | Modo claro / oscuro |
| Ctrl+O | Abrir proyecto |
| Ctrl+N | Proyecto nuevo |
| Ctrl+Mayús+S | Guardar como |

## Qué premia y qué descarta

**Premia:**
- Caras nítidas. La nitidez se mide sobre la silueta de la cara o del sujeto.
- Buena exposición en caras y vestido.
- Cámara estable. Un gimbal o un slider suave cuentan como estables.
- Sujetos de buen tamaño en el cuadro.

**Penaliza o descarta:**
- Desenfoque y barrido.
- Cámara temblorosa.
- Caras quemadas u oscuras.
- Gente de espaldas.
- Caras cortadas por el borde.

**Planos sin personas:** en *lugar* y *detalles* también se valora el cuadro entero.

## Exportar

Hay tres modos de exportación:

| Modo | Qué hace |
|---|---|
| **V2 aprobados · V1 el resto** | El clip entero, con los aprobados encima |
| **Solo aprobados** | Solo los tramos aprobados, uno tras otro |
| **Marcadores** | El clip entero, con un marcador por tramo |

**Cada tramo lleva** un marcador con el nombre de su momento y la puntuación. Verde es muy bueno, amarillo bueno y rojo justo.

**Archivos:** junto al `.otio` se escribe un `.csv` con clip, tiempos, puntuación, momento, sujeto y plano.

## Proyectos

Un archivo **`.camcuts`** guarda el material, la carpeta de resultados, el criterio, los filtros, el color y el momento de cada clip, y cómo dejaste la ventana. El proyecto se guarda solo mientras trabajas.

Si la unidad del material no está conectada al abrir el proyecto:
- El proyecto conserva las rutas.
- El estado avisa de que el material está sin conexión.
- El archivo no se toca hasta que cambies algo.

Un proyecto de la edición de joyas de CAMCUTS se abre solo para mirar: no se reescribe. Para trabajar con él aquí, usa **Guardar como**, que guarda una copia de CAMCUTS Bodas.

## Requisitos

- Windows 10/11.
- Una GPU con DirectX 12 (se usa DirectML). Sin GPU funciona en la CPU, más despacio.
- ffmpeg y ffprobe en el PATH.
- Junto al ejecutable, las carpetas `models\` y `runtime\` de esta carpeta.
