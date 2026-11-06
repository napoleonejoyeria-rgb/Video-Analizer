"""Dónde difieren los tramos aprobados de dos análisis (world frente a propio) y cómo se ven esos cuadros.

Para cada clip calcula los tramos aprobados (preset joyas, como tools/parity.py), toma momentos que solo
aprueba uno de los dos y dibuja lado a lado el cuadro con las joyas de cada análisis (de la caché).

Uso: e2e_diff.py <dir_world> <dir_propio> <salida> [--max 12]
"""
import argparse
import json
import random
import subprocess
import sys
from pathlib import Path

import cv2
import numpy as np

sys.path.insert(0, r"C:\Users\Usuario\Tools\camcuts_next")
from camcuts.scoring import PRESETS, build_segments, refine_jewels, stabilize_labels  # noqa: E402

from render import COLORS  # noqa: E402


def load(d):
    out = {}
    for f in (Path(d) / "cache").glob("*.json"):
        e = json.loads(f.read_text(encoding="utf-8"))
        refine_jewels(e["samples"], e["width"], e["height"])
        stabilize_labels(e["samples"], e["per_second"])
        out[Path(e["path"]).name] = e
    return out


def segs(e):
    return [(s.start, s.end) for s in build_segments(e["samples"], PRESETS["joyas"], e["per_second"], e["duration"])]


def inside(t, ss):
    return any(a <= t <= b for a, b in ss)


def frame_at(path, t, long_side=960):
    vf = f"scale='if(gte(iw,ih),{long_side},-2)':'if(gte(iw,ih),-2,{long_side})':flags=area"
    r = subprocess.run(["ffmpeg", "-v", "error", "-ss", f"{t:.3f}", "-i", path, "-frames:v", "1", "-vf", vf,
                        "-f", "rawvideo", "-pix_fmt", "bgr24", "-"], capture_output=True, creationflags=0x08000000)
    return r.stdout


def draw(img, s, title):
    im = img.copy()
    for j in s.get("jewels", []):
        x0, y0, x1, y1 = [int(v) for v in j["box"]]
        c = COLORS.get(j["label"], (255, 255, 255))
        cv2.rectangle(im, (x0, y0), (x1, y1), c, 2)
        txt = f"{j['label']} {j['conf']:.2f} p99={j.get('lap_p99', 0):.0f}"
        cv2.putText(im, txt, (x0, max(12, y0 - 4)), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 0, 0), 3)
        cv2.putText(im, txt, (x0, max(12, y0 - 4)), cv2.FONT_HERSHEY_SIMPLEX, 0.45, c, 1)
    cv2.putText(im, title, (8, im.shape[0] - 10), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 0, 0), 3)
    cv2.putText(im, title, (8, im.shape[0] - 10), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (255, 255, 255), 1)
    return im


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("out")
    ap.add_argument("--max", type=int, default=12)
    a = ap.parse_args()
    rnd = random.Random(2)
    old, new = load(a.old), load(a.new)
    Path(a.out).mkdir(parents=True, exist_ok=True)
    cands = []
    for name in sorted(set(old) & set(new)):
        eo, en = old[name], new[name]
        so, sn = segs(eo), segs(en)
        for s_o, s_n in zip(eo["samples"], en["samples"]):
            t = s_o["t"]
            a_o, a_n = inside(t, so), inside(t, sn)
            if a_o != a_n:
                cands.append((name, t, "solo world" if a_o else "solo propio", s_o, s_n, eo["path"]))
    rnd.shuffle(cands)
    # repartidos entre clips y entre los dos sentidos
    picked, seen = [], {}
    for c in sorted(cands, key=lambda c: c[2]):
        k = (c[0], c[2])
        if seen.get(k, 0) < 2:
            picked.append(c)
            seen[k] = seen.get(k, 0) + 1
    rnd.shuffle(picked)
    picked = picked[:a.max]
    index = []
    for k, (name, t, kind, s_o, s_n, path) in enumerate(picked):
        raw = frame_at(path, t)
        # orientación del clip (la caché guarda el tamaño ya girado); 960x540 y 540x960 ocupan lo mismo
        e = old[name]
        w, h = (960, 540) if e["width"] >= e["height"] else (540, 960)
        if len(raw) != w * h * 3:
            continue
        img = np.frombuffer(raw, np.uint8).reshape(h, w, 3)
        pair = np.hstack([draw(img, s_o, f"WORLD {name} t={t:.2f}"), draw(img, s_n, f"PROPIO ({kind})")])
        if pair.shape[1] > 1400:
            pair = cv2.resize(pair, (1400, int(pair.shape[0] * 1400 / pair.shape[1])), interpolation=cv2.INTER_AREA)
        f = Path(a.out) / f"{k:02d}.jpg"
        cv2.imwrite(str(f), pair, [cv2.IMWRITE_JPEG_QUALITY, 85])
        index.append({"file": f.name, "clip": name, "t": t, "kind": kind})
    (Path(a.out) / "index.json").write_text(json.dumps(index, indent=1), encoding="utf-8")
    from collections import Counter
    print(len(cands), "muestras distintas;", Counter(c[2] for c in cands), "->", len(index), "imágenes")


if __name__ == "__main__":
    main()
