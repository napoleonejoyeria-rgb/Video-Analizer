"""Ronda 3 (sin reentrenar): detecciones débiles del detector propio pasadas por el verificador CLIP,
simulando exactamente lo que harán `trained.rs` + `verify_weak` del núcleo.

- Detector propio (pesos de la ronda 1) a conf ≥ 0.10 con la supresión entre clases de trained.rs.
- Fuertes: conf ≥ u (0.4). Débiles: lo ≤ conf < u → banda 0.02–0.15 del núcleo.
- verify_weak: hasta 4 débiles de joya que no solapen (> 0.5 de la menor) una fuerte, de más a menos
  confianza; se aceptan con p(joya) de CLIP ≥ 0.6 (mismos textos y recorte +15 % que la app).
- `--tune`: elige `lo` en la VALIDACIÓN revisada (F1 de "joya"). `--eval`: mide en el apartado.

Uso: sim_weak.py --model runs/r2/weights/best.pt --tune | --eval --lo 0.15
"""
import argparse
import json
import sys
from pathlib import Path

import cv2
import numpy as np
import torch

from camcuts_metrics import normalize_contrast
from decisions import gt
from evaluate import cross_class, evaluate, table
from gpu_turn import GpuTurn
from labels import CLASSES, JEWEL7, TRAIN, baseline, iou, load_teacher, overlap_small

sys.path.insert(0, r"C:\Users\Usuario\Tools\camcuts_next")
from camcuts.classify import JEWEL_TEXT, OTHER_TEXT  # noqa: E402

U = 0.4
CACHE = TRAIN / "review" / "weak_cache.json"


def frame(fid, T):
    img = cv2.imread(str(TRAIN / "data" / "img960" / f"{fid}.jpg"))
    return normalize_contrast(img) if T[fid]["flat"] else img


def compute(model_path, ids, T):
    """Predicciones (conf ≥ 0.10, supresión entre clases) y p de CLIP de cada joya con conf < u."""
    import clip
    from PIL import Image
    from ultralytics import YOLO

    m = YOLO(model_path)
    cm, pre = clip.load("ViT-B/32", device="cuda", download_root=str(Path.home() / ".cache" / "clip"))
    with torch.no_grad():
        t = cm.encode_text(clip.tokenize(JEWEL_TEXT + OTHER_TEXT).cuda()).float()
        t = t / t.norm(dim=-1, keepdim=True)
    out = {}
    for fid in ids:
        img = frame(fid, T)
        h, w = img.shape[:2]
        r = m.predict(img, imgsz=960, conf=0.10, verbose=False, device=0)[0]
        dets = [{"box": b, "conf": c, "label": CLASSES[int(k)]} for b, c, k in
                zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist())]
        dets = cross_class(dets)
        crops, idx = [], []
        for i, d in enumerate(dets):
            if d["label"] in JEWEL7 and d["conf"] < U:
                x0, y0, x1, y1 = [int(v) for v in d["box"]]  # Python guarda las cajas truncadas
                px, py = int((x1 - x0) * 0.15), int((y1 - y0) * 0.15)
                c = img[max(0, y0 - py):min(h, y1 + py), max(0, x0 - px):min(w, x1 + px)]
                if c.size:
                    crops.append(c)
                    idx.append(i)
        if crops:
            x = torch.stack([pre(Image.fromarray(cv2.cvtColor(c, cv2.COLOR_BGR2RGB))) for c in crops]).cuda()
            with torch.no_grad():
                e = cm.encode_image(x).float()
                e = e / e.norm(dim=-1, keepdim=True)
                p = (100 * e @ t.T).softmax(-1)[:, :len(JEWEL_TEXT)].sum(-1).tolist()
            for i, pp in zip(idx, p):
                dets[i]["clip_p"] = pp
        out[fid] = dets
    return out


