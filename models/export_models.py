"""Exporta a ONNX los modelos de CAMCUTS para el port en Rust (camcuts-infer).

Uso (con el venv de la app, SIN instalar nada en él; onnx/onnxslim/onnxruntime van en una carpeta aparte):

    set PYTHONPATH=<carpeta con onnx, onnxslim, onnxconverter_common, onnxruntime-directml extraído>
    set YOLO_AUTOINSTALL=false
    python models/export_models.py --weights <carpeta con copias de los .pt> --out models

Los .pt se COPIAN antes a una carpeta temporal: el exportador de Ultralytics escribe el .onnx junto al .pt,
y la carpeta de referencia (camcuts_next) no se debe tocar.

Produce (FP32 y su versión FP16 con entradas/salidas en FP32):
  yolo_world_x.onnx / yolo_world_m.onnx   detector con los 10 prompts de jewel.py ya incrustados
  yolo11n_pose.onnx                       pose COCO de 17 puntos
  mobile_sam_encoder.onnx                 codificador de imagen (1,3,1024,1024) -> (1,256,64,64)
  mobile_sam_decoder.onnx                 prompt de caja + decodificador de máscara (N cajas)
  clip_visual.onnx                        codificador de imagen CLIP ViT-B/32 (B,3,224,224) -> (B,512)
  clip_text.json                          embeddings de texto ya normalizados (verificador y tipos)
  silero_vad.onnx                         copia del modelo ONNX que trae el paquete silero-vad
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from pathlib import Path

# El exportador de Ultralytics intenta instalar paquetes que falten: prohibido tocar el venv de la app.
os.environ.setdefault("YOLO_AUTOINSTALL", "false")

import torch  # noqa: E402

# Mismos prompts y en el mismo orden que camcuts_next/camcuts/jewel.py (el orden fija el índice de clase).
JEWEL_PROMPTS = {"ring": "anillo", "necklace": "collar", "chain necklace": "cadena", "earring": "arete",
                 "stud earring": "topo", "bracelet": "pulsera", "pendant": "dije", "jewelry": "joya"}
PEOPLE_PROMPTS = {"person": "persona", "face": "cara"}
PROMPTS = {**JEWEL_PROMPTS, **PEOPLE_PROMPTS}

# Textos del verificador CLIP y del tipificador (classify.py), copiados literalmente.
JEWEL_TEXT = ["a photo of a piece of jewelry", "a close-up of a gold earring", "a close-up of a ring",
              "a close-up of a bracelet", "a close-up of a necklace or chain", "a close-up of a pendant",
              "a close-up of a diamond or gemstone jewel"]
OTHER_TEXT = ["a photo of skin", "a photo of fabric", "an empty jewelry box", "a photo of a hand",
              "a blurry background", "a photo of a table surface", "a photo of hair", "a photo of a face"]
TYPE_TEXT = {
    "topo": ["a pair of small stud earrings", "small stud earrings in a jewelry box",
             "a tiny stud earring with a gemstone", "a stud earring on an earlobe"],
    "arete": ["a pair of dangling drop earrings", "elongated earrings with gemstones in a jewelry box",
              "an earring hanging from an ear"],
    "candonga": ["a pair of gold hoop earrings", "a hoop earring", "huggie hoop earrings"],
    "anillo": ["a ring", "a gold ring with a gemstone", "a ring worn on a finger", "a ring in a ring box",
               "a band ring held between fingers"],
    "dije": ["a pendant", "a medal pendant on a chain", "a charm pendant", "a cross pendant"],
    "cadena": ["a thin gold chain", "a chain necklace", "a gold chain draped on a surface"],
    "collar": ["a necklace worn on the neck", "a statement necklace with gemstones"],
    "pulsera": ["a bracelet", "a tennis bracelet with gemstones", "a bracelet worn on a wrist",
                "a rigid bangle bracelet"],
    "tobillera": ["an anklet worn on an ankle"],
    "rosario": ["a rosary with beads and a cross"],
    "piercing": ["a body piercing jewel in a nose or navel"],
}

OPSET = 17


def to_fp16(src: Path) -> Path:
    """Copia FP16 del modelo; entradas y salidas siguen en FP32 para que Rust no necesite f16."""
    import onnx
    from onnxruntime.transformers.float16 import convert_float_to_float16

    m = onnx.load(str(src))
    # Sin las formas intermedias guardadas: con ellas, los Cast a float32 de CLIP (LayerNorm en FP32)
    # quedaban con un tipo declarado que no coincidía y ONNX Runtime rechazaba el modelo.
    del m.graph.value_info[:]
    # el conversor de ONNX Runtime deja en FP32 las operaciones que no tienen versión FP16 (Resize, Range…)
    m16 = convert_float_to_float16(m, keep_io_types=True)
    dst = src.with_name(src.stem + "_fp16.onnx")
    onnx.save(m16, str(dst))
    return dst


def export_yolo_world(weights: Path, out: Path, name: str) -> None:
    """YOLO-World con set_classes(prompts) incrustado; entrada dinámica (lote, alto, ancho múltiplos de 32)."""
    from ultralytics import YOLOWorld

    model = YOLOWorld(str(weights))
    model.set_classes(list(PROMPTS))
    f = model.export(format="onnx", imgsz=960, dynamic=True, simplify=True, opset=OPSET, device="cpu")
    dst = out / f"{name}.onnx"
    shutil.move(f, dst)
    # nombres de clase en el orden del modelo, para comprobarlos desde Rust
    (out / f"{name}.classes.json").write_text(json.dumps([PROMPTS[k] for k in PROMPTS], ensure_ascii=False))
    print("ok", dst, "->", to_fp16(dst))


def export_pose(weights: Path, out: Path) -> None:
    from ultralytics import YOLO

    model = YOLO(str(weights))
    f = model.export(format="onnx", imgsz=640, dynamic=True, simplify=True, opset=OPSET, device="cpu")
    dst = out / "yolo11n_pose.onnx"
    shutil.move(f, dst)
    print("ok", dst, "->", to_fp16(dst))


class SamDecoder(torch.nn.Module):
    """Prompt de caja + decodificador de MobileSAM, sin el `bs` entero que congelaría N al trazar.

    Entradas: embeddings (1,256,64,64) del codificador y cajas (N,4) xyxy en el espacio 1024 del letterbox.
    Salidas: logits de máscara de baja resolución (N,1,256,256) y puntuación de calidad (N,1).
    """

    def __init__(self, sam):
        super().__init__()
        self.pe = sam.prompt_encoder
        self.dec = sam.mask_decoder

    def forward(self, image_embeddings: torch.Tensor, boxes: torch.Tensor):
        sparse = self.pe._embed_boxes(boxes)
        h, w = self.pe.image_embedding_size
        dense = self.pe.no_mask_embed.weight.reshape(1, -1, 1, 1).expand(boxes.shape[0], -1, h, w)
        masks, scores = self.dec(image_embeddings=image_embeddings, image_pe=self.pe.get_dense_pe(),
                                 sparse_prompt_embeddings=sparse, dense_prompt_embeddings=dense,
                                 multimask_output=False)
        return masks, scores


def export_sam(weights: Path, out: Path) -> None:
    import torch.nn.functional as F
    from ultralytics import SAM
    from ultralytics.nn.modules.transformer import LayerNorm2d

    # LayerNorm2d usa x.shape[1:2] como forma normalizada; al trazar con N dinámico deja de ser constante.
    # Misma operación con la forma tomada de los pesos (constante): resultado idéntico.
    def _ln_const(self, x):
        return F.layer_norm(x.permute(0, 2, 3, 1), (self.weight.shape[0],), self.weight, self.bias,
                            self.eps).permute(0, 3, 1, 2)

    LayerNorm2d.forward = _ln_const
    sam = SAM(str(weights)).model.float().eval()
    enc = out / "mobile_sam_encoder.onnx"
    with torch.no_grad():
        torch.onnx.export(sam.image_encoder, torch.randn(1, 3, 1024, 1024), str(enc), opset_version=OPSET,
                          input_names=["image"], output_names=["embeddings"], dynamo=False)
        dec = out / "mobile_sam_decoder.onnx"
        torch.onnx.export(SamDecoder(sam).eval(), (torch.randn(1, 256, 64, 64), torch.tensor([[100., 120., 300., 380.],
                                                                                                [10., 10., 50., 60.]])),
                          str(dec), opset_version=OPSET, input_names=["image_embeddings", "boxes"],
                          output_names=["masks", "scores"], dynamo=False,
                          dynamic_axes={"boxes": {0: "n"}, "masks": {0: "n"}, "scores": {0: "n"}})
    for f in (enc, dec):
        print("ok", f, "->", to_fp16(f))


def export_clip(out: Path) -> None:
    """Codificador visual de CLIP ViT-B/32 (pesos FP32 en CPU) y embeddings de texto normalizados."""
    import clip

    model, _ = clip.load("ViT-B/32", device="cpu", jit=False)
    model = model.float().eval()
    f = out / "clip_visual.onnx"
    with torch.no_grad():
        torch.onnx.export(model.visual, torch.randn(2, 3, 224, 224), str(f), opset_version=OPSET,
                          input_names=["pixels"], output_names=["embeds"], dynamo=False,
                          dynamic_axes={"pixels": {0: "batch"}, "embeds": {0: "batch"}})

        def enc(texts):
            t = model.encode_text(clip.tokenize(texts)).float()
            return t / t.norm(dim=-1, keepdim=True)

        verifier = enc(JEWEL_TEXT + OTHER_TEXT)
        typer = []
        for lab in TYPE_TEXT:   # media de las frases de cada tipo, renormalizada (JewelTyper)
            m = enc(TYPE_TEXT[lab]).mean(dim=0)
            typer.append((m / m.norm()).tolist())
    data = {
        "model": "ViT-B/32", "logit_scale": 100.0,
        "verifier": {"jewel_text": JEWEL_TEXT, "other_text": OTHER_TEXT, "n_jewel": len(JEWEL_TEXT),
                     "embeddings": verifier.tolist()},
        "typer": {"labels": list(TYPE_TEXT), "embeddings": typer},
    }
    (out / "clip_text.json").write_text(json.dumps(data), encoding="utf-8")
    print("ok", f, "->", to_fp16(f))


def copy_silero(out: Path) -> None:
    import silero_vad

    src = Path(silero_vad.__file__).parent / "data" / "silero_vad.onnx"
    shutil.copy(src, out / "silero_vad.onnx")
    print("ok", out / "silero_vad.onnx")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--weights", required=True, help="carpeta con COPIAS de los .pt")
    ap.add_argument("--out", required=True)
    ap.add_argument("--only", default="world_x,world_m,pose,sam,clip,vad")
    a = ap.parse_args()
    w, out = Path(a.weights), Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    only = set(a.only.split(","))
    if "world_x" in only:
        export_yolo_world(w / "yolov8x-worldv2.pt", out, "yolo_world_x")
    if "world_m" in only:
        export_yolo_world(w / "yolov8m-worldv2.pt", out, "yolo_world_m")
    if "pose" in only:
        export_pose(w / "yolo11n-pose.pt", out)
    if "sam" in only:
        export_sam(w / "mobile_sam.pt", out)
    if "clip" in only:
        export_clip(out)
    if "vad" in only:
        copy_silero(out)
    if "fp16" in only:   # solo regenerar las copias FP16 a partir de los .onnx FP32 ya exportados
        for stem in ("yolo_world_x", "yolo_world_m", "yolo11n_pose", "mobile_sam_encoder", "mobile_sam_decoder",
                     "clip_visual"):
            if (out / f"{stem}.onnx").exists():
                print("ok", to_fp16(out / f"{stem}.onnx"))


if __name__ == "__main__":
    sys.exit(main())
