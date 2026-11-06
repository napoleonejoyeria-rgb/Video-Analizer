"""Muestreo de cuadros del material de la empresa para entrenar el detector propio de joyas.

Lee (solo lectura) los clips de D:\\CONTENIDO NAPO\\2026, elige clips repartidos por carpeta (fecha,
sesión, cámara) y extrae unos cuantos cuadros de cada uno con ffmpeg a 1920 px de lado largo (JPEG).
El cuadro de 960 px que ve el detector se hace después con INTER_AREA, como el muestreador.

Uso: python sample_frames.py [--target 3600] [--workers 6]
Salida: training/data/img1920/*.jpg y training/data/frames.json (una entrada por cuadro).
"""
from __future__ import annotations

import argparse
import json
import random
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(r"D:\CONTENIDO NAPO\2026")
TRAIN = Path(__file__).resolve().parents[1]
OUT = TRAIN / "data" / "img1920"
VIDEO_EXT = {".mp4", ".mov", ".mxf", ".m4v", ".mts"}
# ejemplo difícil que mandó el coordinador (bol de cristal con piedras, pulsera tenis + cadena)
HARD = {r"julio\28\c0200.mp4": [0.5, 1.0, 1.75, 2.0, 3.0]}
NO_WINDOW = 0x08000000  # CREATE_NO_WINDOW: nunca abrir consolas en la pantalla del usuario
BELOW_NORMAL = 0x00004000


def run(cmd: list[str], timeout: float = 60) -> subprocess.CompletedProcess:
    """Ejecuta un proceso sin ventana y con prioridad baja (el usuario está editando)."""
    return subprocess.run(cmd, capture_output=True, timeout=timeout,
                          creationflags=NO_WINDOW | BELOW_NORMAL)


def probe(p: Path) -> dict | None:
    """Duración y tamaño (ya girado) del clip con ffprobe."""
    r = run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
             "stream=width,height:stream_side_data=rotation:stream_tags=rotate:format=duration",
             "-of", "json", str(p)])
    try:
        j = json.loads(r.stdout.decode("utf-8", "replace"))
        s = j["streams"][0]
        dur = float(j["format"]["duration"])
    except Exception:
        return None
    rot = 0
    for sd in s.get("side_data_list", []) or []:
        if "rotation" in sd:
            rot = int(float(sd["rotation"]))
    if "tags" in s and "rotate" in s["tags"]:
        rot = int(s["tags"]["rotate"])
    w, h = int(s["width"]), int(s["height"])
    if abs(rot) % 180 == 90:
        w, h = h, w
    return {"duration": dur, "width": w, "height": h, "rotation": rot}


def extract(p: Path, t: float, out: Path, exact: bool = False) -> bool:
    """Un cuadro cerca del segundo t, girado como lo ve el editor, a 1920 px de lado largo.

    Sin `exact` se decodifica solo el fotograma clave anterior a t (`-skip_frame nokey`): con 4K de GOP
    largo es ~10 veces más barato y para muestrear da igual unas décimas de segundo."""
    vf = "scale='if(gte(iw,ih),1920,-2)':'if(gte(iw,ih),-2,1920)':flags=area"
    fast = [] if exact else ["-skip_frame", "nokey"]
    r = run(["ffmpeg", "-v", "error", "-y", "-threads", "2", *fast, "-ss", f"{t:.3f}", "-i", str(p),
             "-frames:v", "1", "-vf", vf, "-q:v", "3", str(out)], timeout=120)
    return r.returncode == 0 and out.exists() and out.stat().st_size > 1000


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", type=int, default=3600)
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args()
    rnd = random.Random(a.seed)
    OUT.mkdir(parents=True, exist_ok=True)

    vids = [p for p in ROOT.rglob("*") if p.is_file() and p.suffix.lower() in VIDEO_EXT
            and "\\camcuts\\" not in str(p).lower() and "proxies" not in str(p).lower()]
    by_dir: dict[Path, list[Path]] = {}
    for v in sorted(vids):
        by_dir.setdefault(v.parent, []).append(v)
    print(f"{len(vids)} clips en {len(by_dir)} carpetas", flush=True)

    # clips por carpeta: todas las carpetas aportan; las grandes no dominan
    chosen: list[Path] = []
    for d, vs in by_dir.items():
        k = len(vs) if "julio\\28" in str(d).lower() else min(len(vs), max(2, round(len(vs) * 0.4)), 30)
        chosen += rnd.sample(vs, k)
    print(f"{len(chosen)} clips elegidos; leyendo duraciones…", flush=True)
    with ThreadPoolExecutor(a.workers) as ex:
        infos = list(ex.map(probe, chosen))
    clips = [(p, i) for p, i in zip(chosen, infos) if i and i["duration"] > 0.8]
    total_dur = sum(i["duration"] for _, i in clips)

    # cuadros por clip: 2 fijos + reparto proporcional a la raíz de la duración, hasta el objetivo
    weights = [i["duration"] ** 0.5 for _, i in clips]
    extra = max(0, a.target - 2 * len(clips))
    ws = sum(weights)
    jobs = []
    for (p, i), wgt in zip(clips, weights):
        key = str(p.relative_to(ROOT)).lower()
        times = HARD.get(key)
        if times is None:
            n = 2 + int(round(extra * wgt / ws))
            n = min(n, 12)
            lo, hi = 0.25, max(0.3, i["duration"] - 0.4)
            # tiempos repartidos en franjas iguales con azar dentro de cada franja
            times = sorted(lo + (hi - lo) * (k + rnd.random()) / n for k in range(n))
        for t in times:
            stem = f"{len(jobs):05d}"
            jobs.append({"id": stem, "clip": str(p), "rel": str(p.relative_to(ROOT)), "t": round(t, 3),
                         "folder": str(p.parent.relative_to(ROOT)), **i})
    print(f"{len(clips)} clips, {total_dur / 3600:.1f} h, {len(jobs)} cuadros a extraer", flush=True)
    # el plan completo primero (el maestro y la revisión leen de aquí la carpeta y el clip de cada cuadro)
    (TRAIN / "data" / "plan.json").write_text(json.dumps(jobs, ensure_ascii=False, indent=0), encoding="utf-8")

    def work(j):
        out = OUT / f"{j['id']}.jpg"
        ok = out.exists() or extract(Path(j["clip"]), j["t"], out, exact=j["rel"].lower() in HARD)
        return j if ok else None

    # orden barajado: si hay que parar antes, lo extraído sigue repartido por todas las carpetas
    order = jobs[:]
    random.Random(a.seed + 1).shuffle(order)
    done = []
    with ThreadPoolExecutor(a.workers) as ex:
        for k, r in enumerate(ex.map(work, order)):
            if r:
                done.append(r)
            if k % 200 == 0:
                print(f"  {k}/{len(jobs)}", flush=True)
    (TRAIN / "data" / "frames.json").write_text(json.dumps(done, ensure_ascii=False, indent=0), encoding="utf-8")
    print(f"listo: {len(done)} cuadros", flush=True)


if __name__ == "__main__":
    sys.exit(main())
