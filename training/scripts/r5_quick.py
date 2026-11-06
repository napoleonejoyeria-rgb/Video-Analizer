"""Ronda 3 (macro): en un solo turno de GPU, umbral en validación, exportación a e2e_r5bin/models y
prueba rápida de punta a punta solo con C9181 (filtro: ≥ 40 s aprobados).

Uso: r5_quick.py --model runs/r5/weights/best.pt [--thr X] (sin --thr se elige en validación)
"""
import argparse
import io
import subprocess
import sys
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import choose_thr  # noqa: E402
import export_onnx  # noqa: E402
from gpu_turn import GpuTurn  # noqa: E402

TRAIN = Path(__file__).resolve().parents[1]
BIN = TRAIN / "e2e_r5bin"
CLIP = r"D:\CONTENIDO NAPO\2026\Octubre\27\videos\C9181.MP4"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--thr", type=float, default=None)
    ap.add_argument("--tag", default="r5")
    a = ap.parse_args()
    with GpuTurn():
        thr = a.thr
        if thr is None:
            buf = io.StringIO()
            sys.argv = ["choose_thr.py", "--model", a.model]
            with redirect_stdout(buf):
                choose_thr.main()
            out = buf.getvalue()
            print(out)
            thr = float(out.strip().split()[-1])
        for fp16 in (False, True):
            sys.argv = ["export_onnx.py", "--model", a.model, "--out",
                        str(BIN / "models" / ("jewel_det_fp16.onnx" if fp16 else "jewel_det.onnx")), "--thr", f"{thr:.2f}"]
            if fp16:
                sys.argv.append("--fp16")
            export_onnx.main()
        out = TRAIN / "e2e_runs" / f"C9181_{a.tag}"
        r = subprocess.run([str(BIN / "camcuts-e2e.exe"), CLIP, "--out", str(out), "--detector", "propio", "--no-proxy",
                            "--workers", "2"], capture_output=True, text=True, encoding="utf-8", errors="replace",
                           creationflags=0x08000000 | 0x00004000)
        print(r.stdout.strip().splitlines()[-3:] if r.stdout else r.stderr[-300:])
    p = subprocess.run([sys.executable, "-B", r"C:\Users\Usuario\Tools\camcuts_next\tools\parity.py",
                        str(TRAIN / "e2e_runs" / "Octubre_27_videos_world_w3"), str(out)], capture_output=True, text=True)
    print(f"umbral {thr:.2f}:", p.stdout.strip().splitlines()[0] if p.stdout else p.stderr[-300:])


if __name__ == "__main__":
    main()
