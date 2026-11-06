"""Genera las referencias de Python para la prueba de re-puntuación de Rust (tests/rescore.rs).

Copia las cachés de bench_oct27 a una carpeta temporal, ejecuta `python -m camcuts --rescore` de
camcuts_next con varias combinaciones de opciones y guarda el OTIO y el CSV de cada una en
tests/data/rescore/. Uso (CPU, sin modelos):

    C:\\Users\\Usuario\\Tools\\camcuts\\.venv\\Scripts\\python.exe gen_rescore_ref.py
"""
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

NEXT = Path(r"C:\Users\Usuario\Tools\camcuts_next")
CACHE = NEXT / "bench_oct27" / "cache"
HERE = Path(__file__).resolve().parent
OUT = HERE / "rescore"

# (nombre, argumentos extra, preset): las mismas combinaciones que recorre la prueba de Rust
VARIANTS = [
    ("joyas_v1v2", [], "joyas"),
    ("social_v1v2", ["--preset", "social"], "social"),
    ("todo_aprobados", ["--preset", "todo", "--modo", "aprobados"], "todo"),
    ("joyas_marcadores_filtros", ["--modo", "marcadores", "--joya", "dije,anillo", "--tipo", "producto,modelo"], "joyas"),
    ("joyas_25fps", ["--timeline-fps", "25"], "joyas"),
]


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, PYTHONPATH=str(NEXT), PYTHONHASHSEED="0", PYTHONIOENCODING="utf-8")
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        (work / "cache").mkdir()
        for f in CACHE.glob("*.json"):
            shutil.copy2(f, work / "cache" / f.name)
        for name, extra, preset in VARIANTS:
            cmd = [sys.executable, "-m", "camcuts", "--out", str(work), "--rescore", "--no-sheet"] + extra
            subprocess.run(cmd, check=True, env=env, cwd=str(NEXT))
            shutil.copy2(work / f"selects_{preset}.otio", OUT / f"{name}.otio")
            shutil.copy2(work / f"selects_{preset}.csv", OUT / f"{name}.csv")
            print("ok", name)


if __name__ == "__main__":
    main()
