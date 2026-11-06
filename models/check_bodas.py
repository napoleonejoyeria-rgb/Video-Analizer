"""Referencia de Python para comprobar los modelos de la edición Bodas (en CPU).

Uso: python models/check_bodas.py <carpeta con yolov8x-worldv2.pt> <clip_root> <imagen.png>...
Imprime, por imagen, las detecciones (≥ 0.05, mismos prompts que export_bodas.py) y los tres momentos más
probables (CLIP con el cuadro rellenado a cuadrado), para compararlos con `examples/bodas_check.rs`.
"""
import os
import sys
from pathlib import Path

os.environ.setdefault("YOLO_AUTOINSTALL", "false")
sys.path.insert(0, str(Path(__file__).resolve().parent))

import numpy as np  # noqa: E402
import torch  # noqa: E402
from export_bodas import MOMENT_TEXT, PROMPTS  # noqa: E402


def main() -> None:
    weights, clip_root, images = Path(sys.argv[1]), sys.argv[2], sys.argv[3:]
    from PIL import Image
    from ultralytics import YOLOWorld

    import clip

    det = YOLOWorld(str(weights / "yolov8x-worldv2.pt"))
    det.set_classes(list(PROMPTS))
    names = list(PROMPTS.values())
    model, prep = clip.load("ViT-B/32", device="cpu", jit=False, download_root=clip_root)
    model = model.float().eval()
    with torch.no_grad():
        text = []
        for phrases in MOMENT_TEXT.values():
            t = model.encode_text(clip.tokenize(phrases)).float()
            t = t / t.norm(dim=-1, keepdim=True)
            v = t.mean(dim=0)
            text.append(v / v.norm())
        text = torch.stack(text)
    for path in images:
        img = np.array(Image.open(path).convert("RGB"))
        bgr = img[:, :, ::-1].copy()
        print(f"== {path} ({img.shape[1]}x{img.shape[0]})")
        r = det.predict(bgr, imgsz=960, conf=0.05, device="cpu", verbose=False)[0]
        for (x0, y0, x1, y1), c, k in zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.boxes.cls.tolist()):
            print(f"  {names[int(k)]:<10} {c:.3f}  [{int(x0)}, {int(y0)}, {int(x1)}, {int(y1)}]")
        h, w = img.shape[:2]
        side = max(h, w)
        sq = np.zeros((side, side, 3), np.uint8)
        sq[(side - h) // 2:(side - h) // 2 + h, (side - w) // 2:(side - w) // 2 + w] = img
        with torch.no_grad():
            e = model.encode_image(prep(Image.fromarray(sq)).unsqueeze(0)).float()
            e = e / e.norm(dim=-1, keepdim=True)
            p = (100.0 * e @ text.T).softmax(dim=-1)[0].tolist()
        top = sorted(zip(MOMENT_TEXT, p), key=lambda x: -x[1])[:3]
        print("  momentos:", ", ".join(f"{k} {v:.3f}" for k, v in top))


if __name__ == "__main__":
    main()
