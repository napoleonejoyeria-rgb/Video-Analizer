"""Ronda 3 (macro de piezas largas con poca profundidad de campo): saca cuadros nuevos de
- los clips hermanos de Octubre 27 que no están en la evaluación (C9181 y demás clips de evaluación fuera);
- los clips de entrenamiento que ya dieron planos macro de pulsera/cadena/collar;
- clips aún sin muestrear de las carpetas de entrenamiento con más planos macro.
Reparto por clip: ~15 % de los clips nuevos va a validación, el resto a entrenamiento. Solo lee D:.

Uso: mine_macro.py [--per 4]
Salida: frames en data/img1920, entradas en data/plan.json y data/split.json, data/extra_macro.txt.
"""
import argparse
import json
import random
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from sample_frames import OUT, ROOT, TRAIN, VIDEO_EXT, extract, probe

LONG = {"pulsera", "cadena", "collar"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--per", type=int, default=4)
    a = ap.parse_args()
    rnd = random.Random(17)
    plan = json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    S = {json.loads(l)["id"]: json.loads(l) for l in (TRAIN / "teacher" / "scores.jsonl").read_text().splitlines()}
    by_id = {p["id"]: p for p in plan}
    eval_clips = {p["rel"] for p in plan if split.get(p["id"]) == "eval"}
    eval_folders = {p["folder"] for p in plan if split.get(p["id"]) == "eval" and p["rel"].lower() != "julio\\28\\c0200.mp4"}
    sampled = {p["rel"] for p in plan}

    def long_macro(fid):
        r = S.get(fid)
        if not r or r["macro"] < 0.5:
            return False
        for d in r["dets"]:
            if d["label"] in LONG:
                x0, y0, x1, y1 = d["box"]
                if (x1 - x0) * (y1 - y0) / (r["w"] * r["h"]) > 0.15 or x0 < 4 or x1 > r["w"] - 4:
                    return True
        return False

    clips = []
    # 1) hermanos de Octubre 27 (sin muestrear y sin ser de evaluación)
    o27 = ROOT / "Octubre" / "27" / "videos"
    clips += [str(p.relative_to(ROOT)) for p in sorted(o27.iterdir()) if p.suffix.lower() in VIDEO_EXT
              and str(p.relative_to(ROOT)) not in eval_clips]
    # 2) clips de entrenamiento con planos macro de piezas largas
    hot = sorted({by_id[f]["rel"] for f in S if split.get(f) in ("train", "val") and long_macro(f)})
    clips += hot
    # 3) clips sin muestrear de carpetas de entrenamiento con muchos planos macro
    rate = defaultdict(lambda: [0, 0])
    for f, r in S.items():
        if split.get(f) == "train":
            rate[by_id[f]["folder"]][0] += r["macro"] >= 0.5
            rate[by_id[f]["folder"]][1] += 1
    for folder, (m, n) in rate.items():
        if n >= 4 and m / n >= 0.3 and folder not in eval_folders:
            d = ROOT / folder
            new = [str(p.relative_to(ROOT)) for p in sorted(d.iterdir()) if p.is_file() and p.suffix.lower() in VIDEO_EXT
                   and str(p.relative_to(ROOT)) not in sampled]
            clips += rnd.sample(new, min(4, len(new)))
    clips = [c for c in dict.fromkeys(clips) if c not in eval_clips and "c9181" not in c.lower()]
    print(len(clips), "clips:", sum(c.startswith("Octubre\\27") for c in clips), "de Octubre 27,", len(hot), "con macro ya visto")
    infos = {}
    with ThreadPoolExecutor(6) as ex:
        for c, i in zip(clips, ex.map(lambda c: probe(ROOT / c), clips)):
            if i and i["duration"] > 1:
                infos[c] = i
    nxt = max(int(p["id"]) for p in plan) + 1
    jobs = []
    # validación por clip: uno de cada ~7 clips nuevos (que no tenga ya cuadros de entrenamiento)
    train_clips = {p["rel"] for p in plan if split.get(p["id"]) == "train"}
    for c, i in infos.items():
        sp = "val" if (c not in train_clips and rnd.random() < 0.15) else "train"
        if c in {p["rel"] for p in plan if split.get(p["id"]) == "val"}:
            sp = "val"
        used = [p["t"] for p in plan if p["rel"] == c]
        for _ in range(a.per):
            for _try in range(20):
                t = rnd.uniform(0.3, max(0.4, i["duration"] - 0.5))
                if all(abs(t - u) > 1.2 for u in used):
                    break
            used.append(t)
            jobs.append({"id": f"{nxt:05d}", "clip": str(ROOT / c), "rel": c, "t": round(t, 3),
                         "folder": str(Path(c).parent), **i, "_split": sp})
            nxt += 1
    with ThreadPoolExecutor(6) as ex:
        ok = list(ex.map(lambda j: extract(Path(j["clip"]), j["t"], OUT / f"{j['id']}.jpg"), jobs))
    jobs = [j for j, o in zip(jobs, ok) if o]
    for j in jobs:
        split[j["id"]] = j.pop("_split")
    plan += jobs
    (TRAIN / "data" / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=0), encoding="utf-8")
    (TRAIN / "data" / "split.json").write_text(json.dumps(split), encoding="utf-8")
    (TRAIN / "data" / "extra_macro.txt").write_text("\n".join(j["id"] for j in jobs))
    from collections import Counter
    print(len(jobs), "cuadros nuevos", Counter(split[j["id"]] for j in jobs))


if __name__ == "__main__":
    main()
