"""Revisión en mosaico (4 cuadros por imagen, A–D) para ir rápido con cajas grandes:
- `audit`: cajas ya revisadas de pulsera/cadena/collar grandes o que tocan el borde, para corregir las
  parciales (regla nueva: una caja sobre TODA la extensión visible de la pieza, hasta el borde);
- `new`: propuestas del detector propio en cuadros nuevos (macro de piezas largas y negativos).
Cada tile lleva marcas cada 100 px en coordenadas del cuadro de 960 px.

Uso: mosaic_review.py audit|new --batches 2
Salida: review/mos_<modo>/<k>.jpg, review/mos_<modo>/index.json, review/mos_<modo>_batch<b>.txt
"""
import argparse
import json

import cv2
import numpy as np

from labels import SHORT, TRAIN, load_teacher
from render import COLORS, load_pair

LONG = {"pulsera", "cadena", "collar"}


def tile(rec, boxes, letter, fid, side=480):
    im9, _ = load_pair(fid, rec["flat"])
    h, w = im9.shape[:2]
    s = side / max(h, w)
    v = cv2.resize(im9, (int(w * s), int(h * s)), interpolation=cv2.INTER_AREA)
    for g in range(100, 960, 100):
        if g < w:
            x = int(g * s)
            cv2.line(v, (x, 0), (x, 6), (255, 255, 255), 1)
            cv2.putText(v, str(g), (x + 1, 15), cv2.FONT_HERSHEY_SIMPLEX, 0.3, (255, 255, 255), 1)
        if g < h:
            y = int(g * s)
            cv2.line(v, (0, y), (6, y), (255, 255, 255), 1)
            cv2.putText(v, str(g), (8, y + 3), cv2.FONT_HERSHEY_SIMPLEX, 0.3, (255, 255, 255), 1)
    for b in boxes:
        x0, y0, x1, y1 = [int(c * s) for c in b["box"]]
        col = COLORS.get(b["label"], (255, 255, 255))
        if b.get("ctx"):
            cv2.rectangle(v, (x0, y0), (x1, y1), (150, 150, 150), 1)
            continue
        if b.get("weak"):
            for xx in range(x0, x1, 6):
                cv2.line(v, (xx, y0), (min(xx + 3, x1), y0), col, 1)
                cv2.line(v, (xx, y1), (min(xx + 3, x1), y1), col, 1)
            for yy in range(y0, y1, 6):
                cv2.line(v, (x0, yy), (x0, min(yy + 3, y1)), col, 1)
                cv2.line(v, (x1, yy), (x1, min(yy + 3, y1)), col, 1)
        else:
            cv2.rectangle(v, (x0, y0), (x1, y1), col, 2)
        t = f"{b['n']}{SHORT.get(b['label'], '?')}"
        cv2.putText(v, t, (x0 + 2, max(10, y0 + 12)), cv2.FONT_HERSHEY_SIMPLEX, 0.4, (0, 0, 0), 3)
        cv2.putText(v, t, (x0 + 2, max(10, y0 + 12)), cv2.FONT_HERSHEY_SIMPLEX, 0.4, col, 1)
    cell = np.full((side, side if w > h else int(side * 9 / 16) + 2, 3), 25, np.uint8) if w <= h else np.full((int(side * 9 / 16) + 2, side, 3), 25, np.uint8)
    cell[:v.shape[0], :v.shape[1]] = v[:cell.shape[0], :cell.shape[1]]
    lab = f"{letter} {fid}"
    cv2.putText(cell, lab, (4, cell.shape[0] - 6), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 0, 0), 3)
    cv2.putText(cell, lab, (4, cell.shape[0] - 6), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 255, 255), 1)
    return cell


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode")
    ap.add_argument("--batches", type=int, default=2)
    a = ap.parse_args()
    T = load_teacher()
    items = []  # (fid, w, h, boxes)
    if a.mode == "audit":
        A = json.loads((TRAIN / "review" / "audit_boxes.json").read_text())
        for fid, r in sorted(A.items()):
            bx = [dict(b, ctx=b["label"] not in LONG) for b in r["boxes"]]
            items.append((fid, r["w"], r["h"], bx))
    else:
        P = json.loads((TRAIN / "review" / "r5_props.json").read_text())
        for fid in (TRAIN / "review" / "r5_ids.txt").read_text().split():
            items.append((fid, T[fid]["w"], T[fid]["h"], [dict(b, n=k + 1) for k, b in enumerate(P[fid])]))
    # cuadros verticales juntos (4 en fila) y horizontales juntos (2x2)
    out = TRAIN / "review" / f"mos_{a.mode}"
    out.mkdir(parents=True, exist_ok=True)
    index, k = {}, 0
    for vert in (True, False):
        group = [it for it in items if (it[2] > it[1]) == vert]
        for i in range(0, len(group), 4):
            chunk = group[i:i + 4]
            cells = [tile(T[f], bx, "ABCD"[j], f) for j, (f, _, _, bx) in enumerate(chunk)]
            while len(cells) < 4:
                cells.append(np.zeros_like(cells[0]))
            img = np.hstack(cells) if vert else np.vstack([np.hstack(cells[:2]), np.hstack(cells[2:])])
            name = f"{k:03d}"
            cv2.imwrite(str(out / f"{name}.jpg"), img, [cv2.IMWRITE_JPEG_QUALITY, 85])
            index[name] = {"ABCD"[j]: {"id": f, "w": w, "h": h, "boxes": bx} for j, (f, w, h, bx) in enumerate(chunk)}
            k += 1
    (out / "index.json").write_text(json.dumps(index), encoding="utf-8")
    names = sorted(index)
    for b in range(a.batches):
        (TRAIN / "review" / f"mos_{a.mode}_batch{b}.txt").write_text("\n".join(names[b::a.batches]), encoding="utf-8")
    print(len(items), "cuadros en", len(names), "mosaicos")


if __name__ == "__main__":
    main()
