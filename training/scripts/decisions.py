"""Lee los veredictos de la revisión a mano (review/<set>_decisions*.jsonl) y los convierte en cajas.

gt(set) -> {id: {"boxes": [{"box", "label", "src"}], "unsure": bool, "w", "h"}}
`src` dice de dónde sale cada caja: "ok" (propuesta del maestro aceptada), "relabel" (aceptada con
otra clase), "fix" (corregida a mano) o "add" (añadida a mano).
"""
from __future__ import annotations

import json

from labels import JEWEL7, TRAIN, norm_type


def _label(v: str) -> str | None:
    v = norm_type(str(v).strip().lower())
    return v if v in JEWEL7 else None


def verdicts(set_name: str) -> dict[str, dict]:
    out = {}
    for f in sorted((TRAIN / "review").glob(f"{set_name}_decisions*.jsonl")):
        for line in f.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                d = json.loads(line)
                out[d["id"]] = d
    return out


def gt(set_name: str) -> dict[str, dict]:
    res = {}
    V = verdicts(set_name)
    for fid, d in V.items():
        meta = json.loads((TRAIN / "review" / set_name / f"{fid}.json").read_text(encoding="utf-8"))
        W, H = meta["w"], meta["h"]
        boxes = []
        fixes = {str(k): v for k, v in (d.get("fix") or {}).items()}
        for b in meta["boxes"]:
            v = str((d.get("boxes") or {}).get(str(b["n"]), "no")).strip().lower()
            if v == "no":
                continue
            lab = b["label"] if v == "ok" else _label(v)
            if lab is None:
                continue
            box = fixes.get(str(b["n"]), b["box"])
            src = "fix" if str(b["n"]) in fixes else ("ok" if v == "ok" else "relabel")
            boxes.append({"box": box, "label": lab, "src": src, "n": b["n"]})
        for a in d.get("add") or []:
            lab = _label(a.get("label", ""))
            if lab and len(a.get("box", [])) == 4:
                boxes.append({"box": a["box"], "label": lab, "src": "add"})
        clean = []
        for b in boxes:
            x0, y0, x1, y1 = [float(v) for v in b["box"]]
            x0, x1 = sorted((max(0, x0), min(W, x1)))
            y0, y1 = sorted((max(0, y0), min(H, y1)))
            if x1 - x0 >= 2 and y1 - y0 >= 2:
                b["box"] = [x0, y0, x1, y1]
                clean.append(b)
        res[fid] = {"boxes": clean, "unsure": bool(d.get("unsure")), "w": W, "h": H, "meta": meta,
                    "verdict": d}
    if set_name in ("gold", "more", "r2", "r3"):
        apply_audit(res)
    return res


def apply_audit(res: dict[str, dict]) -> int:
    """Correcciones de la auditoría de piezas largas (review/audit_decisions*.jsonl): cajas parciales de
    pulsera/cadena/collar extendidas a toda la pieza, fragmentos borrados, reetiquetas y piezas añadidas.
    Los números de caja son la posición (1..n) en `gt()`. Devuelve cuántos cuadros cambió."""
    n = 0
    for f in sorted((TRAIN / "review").glob("audit_decisions*.jsonl")):
        for line in f.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            d = json.loads(line)
            g = res.get(d.get("id"))
            if g is None or d.get("ok"):
                continue
            W, H = g["w"], g["h"]
            boxes = g["boxes"]
            for k, b in (d.get("fix") or {}).items():
                i = int(k) - 1
                if 0 <= i < len(boxes) and len(b) == 4:
                    x0, y0, x1, y1 = [float(v) for v in b]
                    boxes[i] = dict(boxes[i], box=[max(0, min(x0, x1)), max(0, min(y0, y1)), min(W, max(x0, x1)), min(H, max(y0, y1))], src="audit")
            for k, lab in (d.get("relabel") or {}).items():
                i = int(k) - 1
                if 0 <= i < len(boxes) and _label(lab):
                    boxes[i] = dict(boxes[i], label=_label(lab))
            drop = {int(k) - 1 for k in d.get("no") or []}
            boxes = [b for i, b in enumerate(boxes) if i not in drop]
            for a in d.get("add") or []:
                if _label(a.get("label", "")) and len(a.get("box", [])) == 4:
                    boxes.append({"box": a["box"], "label": _label(a["label"]), "src": "audit"})
            g["boxes"] = boxes
            n += 1
    return n
