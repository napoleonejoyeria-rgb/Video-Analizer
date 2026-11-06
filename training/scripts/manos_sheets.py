"""Hojas de contacto de la carpeta "Videos Yo Manos Detalle" (ronda 4: anillos sueltos que salían "arete").

Para cada clip toma unos cuantos tiempos de muestra de la caché del detector desplegado
(`bench_perf/vista/carpeta_propio/cache`), saca el cuadro del proxy de 1080 px (solo lectura en D:),
lo pasa por `normalize_contrast` (lo que ve el análisis en un clip S-Log3) y dibuja encima las cajas de joya
de la caché con su etiqueta y confianza. Sirve para decidir a mano qué pieza es cada una antes de reetiquetar.

Uso: manos_sheets.py [--per 3] [--clips C9271,C9278] [--out review/manos]
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import subprocess
from pathlib import Path

import cv2
import numpy as np

from camcuts_metrics import normalize_contrast
from labels import TRAIN

CARPETA = Path(r"D:\CONTENIDO NAPO\2026\Noviembre\3\Videos Yo Manos Detalle")
PROXIES = CARPETA / "CAMCUTS" / "cache" / "proxies"
CACHE = TRAIN.parent / "bench_perf" / "vista" / "carpeta_propio" / "cache"
NO_WINDOW = 0x08000000  # nunca abrir consolas en la pantalla del usuario
BELOW_NORMAL = 0x00004000
# un color por clase para distinguir de un vistazo anillo / arete / topo
COLORS = {"anillo": (0, 220, 0), "arete": (0, 0, 255), "topo": (255, 0, 255), "dije": (0, 200, 255),
          "pulsera": (255, 160, 0), "collar": (255, 255, 0), "cadena": (128, 128, 255)}


def load_caches() -> dict[str, dict]:
    """Caché del detector desplegado por clip (clave: "C9271")."""
    out = {}
    for f in glob.glob(str(CACHE / "*.json")):
        j = json.loads(Path(f).read_text(encoding="utf-8"))
        out[os.path.basename(j["path"])[:5]] = j
    return out


def proxy_of(clip: str) -> Path:
    """Proxy terminado del clip (se ignoran los `.part.mp4` a medio escribir)."""
    c = [p for p in PROXIES.glob(f"{clip}_*.mp4") if not p.name.endswith(".part.mp4")]
    return c[0]


def frame_at(video: Path, t: float, w: int, h: int) -> np.ndarray:
    """Cuadro exacto en el segundo t, escalado a w×h (BGR), leído por una tubería sin tocar el disco."""
    r = subprocess.run(["ffmpeg", "-v", "error", "-ss", f"{t:.3f}", "-i", str(video), "-frames:v", "1",
                        "-vf", f"scale={w}:{h}:flags=area", "-f", "rawvideo", "-pix_fmt", "bgr24", "-"],
                       capture_output=True, creationflags=NO_WINDOW | BELOW_NORMAL, timeout=120)
    return np.frombuffer(r.stdout, np.uint8)[: w * h * 3].reshape(h, w, 3).copy()


def pick_samples(cache: dict, n: int) -> list[dict]:
    """n muestras con joya repartidas por el clip (cuantiles), para ver todas las piezas del clip."""
    js = [s for s in cache["samples"] if s.get("jewels")]
    if not js:
        return []
    idx = sorted({int(round(q * (len(js) - 1))) for q in np.linspace(0.1, 0.9, n)})
    return [js[i] for i in idx]


def draw(img: np.ndarray, s: dict) -> np.ndarray:
    """Cajas de joya de la muestra con etiqueta y confianza (en coordenadas del cuadro de 960 px)."""
    out = img.copy()
    for b in s.get("jewels", []):
        x0, y0, x1, y1 = [int(v) for v in b["box"]]
        c = COLORS.get(b["label"], (255, 255, 255))
        cv2.rectangle(out, (x0, y0), (x1, y1), c, 2)
        cv2.putText(out, f"{b['label'][:3]} {b['conf']:.2f}", (x0 + 2, max(14, y0 - 4)), cv2.FONT_HERSHEY_SIMPLEX,
                    0.55, c, 2, cv2.LINE_AA)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--per", type=int, default=3)
    ap.add_argument("--clips", default="")
    ap.add_argument("--out", default="review/manos")
    ap.add_argument("--rows", type=int, default=3, help="filas de clips por hoja (2 clips por fila)")
    a = ap.parse_args()
    C = load_caches()
    clips = a.clips.split(",") if a.clips else sorted(C)
    out = TRAIN / a.out
    out.mkdir(parents=True, exist_ok=True)
    tw, th = 270, 480  # miniatura: mitad del cuadro de análisis (540×960)
    blocks = []
    for clip in clips:
        cache = C[clip]
        W, H = (540, 960) if cache["height"] > cache["width"] else (960, 540)
        tiles = []
        for s in pick_samples(cache, a.per):
            img = normalize_contrast(frame_at(proxy_of(clip), s["t"], W, H))
            t = cv2.resize(draw(img, s), (tw, th), interpolation=cv2.INTER_AREA)
            cv2.putText(t, f"{clip} t={s['t']:.1f}", (4, th - 8), cv2.FONT_HERSHEY_SIMPLEX, 0.55, (255, 255, 255), 2, cv2.LINE_AA)
            tiles.append(t)
        while len(tiles) < a.per:
            tiles.append(np.zeros((th, tw, 3), np.uint8))
        blocks.append(np.hstack(tiles))
        print(clip, "listo", flush=True)
    per_sheet = 2 * a.rows
    for k in range(0, len(blocks), per_sheet):
        group = blocks[k:k + per_sheet]
        while len(group) % 2:
            group.append(np.zeros_like(blocks[0]))
        sep = np.full((group[0].shape[0], 8, 3), 40, np.uint8)
        rows = [np.hstack([group[i], sep, group[i + 1]]) for i in range(0, len(group), 2)]
        sheet = np.vstack(rows)
        name = out / f"hoja_{k // per_sheet + 1:02d}.jpg"
        cv2.imwrite(str(name), sheet, [cv2.IMWRITE_JPEG_QUALITY, 88])
        print("escrita", name, sheet.shape, flush=True)


if __name__ == "__main__":
    main()
