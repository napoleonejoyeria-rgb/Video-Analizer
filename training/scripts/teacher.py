"""Maestro: pseudoetiquetas de los cuadros muestreados con el pipeline actual, a umbrales bajos.

Por cuadro (960 px, contraste estirado si se ve plano/LOG, como el análisis):
  1. YOLO-World x (prompts de jewel.py) a conf ≥ 0.02;
  2. CLIP ViT-B/32: probabilidad de joya (verificador de classify.py) y tipo (JewelTyper) de cada candidato;
  3. pose si hay persona, y segunda mirada a 1920 px en orejas, cuello y muñecas (--zoom de la app);
  4. MobileSAM con la caja como prompt: caja ajustada a la silueta.
Todo se guarda (también lo débil) en teacher/teacher.jsonl; las reglas de qué se acepta van aparte.

Uso (desde training/, para que Ultralytics encuentre weights/clip): python scripts/teacher.py
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

import cv2
import numpy as np
import torch

TRAIN = Path(__file__).resolve().parents[1]
NEXT = Path(r"C:\Users\Usuario\Tools\camcuts_next")
sys.path.insert(0, str(NEXT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from camcuts.classify import JEWEL_TEXT, OTHER_TEXT, TYPE_TEXT  # noqa: E402
from camcuts.jewel import JEWEL_PROMPTS, PEOPLE_PROMPTS, zoom_regions  # noqa: E402
from camcuts.metrics import is_flat, normalize_contrast  # noqa: E402
from gpu_turn import GpuTurn, Throttle  # noqa: E402

PROMPTS = {**JEWEL_PROMPTS, **PEOPLE_PROMPTS}
JEWELS = set(JEWEL_PROMPTS.values())
OUT = TRAIN / "teacher" / "teacher.jsonl"
TURN_S = 27 * 60  # turno máximo antes de soltar la GPU y volver a pedirla


class Clip:
    """CLIP ViT-B/32: probabilidad de joya (mismos textos que el verificador) y tipo de joya."""

    def __init__(self):
        import clip

        self.clip = clip
        self.model, self.pre = clip.load("ViT-B/32", device="cuda",
                                         download_root=str(Path.home() / ".cache" / "clip"))
        self.model.eval()
        with torch.no_grad():
            t = self.model.encode_text(clip.tokenize(JEWEL_TEXT + OTHER_TEXT).cuda()).float()
            self.verify_text = (t / t.norm(dim=-1, keepdim=True))
            feats = []
            self.types = list(TYPE_TEXT)
            for lab in self.types:
                t = self.model.encode_text(clip.tokenize(TYPE_TEXT[lab]).cuda()).float()
                t = t / t.norm(dim=-1, keepdim=True)
                m = t.mean(dim=0)
                feats.append(m / m.norm())
            self.type_text = torch.stack(feats)

    def run(self, crops: list[np.ndarray]) -> list[tuple[float, dict]]:
        """(p de joya, {tipo: p} de los 3 tipos más probables) por recorte."""
        from PIL import Image

        if not crops:
            return []
        x = torch.stack([self.pre(Image.fromarray(cv2.cvtColor(c, cv2.COLOR_BGR2RGB))) for c in crops]).cuda()
        with torch.no_grad():
            e = self.model.encode_image(x).float()
            e = e / e.norm(dim=-1, keepdim=True)
            pv = (100 * e @ self.verify_text.T).softmax(-1)[:, :len(JEWEL_TEXT)].sum(-1)
            pt = (100 * e @ self.type_text.T).softmax(-1)
        out = []
        for i in range(len(crops)):
            top = torch.topk(pt[i], 3)
            out.append((float(pv[i]), {self.types[int(k)]: round(float(v), 3) for v, k in zip(top.values, top.indices)}))
        return out


def pad_crop(img: np.ndarray, box, pad: float = 0.15) -> np.ndarray:
    """Recorte de la caja agrandada un 15 % por lado (como verify_weak)."""
    x0, y0, x1, y1 = [int(v) for v in box]
    h, w = img.shape[:2]
    px, py = int((x1 - x0) * pad), int((y1 - y0) * pad)
    return img[max(0, y0 - py):min(h, y1 + py), max(0, x0 - px):min(w, x1 + px)]


def main() -> None:
    from ultralytics import SAM, YOLO, YOLOWorld

    torch.set_num_threads(4)  # pocos hilos de CPU: el usuario está editando
    cv2.setNumThreads(2)

    done = set()
    if OUT.exists():
        done = {json.loads(l)["id"] for l in OUT.read_text(encoding="utf-8").splitlines() if l.strip()}
    img960 = TRAIN / "data" / "img960"
    img960.mkdir(parents=True, exist_ok=True)
    src = TRAIN / "data" / "img1920"

    prio_file = TRAIN / "data" / "priority.txt"
    prio = {i: k for k, i in enumerate(prio_file.read_text().split())} if prio_file.exists() else {}

    def pending() -> list[dict]:
        # los cuadros ya extraídos (con más de 5 s, ya escritos del todo) que faltan por etiquetar, primero
        # los de evaluación y candidatos a revisión; el muestreo puede seguir corriendo a la vez
        now = time.time()
        ids = [p.stem for p in src.glob("*.jpg") if p.stem not in done and now - p.stat().st_mtime > 5]
        ids.sort(key=lambda i: (prio.get(i, 10 ** 6), i))
        return [{"id": i} for i in ids]

    todo = pending()
    print(f"{len(todo)} cuadros por etiquetar ({len(done)} hechos)", flush=True)
    k = 0
    while True:
        if k >= len(todo):
            todo, k = pending(), 0
            if not todo:
                if (TRAIN / "data" / "frames.json").exists():
                    break
                time.sleep(20)
                continue
        with GpuTurn() as turn:
            world = YOLOWorld(str(NEXT / "yolov8x-worldv2.pt"))
            world.set_classes(list(PROMPTS))
            names = list(PROMPTS.values())
            pose = YOLO(str(NEXT / "yolo11n-pose.pt"))
            sam = SAM(str(NEXT / "mobile_sam.pt"))
            clipm = Clip()
            # gran parte del tiempo por cuadro es CPU (JPEG, recortes, preprocesado): con 0.8 de ciclo
            # útil sobre el tiempo total la GPU queda muy por debajo del 60 %
            thr = Throttle(0.8)
            finished = False
            with OUT.open("a", encoding="utf-8") as fo:
                while turn.elapsed() < TURN_S:
                    if k >= len(todo):
                        # se acabó la lista: recoge lo que el muestreo haya extraído entretanto
                        todo, k = pending(), 0
                        if not todo:
                            if (TRAIN / "data" / "frames.json").exists():
                                finished = True
                                break
                            time.sleep(10)
                            continue
                    f = todo[k]
                    k += 1
                    done.add(f["id"])
                    big = cv2.imread(str(TRAIN / "data" / "img1920" / f"{f['id']}.jpg"))
                    if big is None:
                        continue
                    thr.start()
                    H, W = big.shape[:2]
                    s = 960 / max(H, W)
                    w9, h9 = int(round(W * s)), int(round(H * s))
                    im9 = cv2.resize(big, (w9, h9), interpolation=cv2.INTER_AREA)
                    cv2.imwrite(str(img960 / f"{f['id']}.jpg"), im9, [cv2.IMWRITE_JPEG_QUALITY, 95])
                    flat = bool(is_flat(im9))
                    det = normalize_contrast(im9) if flat else im9
                    detbig = normalize_contrast(big) if flat else big

                    r = world.predict(det, imgsz=960, conf=0.02, verbose=False, device=0)[0]
                    dets = []
                    for b, c, cl in zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist()):
                        dets.append({"box": [round(v, 1) for v in b], "conf": round(c, 4), "label": names[int(cl)], "zoom": False})
                    people = [d for d in dets if d["label"] not in JEWELS and d["conf"] >= 0.1]
                    jew = sorted([d for d in dets if d["label"] in JEWELS], key=lambda d: -d["conf"])[:15]

                    kps = []
                    if any(d["label"] == "persona" and d["conf"] >= 0.4 for d in dets):
                        pr = pose.predict(det, imgsz=640, conf=0.4, verbose=False, device=0)[0]
                        if pr.keypoints is not None and pr.keypoints.data is not None:
                            kps = [kk.cpu().numpy() for kk in pr.keypoints.data]
                    # segunda mirada a 1920 px en orejas, cuello y muñecas (donde se esconden topos y aretes)
                    kx = W / w9
                    for reg in zoom_regions(kps, w9, h9, 5):
                        x0, y0, x1, y1 = [int(v * kx) for v in reg]
                        crop = detbig[y0:y1, x0:x1]
                        if crop.size == 0:
                            continue
                        zr = world.predict(crop, imgsz=960, conf=0.05, verbose=False, device=0)[0]
                        for b, c, cl in zip(zr.boxes.xyxy.tolist(), zr.boxes.conf.tolist(), zr.boxes.cls.tolist()):
                            if names[int(cl)] in JEWELS:
                                bb = [round(reg[0] + b[0] / kx, 1), round(reg[1] + b[1] / kx, 1),
                                      round(reg[0] + b[2] / kx, 1), round(reg[1] + b[3] / kx, 1)]
                                jew.append({"box": bb, "conf": round(c, 4), "label": names[int(cl)], "zoom": True})

                    # CLIP sobre todos los candidatos de joya (recorte de la imagen de 1920 para más detalle)
                    crops = [pad_crop(detbig, [v * kx for v in d["box"]]) for d in jew]
                    keep = [i for i, c in enumerate(crops) if c.size and min(c.shape[:2]) >= 2]
                    for i, (p, ty) in zip(keep, clipm.run([crops[i] for i in keep])):
                        jew[i]["clip_p"] = round(p, 3)
                        jew[i]["types"] = ty

                    # silueta SAM de los candidatos que podrían ser joya: caja ajustada
                    cand = [i for i, d in enumerate(jew) if d["conf"] >= 0.1 or d.get("clip_p", 0) >= 0.5 or d["zoom"]][:10]
                    if cand:
                        sr = sam.predict(det, bboxes=[jew[i]["box"] for i in cand], verbose=False, device=0)[0]
                        if sr.masks is not None:
                            md = sr.masks.data.cpu().numpy().astype(bool)
                            for j, i in enumerate(cand):
                                if j >= len(md):
                                    break
                                m = md[j]
                                if m.shape != (h9, w9):
                                    m = cv2.resize(m.astype(np.uint8), (w9, h9), interpolation=cv2.INTER_NEAREST).astype(bool)
                                x0, y0, x1, y1 = [int(v) for v in jew[i]["box"]]
                                sub = m[max(0, y0):y1, max(0, x0):x1]
                                if sub.sum() >= 12:
                                    ys, xs = np.nonzero(sub)
                                    jew[i]["sam_box"] = [x0 + int(xs.min()), y0 + int(ys.min()), x0 + int(xs.max()) + 1, y0 + int(ys.max()) + 1]
                                    jew[i]["sam_frac"] = round(float(sub.mean()), 3)
                    rec = {"id": f["id"], "w": w9, "h": h9, "flat": flat, "jewels": jew, "people": people,
                           "kps": [np.round(kk, 1).tolist() for kk in kps]}
                    fo.write(json.dumps(rec) + "\n")
                    fo.flush()
                    torch.cuda.synchronize()
                    thr.pause()
                    if k % 100 == 0:
                        print(f"  {k}/{len(todo)}  {turn.elapsed() / 60:.1f} min", flush=True)
            del world, pose, sam, clipm
            torch.cuda.empty_cache()
        if finished:
            break
        time.sleep(60)  # deja entrar a otros antes de pedir otro turno
    print("maestro listo", flush=True)


if __name__ == "__main__":
    main()
