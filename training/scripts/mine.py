"""Ronda 2: busca, entre los cuadros de entrenamiento aún sin revisar, los que más enseñan (donde el
detector propio de la ronda 1 y el maestro no están de acuerdo, o salen clases escasas) y prepara sus
propuestas para revisión a mano: las del detector propio (sólidas ≥ 0.4, discontinuas 0.2–0.4) más las
del maestro que el propio no ve (discontinuas).

Uso: mine.py --model runs/r1/weights/best.pt --n 400
Salida: review/r2_ids.txt y review/r2_props.json (id -> cajas)
"""
import argparse
import json
from collections import Counter

import cv2

from camcuts_metrics import normalize_contrast
from labels import CLASSES, TRAIN, iou, load_teacher, proposals


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--n", type=int, default=400)
    a = ap.parse_args()
    from ultralytics import YOLO

    m = YOLO(a.model)
    T = load_teacher()
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    plan = {p["id"]: p for p in json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))}
    reviewed = set()
    for f in ("gold_ids.txt", "more_ids.txt"):
        reviewed |= set((TRAIN / "review" / f).read_text().split())
    pool = [i for i in T if split.get(i) == "train" and i not in reviewed]
    scored = []
    props = {}
    rare = {"topo", "arete", "collar", "dije"}
    for fid in pool:
        img = cv2.imread(str(TRAIN / "data" / "img960" / f"{fid}.jpg"))
        if T[fid]["flat"]:
            img = normalize_contrast(img)
        r = m.predict(img, imgsz=960, conf=0.2, verbose=False, device=0)[0]
        st = [{"box": b, "conf": c, "label": CLASSES[int(k)]} for b, c, k in
              zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist()) if CLASSES[int(k)] not in ("persona", "cara")]
        te = [p for p in proposals(T[fid]) if p["conf"] >= 0.3]
        miss_t = [p for p in te if not any(iou(p["box"], s["box"]) > 0.3 for s in st)]
        miss_s = [s for s in st if s["conf"] >= 0.4 and not any(iou(p["box"], s["box"]) > 0.3 for p in te)]
        lab_dis = sum(1 for s in st for p in te if iou(p["box"], s["box"]) > 0.5 and p["label"] != s["label"])
        score = len(miss_t) + len(miss_s) + lab_dis + 1.5 * sum(s["label"] in rare for s in st)
        boxes = [{"box": [round(v) for v in s["box"]], "label": s["label"], "weak": s["conf"] < 0.4, "conf": round(s["conf"], 2)} for s in st]
        boxes += [{"box": [round(v) for v in p["box"]], "label": p["label"], "weak": True, "conf": round(p["conf"], 2)} for p in miss_t]
        props[fid] = boxes[:18]
        scored.append((score, fid))
    scored.sort(reverse=True)
    # como mucho 12 cuadros por carpeta, para no quedarse con una sola sesión
    per = Counter()
    picked = []
    for s, fid in scored:
        f = plan[fid]["folder"]
        if per[f] < 12:
            picked.append(fid)
            per[f] += 1
        if len(picked) >= a.n:
            break
    (TRAIN / "review" / "r2_ids.txt").write_text("\n".join(picked), encoding="utf-8")
    (TRAIN / "review" / "r2_props.json").write_text(json.dumps({i: props[i] for i in picked}), encoding="utf-8")
    print(len(pool), "sin revisar;", len(picked), "elegidos; puntuación mínima", scored[len(picked) - 1][0])


if __name__ == "__main__":
    main()
