"""Imágenes de revisión: un cuadro con sus cajas numeradas y una cuadrícula de coordenadas, más un
recorte ampliado de cada caja (sacado de la imagen de 1920 px) para ver joyas pequeñas.

Las coordenadas de la cuadrícula son las del cuadro de 960 px (las de las etiquetas).
"""
from __future__ import annotations

import cv2
import numpy as np

from labels import SHORT, TRAIN

COLORS = {"anillo": (0, 200, 255), "collar": (255, 120, 0), "cadena": (255, 0, 200), "arete": (0, 255, 0),
          "topo": (0, 255, 255), "pulsera": (60, 60, 255), "dije": (255, 255, 0), "joya": (200, 200, 200),
          "persona": (128, 128, 128), "cara": (90, 90, 90)}


def load_pair(fid: str, flat: bool) -> tuple[np.ndarray, np.ndarray]:
    """(cuadro de 960, imagen de 1920), con el contraste estirado si el cuadro es plano (como el detector)."""
    from camcuts_metrics import normalize_contrast

    im9 = cv2.imread(str(TRAIN / "data" / "img960" / f"{fid}.jpg"))
    big = cv2.imread(str(TRAIN / "data" / "img1920" / f"{fid}.jpg"))
    if flat:
        im9, big = normalize_contrast(im9), normalize_contrast(big)
    return im9, big


def tile(big: np.ndarray, box, scale: float, label: str, side: int = 150) -> np.ndarray:
    """Recorte ampliado de una caja (25 % de margen) con la caja dibujada."""
    x0, y0, x1, y1 = [v * scale for v in box]
    bw, bh = max(x1 - x0, 4), max(y1 - y0, 4)
    m = max(bw, bh) * 0.25 + 6
    H, W = big.shape[:2]
    cx0, cy0, cx1, cy1 = int(max(0, x0 - m)), int(max(0, y0 - m)), int(min(W, x1 + m)), int(min(H, y1 + m))
    crop = big[cy0:cy1, cx0:cx1].copy()
    if crop.size == 0:
        return np.zeros((side, side, 3), np.uint8)
    cv2.rectangle(crop, (int(x0 - cx0), int(y0 - cy0)), (int(x1 - cx0), int(y1 - cy0)), COLORS.get(label, (255, 255, 255)),
                  max(1, int(min(crop.shape[:2]) / 150)))
    s = min(side / crop.shape[1], side / crop.shape[0])
    crop = cv2.resize(crop, (max(1, int(crop.shape[1] * s)), max(1, int(crop.shape[0] * s))),
                      interpolation=cv2.INTER_AREA if s < 1 else cv2.INTER_CUBIC)
    t = np.full((side, side, 3), 30, np.uint8)
    t[:crop.shape[0], :crop.shape[1]] = crop
    return t


def render_frame(rec: dict, boxes: list[dict], out: str, title: str = "", height: int = 720, cols: int = 3) -> None:
    """Cuadro con cajas numeradas (1..n) + recortes. `boxes`: {box, label, weak?, conf?}."""
    im9, big = load_pair(rec["id"], rec.get("flat", False))
    h9, w9 = im9.shape[:2]
    s = height / h9
    view = cv2.resize(im9, (int(w9 * s), height), interpolation=cv2.INTER_AREA)
    # cuadrícula cada 100 px del cuadro de 960
    for g in range(100, max(w9, h9), 100):
        if g < w9:
            x = int(g * s)
            cv2.line(view, (x, 0), (x, 8), (255, 255, 255), 1)
            cv2.line(view, (x, height - 8), (x, height - 1), (255, 255, 255), 1)
            cv2.putText(view, str(g), (x + 2, 18), cv2.FONT_HERSHEY_SIMPLEX, 0.38, (255, 255, 255), 1)
        if g < h9:
            y = int(g * s)
            cv2.line(view, (0, y), (8, y), (255, 255, 255), 1)
            cv2.line(view, (view.shape[1] - 8, y), (view.shape[1] - 1, y), (255, 255, 255), 1)
            cv2.putText(view, str(g), (10, y + 4), cv2.FONT_HERSHEY_SIMPLEX, 0.38, (255, 255, 255), 1)
    tiles = []
    kb = big.shape[1] / w9
    for i, b in enumerate(boxes, 1):
        col = COLORS.get(b["label"], (255, 255, 255))
        x0, y0, x1, y1 = [int(v * s) for v in b["box"]]
        if b.get("weak"):
            # candidato débil: línea discontinua fina
            for xx in range(x0, x1, 8):
                cv2.line(view, (xx, y0), (min(xx + 4, x1), y0), col, 1)
                cv2.line(view, (xx, y1), (min(xx + 4, x1), y1), col, 1)
            for yy in range(y0, y1, 8):
                cv2.line(view, (x0, yy), (x0, min(yy + 4, y1)), col, 1)
                cv2.line(view, (x1, yy), (x1, min(yy + 4, y1)), col, 1)
        else:
            cv2.rectangle(view, (x0, y0), (x1, y1), col, 2)
        tag = str(i)
        cv2.putText(view, tag, (x0 + 2, max(12, y0 + 14)), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 0, 0), 3)
        cv2.putText(view, tag, (x0 + 2, max(12, y0 + 14)), cv2.FONT_HERSHEY_SIMPLEX, 0.5, col, 1)
        if i <= 18:
            t = tile(big, b["box"], kb, b["label"])
            txt = f"{i} {SHORT.get(b['label'], b['label'][:2])}" + ("?" if b.get("weak") else "")
            if "conf" in b:
                txt += f" {b['conf']:.2f}"
            cv2.putText(t, txt, (3, 14), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 0, 0), 3)
            cv2.putText(t, txt, (3, 14), cv2.FONT_HERSHEY_SIMPLEX, 0.45, col, 1)
            tiles.append(t)
    if title:
        cv2.putText(view, title, (30, height - 12), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 0, 0), 3)
        cv2.putText(view, title, (30, height - 12), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 255, 255), 1)
    side = 150
    rows = max(1, (len(tiles) + cols - 1) // cols)
    panel = np.full((max(height, rows * side), cols * side, 3), 20, np.uint8)
    for k, t in enumerate(tiles):
        r, c = divmod(k, cols)
        panel[r * side:(r + 1) * side, c * side:(c + 1) * side] = t
    if panel.shape[0] > height:
        view = np.vstack([view, np.zeros((panel.shape[0] - height, view.shape[1], 3), np.uint8)])
    cv2.imwrite(out, np.hstack([view, panel]), [cv2.IMWRITE_JPEG_QUALITY, 85])