def kept(dets, lo, verify=True):
    """Lo que acaba contando como joya en el núcleo con este `lo` (y personas aparte)."""
    strong = [d for d in dets if d["label"] in JEWEL7 and d["conf"] >= U]
    if not verify or lo is None:
        return strong
    weak = sorted([d for d in dets if d["label"] in JEWEL7 and lo <= d["conf"] < U
                   and not any(overlap_small(d["box"], s["box"]) > 0.5 for s in strong)], key=lambda d: -d["conf"])[:4]
    return strong + [d for d in weak if d.get("clip_p", 0) >= 0.6]


def bg_fp(P, G, ids):
    return sum(1 for f in ids for p in P[f] if not any(iou(p["box"], b["box"]) > 0.1 for b in G[f]["boxes"]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--tune", action="store_true")
    ap.add_argument("--eval", action="store_true")
    ap.add_argument("--lo", type=float, default=0.15)
    a = ap.parse_args()
    T = load_teacher()
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    S = {json.loads(l)["id"]: json.loads(l) for l in (TRAIN / "teacher" / "scores.jsonl").read_text().splitlines()}
    Gv = {}
    for s in ("gold", "more", "r2", "r3"):
        Gv.update({k: v for k, v in gt(s).items() if not v["unsure"] and split.get(k) == "val"})
    Ge = {k: v for k, v in gt("eval").items() if not v["unsure"]}
    Gm = {k: v for k, v in gt("evm").items() if not v["unsure"]}
    cache = json.loads(CACHE.read_text()) if CACHE.exists() else {}
    need = [i for i in sorted({*Gv, *Ge, *Gm}) if i not in cache]
    if need:
        with GpuTurn(max_wait_s=3600):
            cache.update(compute(a.model, need, T))
        CACHE.write_text(json.dumps(cache))
    if a.tune:
        print(f"validación: {len(Gv)} cuadros, {sum(len(g['boxes']) for g in Gv.values())} joyas")
        for lo in [None, 0.10, 0.125, 0.15, 0.175, 0.20, 0.25]:
            P = {f: kept(cache[f], lo) for f in Gv}
            m = evaluate(P, Gv, 0.0)["joya"]
            f1 = 2 * m["p"] * m["r"] / max(1e-9, m["p"] + m["r"])
            print(f"lo={lo}: P={m['p']:.3f} R={m['r']:.3f} F1={f1:.3f} (pred {m['pred']}, FP fondo {bg_fp(P, Gv, list(Gv))})")
    if a.eval:
        T_ = T
        G_all = {**Ge, **Gm}
        G_macro = {k: v for k, v in G_all.items() if S[k]["macro"] >= 0.5}
        base = {f: baseline(T_[f]) for f in G_all}
        r1 = {f: kept(cache[f], None, verify=False) for f in G_all}
        r1c = {f: kept(cache[f], a.lo) for f in G_all}
        for name, G in (("apartado (301)", Ge), (f"corte macro ({len(G_macro)})", G_macro)):
            print(f"##### {name}")
            for lab, P in (("World+CLIP", base), ("propio r1 (umbral 0.4)", r1), (f"propio r1 + CLIP débiles (lo={a.lo})", r1c)):
                print(table(lab, evaluate(P, G, 0.0)))
        ids = list(G_all)
        glass = [f for f in ids if S[f]["glass"] >= 0.1]
        cont = [f for f in ids if S[f]["container"] >= 0.4]
        bj = {f: [d for d in base[f]] for f in ids}
        for lab, P in (("World+CLIP", bj), ("r1", r1), ("r1+CLIP", r1c)):
            print(f"FP de fondo {lab}: todos {bg_fp(P, G_all, ids)} ({len(ids)} cuadros), apartado {bg_fp(P, G_all, list(Ge))}, "
                  f"cristal {bg_fp(P, G_all, glass)} ({len(glass)}), recipientes {bg_fp(P, G_all, cont)} ({len(cont)})")


if __name__ == "__main__":
    main()
