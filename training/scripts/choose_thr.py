"""Elige el umbral de trabajo del detector propio en la VALIDACIÓN revisada (no en la evaluación):
el que maximiza F1 de "joya" (¿hay una joya ahí?), con la supresión entre clases ya aplicada.

Uso: choose_thr.py --model runs/r2/weights/best.pt
"""
import argparse
import json

import numpy as np

from decisions import gt
from evaluate import cross_class, evaluate, student_preds
from labels import TRAIN, load_teacher


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    a = ap.parse_args()
    T = load_teacher()
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    G = {}
    for s in ("gold", "more", "r2", "r3", "r5"):
        G.update({k: v for k, v in gt(s).items() if not v["unsure"] and split.get(k) == "val"})
    P = {k: cross_class(v) for k, v in student_preds(a.model, sorted(G), T).items()}
    rows = []
    for t in np.arange(0.15, 0.61, 0.05):
        m = evaluate(P, G, float(t))["joya"]
        f1 = 2 * m["p"] * m["r"] / max(1e-9, m["p"] + m["r"])
        rows.append((f1, float(t), m["p"], m["r"]))
        print(f"umbral {t:.2f}: P={m['p']:.2f} R={m['r']:.2f} F1={f1:.3f}")
    best = max(rows)
    print(f"{len(G)} cuadros de validación; mejor umbral {best[1]:.2f}")


if __name__ == "__main__":
    main()
