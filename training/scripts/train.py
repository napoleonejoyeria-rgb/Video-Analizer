"""Entrena el detector propio (YOLO11) con turnos de GPU y freno de uso (~55 %).

- Después de cada lote duerme lo necesario para que la GPU trabaje como mucho `--duty` del tiempo.
- Al final de cada época, si el turno pasa de 27 min, suelta el candado y la caché de VRAM, deja pasar a
  otros y vuelve a pedir turno (el proceso sigue vivo: no hace falta reanudar desde disco).

Uso (desde training/): python scripts/train.py --data dataset/data.yaml --name r1 --epochs 30
"""
from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from gpu_turn import GpuTurn, someone_waiting  # noqa: E402
from gpu_turn import duty as gpu_duty  # noqa: E402

TRAIN = Path(__file__).resolve().parents[1]
TURN_S = 27 * 60


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--name", required=True)
    ap.add_argument("--model", default=str(TRAIN / "weights_dl" / "yolo11s.pt"))
    ap.add_argument("--epochs", type=int, default=30)
    ap.add_argument("--imgsz", type=int, default=960)
    ap.add_argument("--batch", type=int, default=12)
    ap.add_argument("--duty", type=float, default=0.55)
    ap.add_argument("--lr0", type=float, default=0.01)
    ap.add_argument("--close_mosaic", type=int, default=6)
    ap.add_argument("--freeze", type=int, default=0)
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--resume", action="store_true", help="--model es un last.pt a continuar")
    # ajuste fino (ronda 4): con "auto" Ultralytics ignora lr0 y elige AdamW 0.000769; para afinar desde un
    # modelo ya entrenado conviene fijar el optimizador, un lr0 menor y un calentamiento corto
    ap.add_argument("--optimizer", default="auto")
    ap.add_argument("--warmup", type=float, default=3.0, help="épocas de calentamiento")
    a = ap.parse_args()
    if a.duty <= 0:
        a.duty = gpu_duty()  # 0.88 sin DaVinci, 0.6 con él

    from ultralytics import YOLO
    import ultralytics.engine.trainer as ut

    # la comprobación de AMP descarga yolo26n.pt: la RTX 5060 va bien con AMP, se da por buena
    ut.check_amp = lambda *args, **kw: True
    model = YOLO(a.model)
    state = {"t": 0.0, "turn": None}

    def batch_start(trainer):
        state["t"] = time.perf_counter()

    def batch_end(trainer):
        # freno: la GPU descansa (1-duty)/duty veces lo que tardó el lote
        torch.cuda.synchronize()
        busy = time.perf_counter() - state["t"]
        time.sleep(busy * (1 - a.duty) / a.duty)

    def epoch_end(trainer):
        # reglas del 2026-11-03: sin límite de turno salvo que otro espere (.gpu_wait); el freno se
        # ajusta cada época según esté o no abierto DaVinci
        a.duty = gpu_duty()
        # si otro espera, se termina el paso: como mucho 25 min más desde que se vio su aviso
        if someone_waiting():
            state.setdefault("seen_wait", time.time())
        else:
            state.pop("seen_wait", None)
        if someone_waiting() and time.time() - state["seen_wait"] > 25 * 60:
            state.pop("seen_wait", None)
            state["turn"].__exit__(None, None, None)
            torch.cuda.empty_cache()
            time.sleep(60)  # el otro agente toma la GPU; GpuTurn espera a que la suelte
            state["turn"] = GpuTurn(max_wait_s=4 * 3600).__enter__()

    model.add_callback("on_train_batch_start", batch_start)
    model.add_callback("on_train_batch_end", batch_end)
    model.add_callback("on_fit_epoch_end", epoch_end)
    state["turn"] = GpuTurn(max_wait_s=4 * 3600).__enter__()
    try:
        if a.resume:
            # continúa el mismo entrenamiento desde last.pt (mismos argumentos guardados)
            model.train(resume=True)
            return
        model.train(data=a.data, epochs=a.epochs, imgsz=a.imgsz, batch=a.batch, workers=a.workers, device=0,
                    project=str(TRAIN / "runs"), name=a.name, exist_ok=True, cache="ram", cos_lr=True,
                    close_mosaic=a.close_mosaic, patience=100, lr0=a.lr0, freeze=a.freeze or None,
                    optimizer=a.optimizer, warmup_epochs=a.warmup,
                    degrees=0.0, fliplr=0.5, scale=0.5, translate=0.1, mosaic=1.0, mixup=0.0,
                    hsv_h=0.015, hsv_s=0.6, hsv_v=0.4, plots=False, verbose=False, amp=True, seed=1)
    finally:
        state["turn"].__exit__(None, None, None)


if __name__ == "__main__":
    main()
