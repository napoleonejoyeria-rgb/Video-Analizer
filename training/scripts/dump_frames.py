"""Escribe cuadros de evaluación como BGR crudo (como los entrega el muestreador, contraste estirado si
son planos) para cronometrar y comparar los detectores en Rust: bench/frames.bin + bench/frames.json."""
import json

import cv2

from camcuts_metrics import normalize_contrast
from decisions import gt
from labels import TRAIN, load_teacher

T = load_teacher()
G = {k: v for k, v in gt("eval").items() if not v["unsure"]}
out = TRAIN / "bench"
out.mkdir(exist_ok=True)
meta, off = [], 0
with open(out / "frames.bin", "wb") as f:
    for fid in sorted(G):
        img = cv2.imread(str(TRAIN / "data" / "img960" / f"{fid}.jpg"))
        if T[fid]["flat"]:
            img = normalize_contrast(img)
        b = img.tobytes()
        f.write(b)
        meta.append({"id": fid, "w": img.shape[1], "h": img.shape[0], "offset": off})
        off += len(b)
(out / "frames.json").write_text(json.dumps(meta), encoding="utf-8")
print(len(meta), "cuadros", off / 1e6, "MB")
