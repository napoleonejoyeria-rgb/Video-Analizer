"""Ronda 3: puntúa todos los cuadros muestreados para buscar casos difíciles.

- CLIP ViT-B/32 sobre el cuadro entero: probabilidad de "macro" (una pieza llena el cuadro), de
  "cristal/recipiente" (bol, vitrina acrílica, cristalería) y de "recipiente con joyas largas".
- Detector propio actual (models/jewel_det, vía Ultralytics con los pesos .pt): sus detecciones a
  conf ≥ 0.15, para minar cadena/pulsera/dije y joyas grandes.
Salida: teacher/scores.jsonl (una línea por cuadro). Con turno de GPU.

Uso: score_frames.py --model runs/r2/weights/best.pt [--ids lista.txt]
"""
import argparse
import json
import sys
from pathlib import Path

import cv2
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from camcuts_metrics import is_flat, normalize_contrast  # noqa: E402
from gpu_turn import GpuTurn, Throttle  # noqa: E402
from labels import CLASSES, TRAIN  # noqa: E402

GROUPS = {
    "macro": ["an extreme close-up macro photo of a single piece of jewelry filling the whole frame",
              "a macro shot of a chain or bracelet so close that only part of it is visible",
              "a very close detail shot of gemstones and gold links"],
    "glass": ["a glass bowl with stones", "a transparent acrylic display stand", "crystal glassware and glass objects",
              "a glass jar or vase on a table"],
    "container": ["jewelry chains and bracelets lying in a bowl", "jewelry laid out on a tray",
                  "necklaces and bracelets inside an open jewelry box"],
    "other": ["a person wearing jewelry", "a woman talking to the camera", "a hand holding a ring",
              "a man talking in a store", "a wide shot of a jewelry store", "a closed jewelry box held in hands",
              "fabric and a table, nothing else"],
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--ids", default="")
    ap.add_argument("--out", default=str(TRAIN / "teacher" / "scores.jsonl"))
    a = ap.parse_args()
    import clip
    from PIL import Image
    from ultralytics import YOLO

    src = TRAIN / "data" / "img960"
    ids = Path(a.ids).read_text().split() if a.ids else sorted(p.stem for p in src.glob("*.jpg"))
    out = Path(a.out)
    done = set()
    if out.exists():
        done = {json.loads(l)["id"] for l in out.read_text().splitlines() if l.strip()}
    ids = [i for i in ids if i not in done]
    print(len(ids), "cuadros por puntuar", flush=True)
    torch.set_num_threads(4)
    with GpuTurn(max_wait_s=3600):
        model, pre = clip.load("ViT-B/32", device="cuda", download_root=str(Path.home() / ".cache" / "clip"))
        texts = [t for g in GROUPS.values() for t in g]
        owner = [g for g, ts in GROUPS.items() for _ in ts]
        with torch.no_grad():
            tf = model.encode_text(clip.tokenize(texts).cuda()).float()
            tf = tf / tf.norm(dim=-1, keepdim=True)
        det = YOLO(a.model)
        thr = Throttle(0.6)
        with out.open("a") as fo:
            for k, fid in enumerate(ids):
                thr.start()
                img = cv2.imread(str(src / f"{fid}.jpg"))
                flat = bool(is_flat(img))
                seen = normalize_contrast(img) if flat else img
                with torch.no_grad():
                    x = pre(Image.fromarray(cv2.cvtColor(seen, cv2.COLOR_BGR2RGB))).unsqueeze(0).cuda()
                    e = model.encode_image(x).float()
                    e = e / e.norm(dim=-1, keepdim=True)
                    p = (100 * e @ tf.T).softmax(-1)[0].tolist()
                sc = {g: 0.0 for g in GROUPS}
                for g, v in zip(owner, p):
                    sc[g] += v
                r = det.predict(seen, imgsz=960, conf=0.15, verbose=False, device=0)[0]
                dets = [{"box": [round(v, 1) for v in b], "conf": round(c, 3), "label": CLASSES[int(kk)]}
                        for b, c, kk in zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist())]
                fo.write(json.dumps({"id": fid, "w": img.shape[1], "h": img.shape[0], "flat": flat,
                                     **{g: round(v, 3) for g, v in sc.items()}, "dets": dets}) + "\n")
                thr.pause()
                if k % 500 == 0:
                    print(k, flush=True)


if __name__ == "__main__":
    main()
