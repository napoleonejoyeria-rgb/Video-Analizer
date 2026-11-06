"""Hojas de contacto por clase con recortes de las cajas revisadas (para comprobar a ojo que cada clase es
coherente). Cada recorte lleva un número; review/sheets/<clase>_<k>.json traduce número -> (cuadro, caja).

Uso: class_sheets.py [--sets gold,more] [--per 60] [--only anillo,topo]
"""
import argparse
import json
import random

import cv2
import numpy as np

from decisions import gt
from labels import JEWEL7, TRAIN, load_teacher
from render import load_pair


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sets", default="gold,more")
    ap.add_argument("--per", type=int, default=60)
    ap.add_argument("--only", default="")
    ap.add_argument("--all", action="store_true", help="todas las cajas de la clase, en varias hojas")
    a = ap.parse_args()
    T = load_teacher()
    rnd = random.Random(4)
    items = {c: [] for c in JEWEL7}
    for s in a.sets.split(","):
        for fid, g in gt(s).items():
            if g["unsure"]:
                continue
            for k, b in enumerate(g["boxes"]):
                items[b["label"]].append((fid, k, b["box"], s))
    out = TRAIN / "review" / "sheets"
    out.mkdir(parents=True, exist_ok=True)
    cache = {}
    for c in (a.only.split(",") if a.only else JEWEL7):
        L = items[c][:]
        rnd.shuffle(L)
        chunks = [L[i:i + a.per] for i in range(0, len(L), a.per)] if a.all else [L[:a.per]]
        for ci, chunk in enumerate(chunks):
            side, cols = 128, 10
            rows = (len(chunk) + cols - 1) // cols
            sheet = np.full((rows * side, cols * side, 3), 25, np.uint8)
            index = []
            for n, (fid, k, box, s) in enumerate(chunk):
                if fid not in cache:
                    cache[fid] = load_pair(fid, T[fid]["flat"])[1]
                big = cache[fid]
                kx = big.shape[1] / T[fid]["w"]
                x0, y0, x1, y1 = [v * kx for v in box]
                m = max(x1 - x0, y1 - y0) * 0.2 + 8
                H, W = big.shape[:2]
                cx0, cy0, cx1, cy1 = int(max(0, x0 - m)), int(max(0, y0 - m)), int(min(W, x1 + m)), int(min(H, y1 + m))
                crop = big[cy0:cy1, cx0:cx1].copy()
                cv2.rectangle(crop, (int(x0 - cx0), int(y0 - cy0)), (int(x1 - cx0), int(y1 - cy0)), (0, 255, 255), max(1, crop.shape[1] // 100))
                sc = min(side / crop.shape[1], side / crop.shape[0])
                crop = cv2.resize(crop, (max(1, int(crop.shape[1] * sc)), max(1, int(crop.shape[0] * sc))))
                r, cc = divmod(n, cols)
                sheet[r * side:r * side + crop.shape[0], cc * side:cc * side + crop.shape[1]] = crop
                cv2.putText(sheet, str(n), (cc * side + 2, r * side + 13), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 0, 0), 3)
                cv2.putText(sheet, str(n), (cc * side + 2, r * side + 13), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                index.append({"n": n, "id": fid, "k": k, "set": s, "box": box})
            cv2.imwrite(str(out / f"{c}_{ci}.jpg"), sheet, [cv2.IMWRITE_JPEG_QUALITY, 85])
            (out / f"{c}_{ci}.json").write_text(json.dumps(index), encoding="utf-8")
        print(c, len(L), "cajas,", len(chunks), "hojas")


if __name__ == "__main__":
    main()
