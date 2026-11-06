"""Análisis de punta a punta de varias carpetas con un detector (world o propio), con turno de GPU.

Uso: e2e_run.py --detector propio|world --tag r1 carpeta1 carpeta2 ...
Cada carpeta va a training/e2e_runs/<nombre>_<detector>[_<tag>] con --no-proxy; el tiempo de pared de
cada análisis se guarda en e2e_runs/times.json.
"""
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from gpu_turn import GpuTurn  # noqa: E402

TRAIN = Path(__file__).resolve().parents[1]
EXE = TRAIN.parent / "target" / "release" / "camcuts-e2e.exe"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--detector", required=True)
    ap.add_argument("--tag", default="")
    ap.add_argument("--workers", default="", help="clips en paralelo (vacío = lo que decida CAMCUTS)")
    ap.add_argument("--exe", default=str(EXE), help="binario (con su carpeta models/ al lado, si la tiene)")
    ap.add_argument("folders", nargs="+")
    a = ap.parse_args()
    runs = TRAIN / "e2e_runs"
    runs.mkdir(exist_ok=True)
    tf = runs / "times.json"
    times = json.loads(tf.read_text()) if tf.exists() else {}
    for folder in a.folders:
        name = folder.rstrip("\\/").replace("D:\\CONTENIDO NAPO\\2026\\", "").replace("\\", "_").replace(" ", "")
        out = runs / f"{name}_{a.detector}{'_' + a.tag if a.tag else ''}"
        with GpuTurn(max_wait_s=3 * 3600):
            t0 = time.time()
            extra = ["--workers", a.workers] if a.workers else []
            r = subprocess.run([a.exe, folder, "--out", str(out), "--detector", a.detector, "--no-proxy", *extra],
                               capture_output=True, text=True, encoding="utf-8", errors="replace",
                               creationflags=0x08000000 | 0x00004000)
            dt = time.time() - t0
        (out / "log.txt").write_text(r.stdout + "\n" + r.stderr, encoding="utf-8")
        times[out.name] = round(dt, 1)
        tf.write_text(json.dumps(times, indent=1))
        print(f"{out.name}: {dt:.0f} s (código {r.returncode})", flush=True)
        time.sleep(20)


if __name__ == "__main__":
    main()
