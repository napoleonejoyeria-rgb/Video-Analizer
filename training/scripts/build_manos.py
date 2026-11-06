"""Ronda 4 (r6): cuadros nuevos de "Videos Yo Manos Detalle" para que los anillos sueltos (sobre la mesa,
en tela o en los dedos) salgan "anillo" y no "arete", sin perder los topos en la oreja de maniquí.

No se etiqueta a mano caja por caja:
- las cajas de joya son las del detector desplegado (r2, el mismo `best.pt` que `models/jewel_det.onnx`)
  sobre el mismo cuadro, y la clase se corrige por clip tras revisarlo en hojas de contacto
  (`manos_sheets.py`): en estos clips todas las piezas de un clip son del mismo tipo;
- personas y caras también salen de r2 (persona ≥ 0.35, cara ≥ 0.25), para no enseñarle a olvidarlas.

Cuadro: el original de D: (solo lectura) en el tiempo exacto de una muestra de la caché, a 540×960 con
INTER_AREA y `normalize_contrast` (S-Log3: lo que ve el análisis con color "auto").

Pasos:
  build_manos.py extraer   -> data/manos960/*.jpg (cuadro LOG sin estirar) y data/manos_r6.json
  build_manos.py hojas     -> review/manos_r6/hoja_*.jpg con las etiquetas finales, para revisarlas
  build_manos.py escribir --out dataset_r6  -> añade los cuadros (menos los de review/manos_r6_drop.txt)
                                               y sus aumentos al reparto train del conjunto
Los clips de APARTADOS no entran: sirven para medir si el arreglo generaliza.
"""
from __future__ import annotations

import argparse
import json
import random
import subprocess
from pathlib import Path

import cv2
import numpy as np

from build_dataset import blur, yolo_lines
from camcuts_metrics import is_flat, normalize_contrast
from evaluate import cross_class
from labels import CLASSES, JEWEL7, TRAIN, iou, overlap_small
from manos_sheets import BELOW_NORMAL, CARPETA, COLORS, NO_WINDOW, load_caches

R2 = TRAIN / "runs" / "r2" / "weights" / "best.pt"
IMG = TRAIN / "data" / "manos960"
MANIFEST = TRAIN / "data" / "manos_r6.json"
DROP = TRAIN / "review" / "manos_r6_drop.txt"
JEWEL_CONF = 0.30  # algo por debajo del umbral de trabajo (0.4) para no dejar piezas reales sin caja

# clip -> (clase de todas sus piezas, cuadros a sacar). Revisado en review/manos/hoja_*.jpg.
TRAIN_CLIPS = {
    # anillos sueltos: montón de anillos en la mesa (30 % "arete"), anillo clavo (8 % "arete"), varios en tela
    "C9279": ("anillo", 24), "C9277": ("anillo", 12), "C9263": ("anillo", 8),
    # anillos sueltos que ya salen bien (para no desplazar la frontera en el otro sentido)
    "C9264": ("anillo", 5), "C9266": ("anillo", 4), "C9267": ("anillo", 5), "C9268": ("anillo", 5),
    "C9269": ("anillo", 6), "C9272": ("anillo", 5), "C9274": ("anillo", 5), "C9276": ("anillo", 5),
    # anillos puestos en la mano
    "C9254": ("anillo", 4), "C9256": ("anillo", 4), "C9258": ("anillo", 4), "C9260": ("anillo", 4),
    "C9262": ("anillo", 4),
    # topos en una oreja de maniquí (C9282 salía "anillo": el error contrario)
    "C9282": ("topo", 12), "C9284": ("topo", 8),
}
# apartados para medir: anillos sueltos (C9271 y C9278 salen 100 % "arete"), puestos y topos
APARTADOS = {
    "anillo": ["C9265", "C9270", "C9271", "C9273", "C9275", "C9278", "C9255", "C9257", "C9259", "C9261"],
    "topo": ["C9280", "C9281", "C9283", "C9285", "C9286"],
}
# muestras "difíciles" de un clip: las que el desplegado etiquetó con una clase distinta de la correcta
HARD_FRAC = 0.5


def original_of(clip: str) -> Path:
    return CARPETA / f"{clip}.MP4"


def frame_exact(video: Path, t: float, w: int, h: int) -> np.ndarray:
    """Cuadro exacto en t del original (girado por ffmpeg como en el editor) a w×h con INTER_AREA."""
    r = subprocess.run(["ffmpeg", "-v", "error", "-threads", "4", "-ss", f"{t:.3f}", "-i", str(video),
                        "-frames:v", "1", "-vf", f"scale={w}:{h}:flags=area", "-f", "rawvideo", "-pix_fmt", "bgr24", "-"],
                       capture_output=True, creationflags=NO_WINDOW | BELOW_NORMAL, timeout=300)
    buf = np.frombuffer(r.stdout, np.uint8)
    if buf.size < w * h * 3:
        raise RuntimeError(f"ffmpeg no dio el cuadro {video.name} t={t}: {r.stderr[-200:]!r}")
    return buf[: w * h * 3].reshape(h, w, 3).copy()


