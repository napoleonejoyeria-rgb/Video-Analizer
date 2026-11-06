"""Reglas comunes: clases del detector propio y cómo se convierten las salidas del maestro en propuestas.

Clases (= etiquetas en español que entiende el núcleo de CAMCUTS):
  0 anillo, 1 collar, 2 cadena, 3 arete, 4 topo, 5 pulsera, 6 dije, 7 persona, 8 cara.
Los tipos que el núcleo aún no reconoce como joya se juntan con el más cercano:
  candonga (aro) -> arete, piercing -> topo, tobillera -> pulsera, rosario -> collar.
"""
from __future__ import annotations

import json
from pathlib import Path

TRAIN = Path(__file__).resolve().parents[1]
JEWEL7 = ["anillo", "collar", "cadena", "arete", "topo", "pulsera", "dije"]
CLASSES = JEWEL7 + ["persona", "cara"]
TYPE_MAP = {"candonga": "arete", "piercing": "topo", "tobillera": "pulsera", "rosario": "collar"}
SHORT = {"anillo": "AN", "collar": "CO", "cadena": "CA", "arete": "AR", "topo": "TO", "pulsera": "PU",
         "dije": "DI", "joya": "JO", "persona": "PE", "cara": "FA"}


def norm_type(t: str) -> str:
    return TYPE_MAP.get(t, t)


def iou(a, b) -> float:
    ix = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    iy = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = ix * iy
    u = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / u if u > 0 else 0.0


def overlap_small(a, b) -> float:
    ix = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    iy = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    s = min((a[2] - a[0]) * (a[3] - a[1]), (b[2] - b[0]) * (b[3] - b[1]))
    return ix * iy / s if s > 0 else 0.0


def load_teacher() -> dict[str, dict]:
    out = {}
    for line in (TRAIN / "teacher" / "teacher.jsonl").read_text(encoding="utf-8").splitlines():
        if line.strip():
            r = json.loads(line)
            out[r["id"]] = r
    return out


def accepted_by_pipeline(d: dict) -> bool:
    """Lo que la app actual acepta como joya (conf ≥ 0.15, o débil confirmada por CLIP)."""
    return d["conf"] >= 0.15 or (d["conf"] >= 0.02 and d.get("clip_p", 0) >= 0.6 and not d["zoom"]) or \
        (d["zoom"] and d["conf"] >= 0.15)


def baseline(rec: dict) -> list[dict]:
    """Salida de la app actual (YOLO-World x + CLIP, sin --zoom) en un cuadro: lo que hay que batir.

    Mismas reglas que verify_weak: hasta 4 débiles que no solapen (> 0.5) una fuerte, aceptadas con p ≥ 0.6
    y su confianza subida a 0.15."""
    strong = [d for d in rec["jewels"] if not d["zoom"] and d["conf"] >= 0.15]
    weak = sorted([d for d in rec["jewels"] if not d["zoom"] and 0.02 <= d["conf"] < 0.15
                   and not any(overlap_small(d["box"], s["box"]) > 0.5 for s in strong)], key=lambda d: -d["conf"])[:4]
    out = [{"box": d["box"], "conf": d["conf"], "label": d["label"]} for d in strong]
    out += [{"box": d["box"], "conf": max(d["conf"], 0.15), "label": d["label"]} for d in weak if d.get("clip_p", 0) >= 0.6]
    return out


def clip_type(d: dict) -> tuple[str, float]:
    """Tipo más probable según CLIP (ya juntado a las 7 clases) y su probabilidad sumada."""
    agg: dict[str, float] = {}
    for t, p in (d.get("types") or {}).items():
        agg[norm_type(t)] = agg.get(norm_type(t), 0.0) + p
    if not agg:
        return "", 0.0
    t = max(agg, key=agg.get)
    return t, agg[t]


def proposals(rec: dict, loose: bool = False) -> list[dict]:
    """Propuestas de joya del maestro para un cuadro, sin duplicados.

    Acepta lo que acepta la app, más lo de la segunda mirada (zoom) con conf ≥ 0.05 y CLIP ≥ 0.5.
    Con `loose`, también candidatos débiles (conf ≥ 0.04 o CLIP ≥ 0.4) marcados como `weak`.
    Etiqueta: la del detector, salvo "joya" (genérica), que toma el tipo de CLIP.
    """
    cands = []
    for d in rec["jewels"]:
        p = d.get("clip_p", 0.0)
        ok = accepted_by_pipeline(d) or (d["zoom"] and d["conf"] >= 0.05 and p >= 0.5)
        weak = False
        if not ok:
            if loose and (d["conf"] >= 0.04 or p >= 0.4):
                weak = True
            else:
                continue
        lab = d["label"]
        ct, cp = clip_type(d)
        if lab == "joya":
            lab = ct or "dije"
        cands.append({"box": list(d["box"]), "label": lab, "det_label": d["label"], "conf": d["conf"], "clip_p": p,
                      "clip_type": ct, "clip_type_p": round(cp, 3), "zoom": d["zoom"], "weak": weak,
                      "sam_box": d.get("sam_box"), "sam_frac": d.get("sam_frac")})
    # duplicados (misma joya vista con dos etiquetas o en la pasada con zoom): se queda la más fuerte
    cands.sort(key=lambda c: (c["weak"], -c["conf"]))
    out: list[dict] = []
    for c in cands:
        dup = next((o for o in out if iou(o["box"], c["box"]) > 0.55), None)
        if dup is None:
            out.append(c)
        else:
            dup.setdefault("alt", []).append(c["label"])
    return out


def people(rec: dict, thr: float = 0.3) -> list[dict]:
    """Personas y caras del maestro (YOLO-World) para entrenar esas dos clases."""
    return [{"box": d["box"], "label": d["label"], "conf": d["conf"]} for d in rec["people"] if d["conf"] >= thr]
