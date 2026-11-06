"""Turnos de GPU entre agentes (GPU_TURNOS.md, reglas del 2026-11-03).

- `GpuTurn`: si `.gpu_lock` es de otro (y tiene menos de 2 h) se espera y se deja `.gpu_wait` con nuestro
  nombre para que el otro suelte la GPU al acabar su paso. Con el candado libre se toma; si DaVinci
  (`Resolve.exe`) está abierto, además se espera a que la GPU esté por debajo del 60 % y queden ≥ 2 GB.
- `duty()`: ciclo útil al que frenar el trabajo: 0.6 con DaVinci abierto, 0.88 sin él (85–90 % de media,
  sin pasar largos ratos al 99 %).
- `someone_waiting()`: otro agente dejó `.gpu_wait`: hay que terminar el paso y soltar el candado.
"""
from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path

LOCK = Path(r"C:\Users\Usuario\Tools\camcuts_rs\.gpu_lock")
WAIT = Path(r"C:\Users\Usuario\Tools\camcuts_rs\.gpu_wait")
NAME = "agente-detector-propio (training/)"
NO_WINDOW = 0x08000000


def gpu_state() -> tuple[float, float]:
    """(uso %, MiB libres) según nvidia-smi."""
    r = subprocess.run(["nvidia-smi", "--query-gpu=utilization.gpu,memory.used,memory.total",
                        "--format=csv,noheader,nounits"], capture_output=True, text=True, creationflags=NO_WINDOW)
    u, used, total = [float(x) for x in r.stdout.strip().splitlines()[0].split(",")]
    return u, total - used


def resolve_running() -> bool:
    """¿Está DaVinci Resolve abierto?"""
    r = subprocess.run(["tasklist", "/FI", "IMAGENAME eq Resolve.exe", "/NH"], capture_output=True, text=True,
                       creationflags=NO_WINDOW)
    return "resolve.exe" in r.stdout.lower()


def duty() -> float:
    return 0.6 if resolve_running() else 0.88


def someone_waiting() -> bool:
    if not WAIT.exists():
        return False
    try:
        return NAME not in WAIT.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False


class GpuTurn:
    """Contexto `with GpuTurn(): ...` que respeta el candado compartido."""

    def __init__(self, max_wait_s: float = 4 * 3600, poll_s: float = 20):
        self.max_wait_s, self.poll_s = max_wait_s, poll_s
        self.t0 = None

    def __enter__(self):
        start = time.time()
        waiting = False
        while True:
            busy = ""
            if LOCK.exists() and time.time() - LOCK.stat().st_mtime < 2 * 3600:
                txt = LOCK.read_text(encoding="utf-8", errors="replace")
                if NAME not in txt:
                    busy = f"candado de otro: {txt.strip()[:80]}"
                elif f"pid {os.getpid()}" not in txt:
                    busy = f"otro proceso propio: {txt.strip()[:80]}"
            if busy and "candado de otro" in busy and not waiting:
                WAIT.write_text(f"{NAME} espera desde {time.strftime('%Y-%m-%d %H:%M:%S')}\n", encoding="utf-8")
                waiting = True
            if not busy and resolve_running():
                u, free = gpu_state()
                if u > 60 or free < 2048:
                    busy = f"DaVinci abierto y GPU ocupada ({u:.0f} %, {free:.0f} MiB libres)"
            if not busy:
                break
            if time.time() - start > self.max_wait_s:
                raise RuntimeError("no hubo turno de GPU: " + busy)
            print("[turno] esperando:", busy, flush=True)
            time.sleep(self.poll_s)
        if waiting and WAIT.exists() and NAME in WAIT.read_text(encoding="utf-8", errors="replace"):
            WAIT.unlink()
        LOCK.write_text(f"{NAME} desde {time.strftime('%Y-%m-%d %H:%M:%S')} pid {os.getpid()}\n", encoding="utf-8")
        self.t0 = time.time()
        print("[turno] GPU tomada", flush=True)
        return self

    def elapsed(self) -> float:
        return time.time() - self.t0

    def __exit__(self, *exc):
        try:
            if LOCK.exists() and NAME in LOCK.read_text(encoding="utf-8", errors="replace"):
                LOCK.unlink()
        finally:
            print(f"[turno] GPU liberada tras {self.elapsed() / 60:.1f} min", flush=True)
        return False


class Throttle:
    """Duerme tras cada trozo de trabajo para que la GPU no pase del ciclo útil pedido."""

    def __init__(self, duty_: float | None = None):
        self.duty = duty_ if duty_ is not None else duty()
        self.t = time.perf_counter()

    def start(self):
        self.t = time.perf_counter()

    def pause(self):
        busy = time.perf_counter() - self.t
        time.sleep(busy * (1 - self.duty) / self.duty)
        self.t = time.perf_counter()
