"""Mide en Rust (DirectML, cuadro a cuadro) YOLO-World x y el detector propio sobre los cuadros de
evaluación, y puntúa las detecciones QUE DEVUELVE RUST (lo que de verdad usaría CAMCUTS) contra la
verdad revisada a mano. Con turno de GPU.

Uso: bench_eval.py <models_dir_propio>
"""
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from decisions import gt  # noqa: E402
from evaluate import evaluate, table  # noqa: E402
from gpu_turn import GpuTurn  # noqa: E402

TRAIN = Path(__file__).resolve().parents[1]
EXE = TRAIN.parent / "target" / "release" / "camcuts-e2e.exe"
B = TRAIN / "bench"


def run(models, kind, prec, out):
    r = subprocess.run([str(EXE), "bench", str(B / "frames.bin"), str(B / "frames.json"), str(models), kind, prec, str(out)],
                       capture_output=True, text=True, creationflags=0x08000000)
    print(r.stdout.strip(), r.stderr.strip()[-300:], flush=True)


def main():
    models = Path(sys.argv[1])
    with GpuTurn(max_wait_s=3600):
        run(models, "propio", "fp32", B / "propio_fp32.json")
        run(models, "propio", "fp16", B / "propio_fp16.json")
        run(TRAIN.parent / "models", "world", "fp32", B / "world_fp32.json")
    G = {k: v for k, v in gt("eval").items() if not v["unsure"]}
    for name in ("propio_fp32", "propio_fp16", "world_fp32"):
        P = json.loads((B / f"{name}.json").read_text())
        # el propio ya sale filtrado por su umbral y reescalado (umbral -> 0.15); world sin CLIP aquí
        print(table(f"Rust {name} (umbral 0.15 sobre la confianza devuelta)", evaluate(P, G, 0.15)))


if __name__ == "__main__":
    main()
