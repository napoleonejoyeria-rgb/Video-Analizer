"""Elige los cuadros de entrenamiento que se revisan a mano enteros ("oro"): los casos donde el maestro
falla más (orejas, cajas enormes, producto en bol/vitrina de Julio 28, etiquetas en desacuerdo) y
unos cuantos al azar. Escribe review/gold_ids.txt.
"""
import json
import random

from labels import TRAIN, clip_type, load_teacher, proposals

rnd = random.Random(11)
T = load_teacher()
split = json.loads((TRAIN / "data" / "split.json").read_text())
plan = {p["id"]: p for p in json.loads((TRAIN / "data" / "plan.json").read_text(encoding="utf-8"))}
train = [i for i in T if split.get(i) == "train"]
rnd.shuffle(train)


def ears(r):
    return any(k[3][2] > 0.5 or k[4][2] > 0.5 for k in r["kps"])


def big(r):
    return any((p["box"][2] - p["box"][0]) * (p["box"][3] - p["box"][1]) > 0.3 * r["w"] * r["h"] for p in proposals(r))


def disagree(r):
    for p in proposals(r):
        t, tp = clip_type(p) if False else (p["clip_type"], p["clip_type_p"])
        if t and tp >= 0.5 and t != p["label"]:
            return True
    return False


picked: list[str] = []


def take(pred, n):
    k = 0
    for i in train:
        if k >= n:
            break
        if i not in picked and pred(T[i]):
            picked.append(i)
            k += 1


take(lambda r: plan[r["id"]]["folder"] == "Julio\\28", 50)
take(ears, 90)
take(big, 45)
take(disagree, 50)
take(lambda r: True, 65)
(TRAIN / "review" / "gold_ids.txt").write_text("\n".join(picked), encoding="utf-8")
print(len(picked), "cuadros oro de", len(train), "de entrenamiento ya etiquetados")
