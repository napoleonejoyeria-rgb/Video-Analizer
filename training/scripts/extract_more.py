"""Cuadros extra de clips concretos (p. ej. los macro de las carpetas apartadas) para ampliar un
conjunto. Añade las entradas a data/plan.json y data/split.json con ids nuevos (desde 04000).

Uso: extract_more.py --clips lista_rel.txt --per 3 --split eval [--seed 9]
"""
import argparse
import json
import random
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from sample_frames import OUT, ROOT, TRAIN, extract


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--clips", required=True)
    ap.add_argument("--per", type=int, default=3)
    ap.add_argument("--split", required=True)
    ap.add_argument("--seed", type=int, default=9)
    a = ap.parse_args()
    rnd = random.Random(a.seed)
    plan = json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    info = {p["rel"]: p for p in plan}
    nxt = max(4000, max(int(p["id"]) for p in plan) + 1)
    jobs = []
    for rel in Path(a.clips).read_text(encoding="utf-8").splitlines():
        rel = rel.strip()
        if not rel or rel not in info:
            continue
        p = info[rel]
        used = [q["t"] for q in plan if q["rel"] == rel]
        for _ in range(a.per):
            # lejos (> 1.5 s) de los cuadros que ya hay del mismo clip
            for _try in range(20):
                t = rnd.uniform(0.3, max(0.4, p["duration"] - 0.5))
                if all(abs(t - u) > 1.5 for u in used):
                    break
            used.append(t)
            j = {k: p[k] for k in ("clip", "rel", "folder", "duration", "width", "height", "rotation")}
            j.update(id=f"{nxt:05d}", t=round(t, 3))
            nxt += 1
            jobs.append(j)
    with ThreadPoolExecutor(6) as ex:
        ok = list(ex.map(lambda j: extract(Path(j["clip"]), j["t"], OUT / f"{j['id']}.jpg"), jobs))
    jobs = [j for j, o in zip(jobs, ok) if o]
    plan += jobs
    for j in jobs:
        split[j["id"]] = a.split
    (TRAIN / "data" / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=0), encoding="utf-8")
    (TRAIN / "data" / "split.json").write_text(json.dumps(split), encoding="utf-8")
    (TRAIN / "data" / f"extra_{a.split}.txt").write_text("\n".join(j["id"] for j in jobs))
    print(len(jobs), "cuadros nuevos", jobs[0]["id"] if jobs else "", "->", jobs[-1]["id"] if jobs else "")


if __name__ == "__main__":
    main()