def choose_times(cache: dict, n: int, good: str, rnd: random.Random) -> list[tuple[float, bool]]:
    """n tiempos de muestra con joya: la mitad (como mucho) de las "difíciles" (alguna caja con otra
    clase) y el resto repartidos por el clip; separados al menos 1.5 s. Devuelve (t, difícil)."""
    js = [s for s in cache["samples"] if s.get("jewels")]
    hard = [s for s in js if any(b["label"] != good for b in s["jewels"])]
    chosen: list[tuple[float, bool]] = []

    def free(t):
        return all(abs(t - u) >= 1.5 for u, _ in chosen)

    nh = min(len(hard), int(round(n * HARD_FRAC))) if hard else 0
    for q in np.linspace(0.02, 0.98, max(nh, 1)) if nh else []:
        s = hard[int(round(q * (len(hard) - 1)))]
        if free(s["t"]):
            chosen.append((s["t"], True))
    for q in np.linspace(0.04, 0.96, n * 3):
        if len(chosen) >= n:
            break
        s = js[int(round(q * (len(js) - 1)))]
        if free(s["t"]):
            chosen.append((s["t"], False))
    return sorted(chosen)


def r2_boxes(model, img: np.ndarray) -> tuple[list[dict], list[dict]]:
    """(joyas, personas) de r2 en el cuadro tal como lo ve el análisis."""
    r = model.predict(img, imgsz=960, conf=0.05, verbose=False, device="cpu")[0]
    d = [{"box": [round(v, 1) for v in b], "conf": round(c, 3), "label": CLASSES[int(k)]}
         for b, c, k in zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist())]
    d = cross_class(d)
    jewels = [x for x in d if x["label"] in JEWEL7 and x["conf"] >= JEWEL_CONF]
    people = [x for x in d if (x["label"] == "persona" and x["conf"] >= 0.35) or (x["label"] == "cara" and x["conf"] >= 0.25)]
    return jewels, people


def relabel(jewels: list[dict], good: str) -> list[dict]:
    """Todas las piezas del clip a la clase correcta; si dos cajas acaban siendo la misma pieza con la
    misma clase (IoU > 0.6), se queda la más segura."""
    out: list[dict] = []
    for j in sorted(jewels, key=lambda x: -x["conf"]):
        if any(iou(j["box"], o["box"]) > 0.6 for o in out):
            continue
        out.append(dict(j, det_label=j["label"], label=good))
    return out


def area(b) -> float:
    return max(0.0, b[2] - b[0]) * max(0.0, b[3] - b[1])


def nested(j: dict, jewels: list[dict]) -> bool:
    """Caja parcial dentro de otra pieza (el trébol de un anillo ancho, la cola del pájaro de un topo): está
    al menos al 70 % dentro de una caja de joya que mide más del doble. Se quita: es parte de esa pieza."""
    return any(o is not j and area(o["box"]) > 2 * area(j["box"]) and overlap_small(j["box"], o["box"]) >= 0.7
               for o in jewels)


def cmd_extraer(a):
    from ultralytics import YOLO

    rnd = random.Random(11)
    C = load_caches()
    model = YOLO(str(R2))
    IMG.mkdir(parents=True, exist_ok=True)
    frames = []
    for clip, (good, n) in TRAIN_CLIPS.items():
        cache = C[clip]
        W, H = (540, 960) if cache["height"] > cache["width"] else (960, 540)
        raws = []
        for t, hard in choose_times(cache, n, good, rnd):
            fid = f"{clip}_{int(round(t * 1000)):06d}"
            path = IMG / f"{fid}.jpg"
            if path.exists():
                raw = cv2.imread(str(path))
            else:
                raw = frame_exact(original_of(clip), t, W, H)
                cv2.imwrite(str(path), raw, [cv2.IMWRITE_JPEG_QUALITY, 95])
            raws.append((fid, t, hard, raw))
        # como `analyse.rs`: el clip entero es LOG si lo es la mayoría de sus cuadros (allí, de los 4 primeros)
        flat = 2 * sum(bool(is_flat(r)) for *_, r in raws) >= len(raws)
        for fid, t, hard, raw in raws:
            seen = normalize_contrast(raw) if flat else raw
            jewels, people = r2_boxes(model, seen)
            frames.append({"id": fid, "clip": clip, "t": t, "good": good, "flat": flat, "w": W, "h": H,
                           "hard": hard or any(j["label"] != good for j in jewels),
                           "jewels": relabel(jewels, good), "people": people})
        print(clip, sum(f["clip"] == clip for f in frames), "cuadros", flush=True)
    MANIFEST.write_text(json.dumps(frames, ensure_ascii=False, indent=0), encoding="utf-8")
    print(len(frames), "cuadros;", sum(f["hard"] for f in frames), "difíciles;",
          sum(not f["flat"] for f in frames), "no planos")


