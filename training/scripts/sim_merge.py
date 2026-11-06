"""Simulación sin reentrenar: unir las cajas de piezas largas (pulsera/cadena/collar, y anillo cuando
solapa con ellas) que son trozos de la misma pieza. En los macro de poca profundidad de campo el
detector parte la pieza en dos medias cajas y reparte la confianza entre clases; la unión cubre la pieza
entera (incluida la parte enfocada) y su confianza es la "o" ruidosa de las partes: 1 − Π(1 − cᵢ).

Uso: sim_merge.py --model runs/r5/weights/best.pt  (validación + clips C9181 + corte macro)
"""
import argparse
import json

import cv2

from camcuts_metrics import normalize_contrast
from decisions import gt
from evaluate import cross_class, evaluate
from gpu_turn import GpuTurn
from labels import CLASSES, JEWEL7, TRAIN, iou, load_teacher, overlap_small

LONG = {"pulsera", "cadena", "collar"}


def merge_long(dets, iou_thr=0.25, ov_thr=0.7):
    """Agrupa joyas largas que se solapan (IoU ≥ iou_thr o una dentro de otra ≥ ov_thr) en una caja."""
    js = sorted([d for d in dets if d["label"] in LONG], key=lambda d: -d["conf"])
    rest = [d for d in dets if d["label"] not in LONG]
    groups = []
    for d in js:
        for g in groups:
            if any(iou(d["box"], e["box"]) >= iou_thr or overlap_small(d["box"], e["box"]) >= ov_thr for e in g):
                g.append(d)
                break
        else:
            groups.append([d])
    out = []
    for g in groups:
        p = 1.0
        for e in g:
            p *= 1 - e["conf"]
        b = [min(e["box"][0] for e in g), min(e["box"][1] for e in g), max(e["box"][2] for e in g), max(e["box"][3] for e in g)]
        out.append({"box": b, "conf": 1 - p, "label": g[0]["label"]})
    return rest + out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    a = ap.parse_args()
    from ultralytics import YOLO

    T = load_teacher()
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    S = {json.loads(l)["id"]: json.loads(l) for l in (TRAIN / "teacher" / "scores.jsonl").read_text().splitlines()}
    Gv = {}
    for s in ("gold", "more", "r2", "r3", "r5"):
        Gv.update({k: v for k, v in gt(s).items() if not v["unsure"] and split.get(k) == "val"})
    Ge = {k: v for k, v in {**gt("eval"), **gt("evm")}.items() if not v["unsure"]}
    plan = {p["id"]: p for p in json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))}
    c9181 = [f for f in Ge if "C9181" in plan[f]["rel"]]
    ids = sorted(set(Gv) | set(Ge))
    m = YOLO(a.model)
    P = {}
    with GpuTurn():
        for f in ids:
            img = cv2.imread(str(TRAIN / "data" / "img960" / f"{f}.jpg"))
            if T[f]["flat"]:
                img = normalize_contrast(img)
            r = m.predict(img, imgsz=960, conf=0.05, verbose=False, device=0)[0]
            d = [{"box": b, "conf": c, "label": CLASSES[int(k)]} for b, c, k in
                 zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist())]
            P[f] = [x for x in cross_class([x for x in d if x["conf"] >= 0.10]) if x["label"] in JEWEL7]
    Gm = {k: v for k, v in Ge.items() if S[k]["macro"] >= 0.5}
    for name, fn in (("tal cual", lambda d: d), ("unir piezas largas", merge_long)):
        Q = {f: fn(P[f]) for f in ids}
        for thr in (0.35, 0.4, 0.45):
            mv = evaluate(Q, Gv, thr)["joya"]
            f1 = 2 * mv["p"] * mv["r"] / max(1e-9, mv["p"] + mv["r"])
            mm = evaluate(Q, Gm, thr)["joya"]
            ma = evaluate(Q, {k: Ge[k] for k in Ge}, thr)["joya"]
            hit = sum(any(d["conf"] >= thr and d["label"] in LONG | {"anillo"} and (d["box"][2] - d["box"][0]) > 600 for d in Q[f]) for f in c9181)
            print(f"{name:20s} u={thr}: val F1={f1:.3f} (P {mv['p']:.2f} R {mv['r']:.2f}) | apartado+evm P {ma['p']:.2f} R {ma['r']:.2f} | macro R {mm['r']:.2f} P {mm['p']:.2f} | C9181 {hit}/{len(c9181)}")


if __name__ == "__main__":
    main()
