"""Prepara la revisión a mano de cuadros enteros: imagen con cajas numeradas + recortes, y un JSON con
las cajas en el mismo orden (para traducir el veredicto "caja 3 = topo" a coordenadas).

Uso: make_review.py --set eval|gold --ids lista.txt --batches 4
Salida: review/<set>/<id>.jpg, review/<set>/<id>.json, review/<set>_batch<k>.txt
"""
import argparse
import json

from labels import TRAIN, load_teacher, proposals
from render import render_frame


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--set", required=True)
    ap.add_argument("--ids", required=True)
    ap.add_argument("--batches", type=int, default=4)
    ap.add_argument("--props", default="", help="JSON id -> cajas propuestas (si no, las del maestro)")
    a = ap.parse_args()
    T = load_teacher()
    given = json.loads(open(a.props, encoding="utf-8").read()) if a.props else None
    ids = [i for i in open(a.ids).read().split() if i in T]
    out = TRAIN / "review" / a.set
    out.mkdir(parents=True, exist_ok=True)
    plan = {p["id"]: p for p in json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))}
    for i in ids:
        r = T[i]
        P = given[i] if given is not None else proposals(r, loose=True)[:18]
        boxes = [{"n": k, "box": [round(v) for v in p["box"]], "label": p["label"], "weak": p["weak"],
                  "conf": round(p["conf"], 2)} for k, p in enumerate(P, 1)]
        (out / f"{i}.json").write_text(json.dumps({"id": i, "w": r["w"], "h": r["h"], "flat": r["flat"],
                                                   "clip": plan[i]["rel"], "boxes": boxes}), encoding="utf-8")
        render_frame(r, boxes, str(out / f"{i}.jpg"), title=f"{i}  {plan[i]['rel'][-28:]}")
    n = a.batches
    for k in range(n):
        part = ids[k::n]
        (TRAIN / "review" / f"{a.set}_batch{k}.txt").write_text("\n".join(part), encoding="utf-8")
    print(len(ids), "cuadros preparados en", out)


if __name__ == "__main__":
    main()
