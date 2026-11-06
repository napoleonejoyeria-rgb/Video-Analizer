"""Ronda 4 (r6/r7): comparación del detector desplegado (r2) con los afinados para anillos sueltos.

- r6: r2 + cuadros de Manos (anillos sueltos y puestos como "anillo", topos en oreja de maniquí como "topo").
- r7: lo mismo sin los topos de la oreja (en r6 llevaron sellos y anillos de cabeza redonda a "topo").

Pasos (cada uno que usa la GPU pide turno con `GpuTurn`):
  eval_r6.py metricas --modelos r2,r6,r7   conjunto apartado ("eval", 301 cuadros revisados) y validación
                                 revisada (la de choose_thr): AP50/P/R por clase al umbral 0.4 con la supresión
                                 entre clases de trained.rs, F1 de "joya" por umbral y matriz de confusión
                                 anillo/arete/topo (¿con qué clase sale cada joya real?)
  eval_r6.py exportar --tag r7   best.pt a ONNX FP32 y FP16 en training/staging_<tag> (+ classes.json) y
                                 enlaces duros en e2e_r6/bin_<tag>/models para la prueba de punta a punta
  eval_r6.py e2e --bins r2,r7    camcuts.exe --analizar de cada copia sobre Manos y Octubre 27, con
                                 --sin-respaldo (se compara el detector, no el respaldo con YOLO-World)
  eval_r6.py manos --tags r2,r7  etiquetas de las cajas de joya en las cachés de Manos, por grupo de clips
  eval_r6.py paridad --tags r2,r7  segundos aprobados (preset "joyas", parity.py de la app de referencia)
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from labels import JEWEL7, TRAIN, iou  # noqa: E402

OUT = TRAIN / "e2e_runs" / "r6cmp"
FOLDERS = {"manos": r"D:\CONTENIDO NAPO\2026\Noviembre\3\Videos Yo Manos Detalle",
           "oct27": r"D:\CONTENIDO NAPO\2026\Octubre\27\videos"}
PARITY = r"C:\Users\Usuario\Tools\camcuts_next\tools\parity.py"
NO_WINDOW, BELOW_NORMAL = 0x08000000, 0x00004000


def weights(tag: str) -> Path:
    return TRAIN / "runs" / tag / "weights" / "best.pt"


def bin_of(tag: str) -> Path:
    """Carpeta del camcuts.exe de prueba de cada modelo. El desplegado (r2) usa el exe compilado del
    workspace: sin models/ a su lado, la app carga los de `camcuts_rs/models` (los instalados)."""
    if tag == "r2":
        return TRAIN.parent / "target" / "release"
    return TRAIN / "e2e_r6" / f"bin_{tag}"


def confusion(P: dict, G: dict, thr: float) -> dict[str, Counter]:
    """Para cada joya real, la clase de la predicción (≥ thr) que mejor la cubre con IoU ≥ 0.5, o "nada"."""
    out: dict[str, Counter] = {}
    for fid, g in G.items():
        preds = [p for p in P.get(fid, []) if p["label"] in JEWEL7 and p["conf"] >= thr]
        for b in g["boxes"]:
            best, lab = 0.5, "nada"
            for p in preds:
                v = iou(p["box"], b["box"])
                if v >= best:
                    best, lab = v, p["label"]
            out.setdefault(b["label"], Counter())[lab] += 1
    return out


def cmd_metricas(a):
    from decisions import gt
    from evaluate import cross_class, evaluate, student_preds, table
    from gpu_turn import GpuTurn
    from labels import load_teacher

    T = load_teacher()
    split = json.loads((TRAIN / "data" / "split.json").read_text())
    Ge = {k: v for k, v in gt("eval").items() if not v["unsure"]}
    Gv = {}
    for s in ("gold", "more", "r2", "r3", "r5"):
        Gv.update({k: v for k, v in gt(s).items() if not v["unsure"] and split.get(k) == "val"})
    ids = sorted(set(Ge) | set(Gv))
    models = {k: str(weights(k)) for k in a.modelos.split(",")}
    P = {}
    with GpuTurn(max_wait_s=3 * 3600):
        for k, m in models.items():
            P[k] = {f: cross_class(v) for f, v in student_preds(m, ids, T).items()}
    res = {}
    for name, G in (("apartado (eval)", Ge), ("validación", Gv)):
        print(f"##### {name}: {len(G)} cuadros, {sum(len(g['boxes']) for g in G.values())} joyas")
        for k in models:
            m = evaluate(P[k], G, a.thr)
            res[f"{k}|{name}"] = m
            print(table(f"{k} (umbral {a.thr})", m))
            cf = confusion(P[k], G, a.thr)
            for c in ("anillo", "arete", "topo"):
                tot = sum(cf.get(c, Counter()).values())
                print(f"    real {c:7s} n={tot:3d} -> " + ", ".join(f"{l} {n}" for l, n in cf.get(c, Counter()).most_common()))
        # F1 de "joya" por umbral (como choose_thr)
        for k in models:
            row = []
            for t in (0.3, 0.35, 0.4, 0.45, 0.5):
                j = evaluate(P[k], G, t)["joya"]
                row.append(f"{t:.2f}:{2 * j['p'] * j['r'] / max(1e-9, j['p'] + j['r']):.3f}")
            print(f"  F1 joya {k}: " + "  ".join(row))
    (TRAIN / "review" / f"metricas_{'_'.join(models)}.json").write_text(json.dumps(res, indent=1), encoding="utf-8")


def cmd_exportar(a):
    import export_onnx
    from gpu_turn import GpuTurn

    staging = TRAIN / f"staging_{a.tag}"
    staging.mkdir(exist_ok=True)
    with GpuTurn(max_wait_s=3 * 3600):  # la versión FP16 se exporta en CUDA
        for fp16 in (False, True):
            sys.argv = ["export_onnx.py", "--model", str(weights(a.tag)), "--out",
                        str(staging / ("jewel_det_fp16.onnx" if fp16 else "jewel_det.onnx")), "--thr", f"{a.thr:.2f}"]
            if fp16:
                sys.argv.append("--fp16")
            export_onnx.main()
    # copia de prueba de la app (lee models/ junto al exe): el exe de r2 y enlaces duros a sus modelos (no
    # ocupan disco), salvo el detector propio, que apunta a staging
    b = bin_of(a.tag)
    (b / "models").mkdir(parents=True, exist_ok=True)
    if not (b / "camcuts.exe").exists():
        shutil.copy2(TRAIN.parent / "target" / "release" / "camcuts.exe", b / "camcuts.exe")
    for f in (TRAIN.parent / "models").iterdir():
        if not f.is_file() or f.suffix not in (".onnx", ".json"):
            continue
        dst = b / "models" / f.name
        src = staging / f.name if f.name.startswith("jewel_det") else f
        dst.unlink(missing_ok=True)
        dst.hardlink_to(src)
    print("modelos en", staging)


def cmd_e2e(a):
    from gpu_turn import GpuTurn

    OUT.mkdir(parents=True, exist_ok=True)
    times = {}
    for fk in a.folders.split(","):
        for bk in a.bins.split(","):
            out = OUT / f"{fk}_{bk}"
            with GpuTurn(max_wait_s=3 * 3600):
                t0 = time.time()
                with open(out.with_suffix(".log"), "w", encoding="utf-8") as fo:
                    # camcuts.exe es del subsistema de ventanas: solo --analizar, sin ventana, salida a archivo
                    r = subprocess.run([str(bin_of(bk) / "camcuts.exe"), "--analizar", FOLDERS[fk], "--out", str(out),
                                        "--detector", "propio", "--no-proxy", "--no-sheet", "--sin-respaldo",
                                        *(["--workers", a.workers] if a.workers else [])],
                                       stdout=fo, stderr=subprocess.STDOUT, creationflags=NO_WINDOW | BELOW_NORMAL)
                times[out.name] = round(time.time() - t0, 1)
            print(f"{out.name}: {times[out.name]:.0f} s (código {r.returncode})", flush=True)
    (OUT / f"tiempos_{int(time.time())}.json").write_text(json.dumps(times, indent=1))


def labels_of(folder: Path) -> dict[str, Counter]:
    """Etiquetas de todas las cajas de joya de cada clip (clave: "C9271")."""
    out = {}
    for f in (folder / "cache").glob("*.json"):
        j = json.loads(f.read_text(encoding="utf-8"))
        c = Counter()
        for s in j["samples"]:
            for b in s.get("jewels", []):
                c[b["label"]] += 1
        out[Path(j["path"]).stem] = c
    return out


def cmd_manos(a):
    from build_manos import APARTADOS, TRAIN_CLIPS

    tags = a.tags.split(",")
    L = {k: labels_of(OUT / f"manos_{k}") for k in tags}
    groups = {
        "anillos sueltos apartados": [c for c in APARTADOS["anillo"] if c >= "C9263"],
        "anillos puestos apartados": [c for c in APARTADOS["anillo"] if c < "C9263"],
        "topos apartados": APARTADOS["topo"],
        "anillos entrenados": [c for c, (g, _) in TRAIN_CLIPS.items() if g == "anillo"],
        "topos entrenados en r6 (fuera de r7)": [c for c, (g, _) in TRAIN_CLIPS.items() if g == "topo"],
    }
    for name, clips in groups.items():
        want = "topo" if "topos" in name else "anillo"
        print(f"== {name} (correcta: {want})")
        for k in tags:
            tot = Counter()
            for c in clips:
                tot += L[k].get(c, Counter())
            n = sum(tot.values())
            print(f"  {k}: {n} cajas  " + "  ".join(f"{l} {100 * v / max(1, n):.1f}%" for l, v in tot.most_common()))
        for c in clips:
            row = []
            for k in tags:
                t = L[k].get(c, Counter())
                n = sum(t.values())
                row.append(f"{k} {want} {100 * t[want] / max(1, n):5.1f}% (arete {100 * t['arete'] / max(1, n):4.1f}%)")
            print(f"    {c}: " + " | ".join(row))


def cmd_paridad(a):
    old, new = a.tags.split(",")[:2]
    for fk in a.folders.split(","):
        r = subprocess.run([sys.executable, "-B", PARITY, str(OUT / f"{fk}_{old}"), str(OUT / f"{fk}_{new}")],
                           capture_output=True, text=True, encoding="utf-8", errors="replace", creationflags=NO_WINDOW)
        print(f"##### {fk}: {old} -> {new}\n{r.stdout}{r.stderr[-500:]}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("paso", choices=["metricas", "exportar", "e2e", "manos", "paridad"])
    ap.add_argument("--thr", type=float, default=0.4)
    ap.add_argument("--folders", default="manos,oct27")
    ap.add_argument("--bins", default="r2,r6")
    ap.add_argument("--modelos", default="r2,r6")
    ap.add_argument("--tags", default="r2,r6")
    ap.add_argument("--tag", default="r6", help="modelo a exportar")
    # con DaVinci abierto, 3 clips en paralelo no caben en la VRAM y todo va unas 4 veces más lento
    ap.add_argument("--workers", default="", help="clips en paralelo del análisis (vacío = lo que decida CAMCUTS)")
    a = ap.parse_args()
    {"metricas": cmd_metricas, "exportar": cmd_exportar, "e2e": cmd_e2e, "manos": cmd_manos,
     "paridad": cmd_paridad}[a.paso](a)


if __name__ == "__main__":
    main()
