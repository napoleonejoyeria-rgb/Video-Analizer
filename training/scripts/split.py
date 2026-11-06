"""Reparto de cuadros: evaluación (carpetas enteras apartadas + el clip difícil C0200), validación
durante el entrenamiento (clips sueltos) y entrenamiento. Escribe data/split.json y data/priority.txt
(orden en que el maestro etiqueta: primero evaluación y candidatos a revisión).
"""
import json
import random

from labels import TRAIN

# carpetas apartadas para evaluar: variadas (producto, modelo, hablando, aretes, horizontal y vertical)
EVAL_FOLDERS = [
    "Octubre\\27", "Octubre\\27\\videos", "Octubre\\27\\videos\\Graficos",
    "marzo\\13\\Dijes Colombia", "Octubre\\8\\Candongas",
    "Octubre\\16\\Cosas sin Editar\\Dani Hablando el Tesoro", "septiembre\\26\\Anillo",
    "marzo\\10\\PULSERAS", "Octubre\\16\\Cosas sin Editar\\Ella y El",
    "septiembre\\24\\Concepto Vale Vestier", "Agosto\\5\\Nueva carpeta", "Octubre\\9\\Cholocatina",
    "septiembre\\22\\Video",
]
EVAL_CLIPS = ["julio\\28\\c0200.mp4"]


def main():
    plan = json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))
    rnd = random.Random(3)
    split = {}
    clips = sorted({p["rel"] for p in plan})
    val_clips = set(rnd.sample([c for c in clips], 45))
    for p in plan:
        if p["folder"] in EVAL_FOLDERS or p["rel"].lower() in EVAL_CLIPS:
            split[p["id"]] = "eval"
        elif p["rel"] in val_clips:
            split[p["id"]] = "val"
        else:
            split[p["id"]] = "train"
    (TRAIN / "data" / "split.json").write_text(json.dumps(split), encoding="utf-8")
    ev = [i for i, s in split.items() if s == "eval"]
    tr = [i for i, s in split.items() if s == "train"]
    rnd.shuffle(tr)
    # Julio 28 (producto difícil) entra pronto en la cola de candidatos a revisión
    j28 = [p["id"] for p in plan if p["folder"] == "Julio\\28" and split[p["id"]] == "train"]
    first = ev + j28[:60] + [i for i in tr if i not in set(j28[:60])][:700]
    (TRAIN / "data" / "priority.txt").write_text("\n".join(first), encoding="utf-8")
    from collections import Counter
    print(Counter(split.values()))


if __name__ == "__main__":
    main()