def cmd_hojas(a):
    frames = json.loads(MANIFEST.read_text(encoding="utf-8"))
    out = TRAIN / "review" / "manos_r6"
    out.mkdir(parents=True, exist_ok=True)
    tw, th, cols, rows = 200, 356, 8, 3
    tiles = []
    for f in frames:
        raw = cv2.imread(str(IMG / f"{f['id']}.jpg"))
        img = normalize_contrast(raw) if f["flat"] else raw
        for j in f["jewels"]:
            x0, y0, x1, y1 = [int(v) for v in j["box"]]
            c = COLORS.get(j["label"], (255, 255, 255))
            cv2.rectangle(img, (x0, y0), (x1, y1), c, 3)
            # etiqueta final y, entre paréntesis, la que daba r2 si era otra
            txt = j["label"][:3] + ("" if j["det_label"] == j["label"] else f"({j['det_label'][:3]})")
            cv2.putText(img, txt, (x0 + 2, max(18, y0 - 6)), cv2.FONT_HERSHEY_SIMPLEX, 0.8, c, 2, cv2.LINE_AA)
        for p in f["people"]:
            x0, y0, x1, y1 = [int(v) for v in p["box"]]
            cv2.rectangle(img, (x0, y0), (x1, y1), (200, 200, 200), 1)
        t = cv2.resize(img, (tw, th), interpolation=cv2.INTER_AREA)
        cv2.putText(t, f["id"], (3, th - 6), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 0, 0), 3, cv2.LINE_AA)
        cv2.putText(t, f["id"], (3, th - 6), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1, cv2.LINE_AA)
        tiles.append(t)
    per = cols * rows
    for k in range(0, len(tiles), per):
        group = tiles[k:k + per]
        group += [np.zeros_like(tiles[0])] * (per - len(group))
        sheet = np.vstack([np.hstack(group[r * cols:(r + 1) * cols]) for r in range(rows)])
        cv2.imwrite(str(out / f"hoja_{k // per + 1:02d}.jpg"), sheet, [cv2.IMWRITE_JPEG_QUALITY, 85])
    print(len(tiles), "cuadros en", (len(tiles) + per - 1) // per, "hojas ->", out)


def cmd_escribir(a):
    frames = json.loads(MANIFEST.read_text(encoding="utf-8"))
    drop = set(DROP.read_text(encoding="utf-8").split()) if DROP.exists() else set()
    ds = TRAIN / a.out
    rnd = random.Random(23)
    n = {"base": 0, "raw": 0, "blur": 0, "flip": 0}
    fuera = set(a.excluir.split(",")) if a.excluir else set()
    for f in frames:
        if f["id"] in drop or not f["jewels"] or f["clip"] in fuera:
            continue
        raw = cv2.imread(str(IMG / f"{f['id']}.jpg"))
        seen = normalize_contrast(raw) if f["flat"] else raw
        jewels = [j for j in f["jewels"] if not nested(j, f["jewels"])]
        boxes = [(j["label"], j["box"]) for j in jewels] + [(p["label"], p["box"]) for p in f["people"]]
        W = seen.shape[1]

        def save(name, im, bx, kind):
            cv2.imwrite(str(ds / "images" / "train" / f"m_{name}.jpg"), im, [cv2.IMWRITE_JPEG_QUALITY, 93])
            (ds / "labels" / "train" / f"m_{name}.txt").write_text("\n".join(yolo_lines(bx, im.shape[1], im.shape[0])),
                                                                   encoding="utf-8")
            n[kind] += 1

        save(f["id"], seen, boxes, "base")
        # mismos aumentos fuera de línea que build_dataset para un clip plano
        if f["flat"] and rnd.random() < a.aug:
            save(f"{f['id']}_raw", raw, boxes, "raw")
        if rnd.random() < a.aug:
            save(f"{f['id']}_blur", blur(seen, rnd), boxes, "blur")
        # los cuadros difíciles (r2 se equivocaba de clase) pesan el doble: copia volteada
        if f["hard"]:
            save(f"{f['id']}_flip", cv2.flip(seen, 1), [(lab, [W - b[2], b[1], W - b[0], b[3]]) for lab, b in boxes], "flip")
    print("añadidas a", ds, n, "| descartados", len(drop))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("paso", choices=["extraer", "hojas", "escribir"])
    ap.add_argument("--out", default="dataset_r6")
    ap.add_argument("--aug", type=float, default=0.35)
    # r7: sin los topos de la oreja de maniquí (en r6 llevaron sellos y anillos de cabeza redonda a "topo")
    ap.add_argument("--excluir", default="", help="clips que no entran (p. ej. C9282,C9284)")
    a = ap.parse_args()
    {"extraer": cmd_extraer, "hojas": cmd_hojas, "escribir": cmd_escribir}[a.paso](a)


if __name__ == "__main__":
    main()
