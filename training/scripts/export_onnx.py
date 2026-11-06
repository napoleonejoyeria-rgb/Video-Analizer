"""Exporta el detector propio a ONNX sin el paquete `onnx` (no está en el venv y no se instala nada):
se usa directamente el exportador de PyTorch (TorchScript, opset 17) con la cabeza Detect en modo
exportación de Ultralytics, igual que hace `model.export(format="onnx")`.

- Entrada `images` f32 [lote, 3, alto, ancho] dinámica; salida `output0` [lote, 4 + clases, anclas] con
  cajas xywh en píxeles de la entrada y puntuaciones con sigmoide (el formato que ya lee `yolo.rs`).
- `--fp16`: pesos y cálculo en FP16 (exportado en CUDA), con entrada y salida en f32 (casts dentro del
  grafo), así Rust no necesita tipos f16.
- Escribe también `jewel_det.classes.json` con las clases y los umbrales de trabajo.

Uso: export_onnx.py --model runs/r1/weights/best.pt --out ../models/jewel_det.onnx [--fp16] [--thr 0.3]
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

import torch

from labels import CLASSES, JEWEL7


class F32Wrapper(torch.nn.Module):
    """Entrada y salida en f32 alrededor de un modelo en f16."""

    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, x):
        y = self.m(x.half())
        y = y[0] if isinstance(y, (list, tuple)) else y
        return y.float()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--fp16", action="store_true")
    ap.add_argument("--thr", type=float, default=0.3, help="umbral de trabajo por defecto de las joyas")
    ap.add_argument("--thr_json", default="", help="JSON {clase: umbral} opcional")
    ap.add_argument("--weak", type=float, default=None,
                    help="umbral de las débiles que van al verificador CLIP (weak_threshold; ver sim_weak.py)")
    a = ap.parse_args()

    from ultralytics import YOLO
    from ultralytics.nn.modules import Detect
    from torch.onnx._internal.torchscript_exporter import onnx_proto_utils

    # este paso solo inserta funciones onnxscript propias (no hay ninguna) y es lo único que pide el
    # paquete `onnx`: sin él se devuelve el modelo tal cual
    onnx_proto_utils._add_onnxscript_fn = lambda model_bytes, custom_opsets: model_bytes

    yolo = YOLO(a.model)
    names = [yolo.names[i] for i in range(len(yolo.names))]
    assert names == CLASSES, names
    model = copy.deepcopy(yolo.model).float().eval()
    model = model.fuse() if hasattr(model, "fuse") else model
    for m in model.modules():
        if isinstance(m, Detect):
            # como el Exporter de Ultralytics: salida concatenada (xywh + sigmoide) sin postproceso
            m.export, m.format, m.dynamic = True, "onnx", True
            if hasattr(m, "max_det"):
                m.max_det = 300
    for p in model.parameters():
        p.requires_grad = False
    dev = "cuda" if a.fp16 else "cpu"
    net = F32Wrapper(model.half().to(dev)) if a.fp16 else model
    x = torch.zeros(1, 3, 544, 960, device=dev)
    with torch.no_grad():
        y = net(x)
        y = y[0] if isinstance(y, (list, tuple)) else y
        print("salida de prueba", tuple(y.shape))
        torch.onnx.export(net, x, a.out, opset_version=17, input_names=["images"], output_names=["output0"],
                          dynamic_axes={"images": {0: "batch", 2: "height", 3: "width"},
                                        "output0": {0: "batch", 2: "anchors"}},
                          do_constant_folding=True, dynamo=False)
    thr = {c: a.thr for c in JEWEL7}
    if a.thr_json:
        thr.update(json.loads(Path(a.thr_json).read_text()))
    cls = {"names": CLASSES, "imgsz": 960, "thresholds": thr, "default_threshold": a.thr,
           "source": str(Path(a.model).resolve())}
    if a.weak is not None:
        cls["weak_threshold"] = a.weak
    Path(a.out).with_name("jewel_det.classes.json").write_text(json.dumps(cls, ensure_ascii=False, indent=1), encoding="utf-8")
    print("escrito", a.out, f"{Path(a.out).stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
