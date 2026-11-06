"""Ronda 3: elige cuadros de entrenamiento sin revisar para los puntos débiles del detector propio
(macro / primer plano, cadena frente a pulsera y piezas largas en recipientes, cristal y vitrinas,
dijes y cadenas) y prepara sus propuestas: las del detector propio (sólidas ≥ 0.4, discontinuas
0.15–0.4) más las del maestro que el propio no ve (discontinuas).

Salida: review/r3_ids.txt, review/r3_props.json, y para la evaluación macro review/evm_ids.txt + evm_props.json.
"""
import json
import random
from collections import Counter

from labels import TRAIN, iou, load_teacher, proposals


def props_for(fid, S, T):
    st = [d for d in S[fid]["dets"] if d["label"] not in ("persona", "cara")]
    te = [p for p in proposals(T[fid]) if p["conf"] >= 0.3] if fid in T else []
    miss = [p for p in te if not any(iou(p["box"], s["box"]) > 0.3 for s in st)]
    boxes = [{"box": [round(v) for v in s["box"]], "label": s["label"], "weak": s["conf"] < 0.4, "conf": s["conf"]} for s in st]
    boxes += [{"box": [round(v) for v in p["box"]], "label": p["label"], "weak": True, "conf": round(p["conf"], 2)} for p in miss]
    return boxes[:18]


def main():
    rnd = random.Random(31)
    T = load_teacher()
    S = {}
    for l in (TRAIN / "teacher" / "scores.jsonl").read_text().splitlines():
        r = json.loads(l)
        S[r["id"]] = r
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    plan = {p["id"]: p for p in json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))}
    reviewed = set()
    for f in ("gold_ids.txt", "more_ids.txt", "r2_ids.txt"):
        reviewed |= set((TRAIN / "review" / f).read_text().split())
    pool = [i for i in S if split.get(i) == "train" and i not in reviewed and i in T]
    rnd.shuffle(pool)
    per = Counter()
    picked: list[str] = []

    def lab(fid, names, lo=0.15, hi=1.0):
        return any(d["label"] in names and lo <= d["conf"] <= hi for d in S[fid]["dets"])

    def take(pred, n, cap=10):
        k = 0
        for i in pool:
            if k >= n:
                break
            f = plan[i]["folder"]
            if i not in picked and per[f] < cap and pred(i):
                picked.append(i)
                per[f] += 1
                k += 1
        return k

    got = {
        "macro": take(lambda i: S[i]["macro"] >= 0.5, 190, cap=12),
        "recipiente/cadena-pulsera": take(lambda i: S[i]["container"] >= 0.35 or (lab(i, {"cadena"}) and lab(i, {"pulsera"}))
                                          or lab(i, {"cadena", "pulsera"}, 0.15, 0.5), 120),
        "cristal": take(lambda i: S[i]["glass"] >= 0.15, 60, cap=14),
        "dije": take(lambda i: lab(i, {"dije"}) or any(p["label"] == "dije" for p in proposals(T[i])), 100),
        "cadena": take(lambda i: lab(i, {"cadena"}), 30),
    }
    print(got, "total", len(picked), "de", len(pool))
    (TRAIN / "review" / "r3_ids.txt").write_text("\n".join(picked), encoding="utf-8")
    (TRAIN / "review" / "r3_props.json").write_text(json.dumps({i: props_for(i, S, T) for i in picked}), encoding="utf-8")
    # evaluación macro: los cuadros nuevos de clips macro apartados (los antiguos ya están revisados)
    ev = (TRAIN / "data" / "extra_eval.txt").read_text().split()
    (TRAIN / "review" / "evm_ids.txt").write_text("\n".join(ev), encoding="utf-8")
    (TRAIN / "review" / "evm_props.json").write_text(json.dumps({i: props_for(i, S, T) for i in ev}), encoding="utf-8")


if __name__ == "__main__":
    main()
