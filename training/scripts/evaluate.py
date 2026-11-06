"""Evaluación en el conjunto apartado revisado a mano: detector propio frente a la app actual
(YOLO-World x + verificador CLIP, mismas reglas que verify_weak) sobre los mismos cuadros.

Métricas por clase de joya: AP50 (interpolación de todos los puntos), precisión y exhaustividad en el
punto de trabajo; también "joya" sin clase (¿hay una joya ahí?), que es lo que más pesa en CAMCUTS.

Uso: evaluate.py --model runs/r1/weights/best.pt [--thr 0.25] [--set eval] [--dump preds.json]
"""
from __future__ import annotations

import argparse
import json

import cv2
import numpy as np

from camcuts_metrics import normalize_contrast
from decisions import gt
from labels import CLASSES, JEWEL7, TRAIN, baseline, iou, load_teacher


def ap(scores_tp: list[tuple[float, int]], npos: int) -> float:
    """AP con interpolación de todos los puntos (como VOC 2010+/COCO a IoU fijo)."""
    if npos == 0:
        return float("nan")
    s = sorted(scores_tp, key=lambda x: -x[0])
    tp = np.cumsum([t for _, t in s])
    fp = np.cumsum([1 - t for _, t in s])
    rec = tp / npos if len(s) else np.array([0.0])
    prec = tp / np.maximum(tp + fp, 1e-9) if len(s) else np.array([0.0])
    mrec = np.concatenate([[0.0], rec, [1.0]])
    mpre = np.concatenate([[1.0], prec, [0.0]])
    for i in range(len(mpre) - 2, -1, -1):
        mpre[i] = max(mpre[i], mpre[i + 1])
    idx = np.where(mrec[1:] != mrec[:-1])[0]
    return float(np.sum((mrec[idx + 1] - mrec[idx]) * mpre[idx + 1]))


def match(preds: list[dict], gts: list[dict], same_class: bool = True) -> list[tuple[float, int, str]]:
    """Empareja de mayor a menor confianza con IoU ≥ 0.5: (conf, acierto, clase predicha)."""
    used = [False] * len(gts)
    out = []
    for p in sorted(preds, key=lambda d: -d["conf"]):
        best, bi = 0.5, -1
        for i, g in enumerate(gts):
            if used[i] or (same_class and g["label"] != p["label"]):
                continue
            v = iou(p["box"], g["box"])
            if v >= best:
                best, bi = v, i
        if bi >= 0:
            used[bi] = True
        out.append((p["conf"], int(bi >= 0), p["label"]))
    return out


def evaluate(preds: dict[str, list[dict]], G: dict[str, dict], thr: float | dict) -> dict:
    """Métricas por clase y de "joya" sin clase. `thr`: umbral de trabajo (global o por clase)."""
    res = {}
    for c in JEWEL7 + ["joya"]:
        st, npos = [], 0
        for fid, g in G.items():
            if c == "joya":
                gg = [{"box": b["box"], "label": "joya"} for b in g["boxes"]]
                pp = [{"box": p["box"], "conf": p["conf"], "label": "joya"} for p in preds.get(fid, []) if p["label"] in JEWEL7]
            else:
                gg = [b for b in g["boxes"] if b["label"] == c]
                pp = [p for p in preds.get(fid, []) if p["label"] == c]
            npos += len(gg)
            st += [(s, t) for s, t, _ in match(pp, gg)]
        t = thr.get(c, thr.get("default", 0.25)) if isinstance(thr, dict) else thr
        kept = [x for x in st if x[0] >= t]
        tp = sum(x[1] for x in kept)
        res[c] = {"n": npos, "ap50": ap(st, npos), "p": tp / max(1, len(kept)), "r": tp / max(1, npos),
                  "pred": len(kept)}
    res["map50"] = float(np.nanmean([res[c]["ap50"] for c in JEWEL7]))
    return res


CROSS_IOU = 0.6


def cross_class(dets: list[dict], thr: float = CROSS_IOU) -> list[dict]:
    """Quita la caja de joya más débil cuando otra joya de distinta clase la tapa (IoU > thr): la misma
    pieza vista con dos etiquetas. Igual que `trained.rs`."""
    keep: list[dict] = []
    for d in sorted(dets, key=lambda x: -x["conf"]):
        if d["label"] in JEWEL7 and any(k["label"] in JEWEL7 and k["label"] != d["label"] and iou(k["box"], d["box"]) > thr for k in keep):
            continue
        keep.append(d)
    return keep


def student_preds(model_path: str, ids: list[str], T: dict, conf: float = 0.01) -> dict[str, list[dict]]:
    from ultralytics import YOLO

    m = YOLO(model_path)
    out = {}
    for fid in ids:
        img = cv2.imread(str(TRAIN / "data" / "img960" / f"{fid}.jpg"))
        if T[fid]["flat"]:
            img = normalize_contrast(img)
        r = m.predict(img, imgsz=960, conf=conf, verbose=False, device=0)[0]
        out[fid] = [{"box": b, "conf": c, "label": CLASSES[int(k)]}
                    for b, c, k in zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist())]
    return out


def table(name: str, m: dict) -> str:
    lines = [f"== {name}: mAP50 (7 clases) {m['map50']:.3f}"]
    for c in JEWEL7 + ["joya"]:
        x = m[c]
        lines.append(f"  {c:8s} n={x['n']:4d}  AP50={x['ap50']:.3f}  P={x['p']:.2f}  R={x['r']:.2f}  (pred {x['pred']})")
    return "\n".join(lines)


def main():
    ap_ = argparse.ArgumentParser()
    ap_.add_argument("--model", required=True)
    ap_.add_argument("--thr", type=float, default=0.25)
    ap_.add_argument("--set", default="eval")
    ap_.add_argument("--dump", default="")
    a = ap_.parse_args()
    T = load_teacher()
    G = {k: v for k, v in gt(a.set).items() if not v["unsure"]}
    ids = sorted(G)
    base = {fid: baseline(T[fid]) for fid in ids}
    stud = student_preds(a.model, ids, T)
    mb = evaluate(base, G, 0.15)
    ms = evaluate(stud, G, a.thr)
    print(f"{len(ids)} cuadros revisados ({a.set}), {sum(len(g['boxes']) for g in G.values())} joyas")
    print(table("YOLO-World x + CLIP (app actual, umbral 0.15 / CLIP)", mb))
    print(table(f"detector propio ({a.model}, umbral {a.thr})", ms))
    stud_x = {k: cross_class(v) for k, v in stud.items()}
    print(table(f"detector propio + supresión entre clases (IoU > {CROSS_IOU})", evaluate(stud_x, G, a.thr)))
    stud = stud_x
    # umbral que maximiza F1 de "joya" para el detector propio (informativo)
    best = max(((t, evaluate(stud, G, t)["joya"]) for t in np.arange(0.1, 0.65, 0.05)),
               key=lambda x: 2 * x[1]["p"] * x[1]["r"] / max(1e-9, x[1]["p"] + x[1]["r"]))
    print(f"mejor F1 de joya en umbral {best[0]:.2f}: P={best[1]['p']:.2f} R={best[1]['r']:.2f}")
    if a.dump:
        json.dump({"student": stud, "baseline": base}, open(a.dump, "w"))


if __name__ == "__main__":
    main()
