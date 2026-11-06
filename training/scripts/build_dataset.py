"""Construye el conjunto de entrenamiento en formato YOLO a partir de los cuadros revisados a mano.

- Entrenamiento: cuadros "oro" y "more" revisados (sin los marcados `unsure`) del reparto train.
- Validación (solo para seguir el entrenamiento): los revisados del reparto val.
- La evaluación (carpetas apartadas) NO entra aquí.
- Joyas: las cajas revisadas. Personas y caras: las de YOLO-World del maestro (conf ≥ 0.35), sin revisar.
- Imagen: el cuadro de 960 px tal como lo ve el detector (contraste estirado si es plano/LOG).
- Aumentos fuera de línea (además de los de Ultralytics: mosaico, volteo, color, escala):
  * LOG: se aplana el tono y el color como un S-Log, y la mitad de las copias pasan además por
    `normalize_contrast` (lo que hace el análisis al ver un clip plano);
  * desenfoque: gaussiano o de movimiento (las joyas desenfocadas también deben detectarse: la nitidez
    la mide CAMCUTS aparte);
  * vertical: recorte 9:16 de cuadros horizontales alrededor de las joyas.

Uso: build_dataset.py --out dataset_r1 [--sets gold,more]
"""
from __future__ import annotations

import argparse
import json
import random
import shutil

import cv2
import numpy as np

from camcuts_metrics import normalize_contrast
from decisions import gt
from labels import CLASSES, TRAIN, load_teacher, people


def logify(img: np.ndarray, rnd: random.Random) -> np.ndarray:
    """Imita material LOG sin corregir: tonos comprimidos en una franja estrecha y color apagado."""
    x = img.astype(np.float32) / 255.0
    lo, hi = rnd.uniform(0.08, 0.2), rnd.uniform(0.62, 0.8)
    x = lo + (hi - lo) * np.power(x, rnd.uniform(0.6, 0.85))
    gray = x.mean(axis=2, keepdims=True)
    x = gray + (x - gray) * rnd.uniform(0.45, 0.7)
    return np.clip(x * 255, 0, 255).astype(np.uint8)


