"""Evaluación de la ronda 3: el mismo conjunto apartado de antes (301 cuadros) y el corte macro
(cuadros apartados con puntuación CLIP de macro ≥ 0.5: los antiguos más los nuevos de clips macro),
para YOLO-World x + CLIP, el modelo de la ronda anterior y el nuevo. Umbral de trabajo de cada modelo
propio = el de su `classes.json` (0.4), con la supresión entre clases de `trained.rs`.

Uso: evaluate_r3.py --old runs/r2/weights/best.pt --new runs/r3/weights/best.pt [--thr 0.4]
"""
import argparse
import json

from decisions import gt
from evaluate import cross_class, evaluate, student_preds, table
from gpu_turn import GpuTurn
from labels import TRAIN, baseline, load_teacher


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--old", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--thr", type=float, default=0.4)
    a = ap.parse_args()
    T = load_teacher()
    S = {json.loads(l)["id"]: json.loads(l) for l in (TRAIN / "teacher" / "scores.jsonl").read_text().splitlines()}
    G_main = {k: v for k, v in gt("eval").items() if not v["unsure"]}
    G_new = {k: v for k, v in gt("evm").items() if not v["unsure"]}
    G_all = {**G_main, **G_new}
    G_macro = {k: v for k, v in G_all.items() if S[k]["macro"] >= 0.5}
    ids = sorted(G_all)
    with GpuTurn(max_wait_s=3600):
        P_old = {k: cross_class(v) for k, v in student_preds(a.old, ids, T).items()}
        P_new = {k: cross_class(v) for k, v in student_preds(a.new, ids, T).items()}
    base = {f: baseline(T[f]) for f in ids}
    for name, G in (("apartado (mismos 301 cuadros)", G_main), (f"corte macro ({len(G_macro)} cuadros)", G_macro)):
        print(f"##### {name}: {sum(len(g['boxes']) for g in G.values())} joyas")
        print(table("YOLO-World x + CLIP", evaluate(base, G, 0.15)))
        print(table(f"propio ronda anterior ({a.old})", evaluate(P_old, G, a.thr)))
        print(table(f"propio nuevo ({a.new})", evaluate(P_new, G, a.thr)))
    json.dump({"old": P_old, "new": P_new}, open(TRAIN / "review" / "r3_eval_preds.json", "w"))


if __name__ == "__main__":
    main()