def blur(img: np.ndarray, rnd: random.Random) -> np.ndarray:
    """Desenfoque de foco (gaussiano) o de movimiento (línea con ángulo al azar)."""
    if rnd.random() < 0.5:
        s = rnd.uniform(1.2, 3.2)
        return cv2.GaussianBlur(img, (0, 0), s)
    n = rnd.choice([7, 9, 11, 13, 15, 19])
    k = np.zeros((n, n), np.float32)
    k[n // 2, :] = 1.0
    m = cv2.getRotationMatrix2D((n / 2 - 0.5, n / 2 - 0.5), rnd.uniform(0, 180), 1.0)
    k = cv2.warpAffine(k, m, (n, n))
    k /= max(k.sum(), 1e-6)
    return cv2.filter2D(img, -1, k)


def yolo_lines(boxes: list[tuple[str, list[float]]], w: int, h: int) -> list[str]:
    out = []
    for lab, (x0, y0, x1, y1) in boxes:
        x0, x1 = max(0.0, min(x0, x1)), min(float(w), max(x0, x1))
        y0, y1 = max(0.0, min(y0, y1)), min(float(h), max(y0, y1))
        if x1 - x0 < 2 or y1 - y0 < 2:
            continue
        out.append(f"{CLASSES.index(lab)} {(x0 + x1) / 2 / w:.6f} {(y0 + y1) / 2 / h:.6f} {(x1 - x0) / w:.6f} {(y1 - y0) / h:.6f}")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--sets", default="gold,more")
    ap.add_argument("--aug", type=float, default=0.35)
    ap.add_argument("--dup", default="", help="clases cuyos cuadros se repiten una vez más (sobremuestreo)")
    ap.add_argument("--no_audit", action="store_true",
                    help="sin las correcciones de la auditoría de piezas largas (las etiquetas con que se entrenó r2)")
    a = ap.parse_args()
    if a.no_audit:
        # `gt()` llama a `decisions.apply_audit`: se anula para reproducir las cajas de la ronda 2
        import decisions
        decisions.apply_audit = lambda res: 0
    rnd = random.Random(5)
    T = load_teacher()
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    out = TRAIN / a.out
    if out.exists():
        shutil.rmtree(out)
    for s in ("train", "val"):
        (out / "images" / s).mkdir(parents=True)
        (out / "labels" / s).mkdir(parents=True)
    frames = {}
    for name in a.sets.split(","):
        for fid, g in gt(name).items():
            if not g["unsure"] and split.get(fid) in ("train", "val"):
                frames[fid] = g
    counts = {"train": 0, "val": 0}
    nbox = {c: 0 for c in CLASSES}
    for fid, g in sorted(frames.items()):
        r = T[fid]
        img = cv2.imread(str(TRAIN / "data" / "img960" / f"{fid}.jpg"))
        if img is None:
            continue
        h, w = img.shape[:2]
        seen = normalize_contrast(img) if r["flat"] else img
        boxes = [(b["label"], b["box"]) for b in g["boxes"]]
        # personas ≥ 0.35; caras ≥ 0.25 (YOLO-World casi nunca pasa de 0.3 en caras)
        boxes += [(p["label"], p["box"]) for p in people(r, 0.25) if p["label"] == "cara" or p["conf"] >= 0.35]
        for lab, _ in boxes:
            nbox[lab] += 1
        sp = split[fid]

        def save(name, im, bx):
            cv2.imwrite(str(out / "images" / sp / f"{name}.jpg"), im, [cv2.IMWRITE_JPEG_QUALITY, 93])
            (out / "labels" / sp / f"{name}.txt").write_text("\n".join(yolo_lines(bx, im.shape[1], im.shape[0])), encoding="utf-8")
            counts[sp] += 1

        save(fid, seen, boxes)
        if sp != "train":
            continue
        # sobremuestreo de clases que se resienten al añadir muchos cuadros de otras (aretes, topos)
        if a.dup and any(lab in a.dup.split(",") for lab, _ in boxes):
            save(f"{fid}_dup", cv2.flip(seen, 1), [(lab, [w - b[2], b[1], w - b[0], b[3]]) for lab, b in boxes])
        # LOG / plano
        if not r["flat"] and rnd.random() < a.aug:
            flat = logify(img, rnd)
            save(f"{fid}_log", normalize_contrast(flat) if rnd.random() < 0.5 else flat, boxes)
        elif r["flat"] and rnd.random() < a.aug:
            save(f"{fid}_raw", img, boxes)  # el clip LOG sin estirar, por si la detección de plano falla
        # desenfoque
        if rnd.random() < a.aug:
            save(f"{fid}_blur", blur(seen, rnd), boxes)
        # recorte vertical 9:16 de un cuadro horizontal, centrado en una joya al azar
        jewels = [b for b in g["boxes"]]
        if w > h and jewels and rnd.random() < 0.6:
            cw = int(round(h * 9 / 16))
            b = rnd.choice(jewels)["box"]
            cx = (b[0] + b[2]) / 2 + rnd.uniform(-0.25, 0.25) * cw
            x0 = int(min(max(0, cx - cw / 2), w - cw))
            crop = seen[:, x0:x0 + cw]
            kept = []
            for lab, (bx0, by0, bx1, by1) in boxes:
                ix0, ix1 = max(bx0, x0), min(bx1, x0 + cw)
                if ix1 - ix0 >= 0.6 * (bx1 - bx0):
                    kept.append((lab, [ix0 - x0, by0, ix1 - x0, by1]))
            # una caja cortada a más de la mitad no se puede etiquetar bien: entonces no se guarda
            cut = any(max(bx0, x0) < min(bx1, x0 + cw) and min(bx1, x0 + cw) - max(bx0, x0) < 0.6 * (bx1 - bx0)
                      for _, (bx0, _y0, bx1, _y1) in boxes)
            if not cut:
                save(f"{fid}_vert", cv2.resize(crop, (540, 960), interpolation=cv2.INTER_LINEAR),
                     [(lab, [bb[0] * 540 / cw, bb[1] * 960 / h, bb[2] * 540 / cw, bb[3] * 960 / h]) for lab, bb in kept])
    (out / "data.yaml").write_text(
        f"path: {out.as_posix()}\ntrain: images/train\nval: images/val\nnames:\n" +
        "".join(f"  {i}: {c}\n" for i, c in enumerate(CLASSES)), encoding="utf-8")
    print("imágenes", counts, "cuadros revisados", len(frames))
    print("cajas (sin aumentos)", nbox)


if __name__ == "__main__":
    main()
