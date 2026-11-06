"""Exporta los modelos propios de la edición Bodas de CAMCUTS (camcuts-bodas.exe).

Uso (venv de la app, en CPU, sin instalar nada):

    set CUDA_VISIBLE_DEVICES=
    set YOLO_AUTOINSTALL=false
    python models/export_bodas.py --weights <carpeta con COPIAS de yolov8x/m-worldv2.pt> --out models/bodas

Produce:
  yolo_world_bodas_x.onnx / yolo_world_bodas_m.onnx  YOLO-World con los 12 sujetos de boda ya incrustados
  yolo_world_bodas_x.classes.json (…_m)              etiquetas en español en el orden de las clases
  clip_moments.json                                  embeddings de texto de los 16 momentos (CLIP ViT-B/32),
                                                     media de 3-6 frases por momento, renormalizada

A diferencia de export_models.py, no usa el exportador de Ultralytics (que exige el paquete `onnx`):
llama a torch.onnx.export (exportador TorchScript, sin dependencias extra) con la cabeza en modo export,
que da la misma salida `[lote, 4 + clases, anclas]` con las puntuaciones ya con sigmoide.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

os.environ.setdefault("YOLO_AUTOINSTALL", "false")

import torch  # noqa: E402

# Mismos prompts y orden que la edición Bodas de Python (camcuts_bodas/camcuts/jewel.py, SUBJECT_PROMPTS).
PROMPTS = {
    "face": "cara",
    "person": "persona",
    "bride in a white wedding dress": "novia",
    "groom in a suit": "novio",
    "wedding dress": "vestido",
    "wedding rings": "anillos",
    "bridal bouquet of flowers": "ramo",
    "wedding cake": "pastel",
    "champagne glass": "copas",
    "microphone": "micrófono",
    "high heel shoes": "zapatos",
    "sparkler": "bengalas",
}

# Momentos de la boda con varias frases cada uno (camcuts_bodas/camcuts/classify.py, MOMENT_TEXT).
MOMENT_TEXT = {
    "preparativos": ["a bride getting her makeup done before the wedding",
                     "a hairdresser styling the bride's hair",
                     "bridesmaids helping the bride into her wedding dress",
                     "a groom getting dressed and fixing his tie in a hotel room",
                     "the bride and her family getting ready in a bedroom before the wedding"],
    "detalles": ["a close-up of wedding rings on a table",
                 "a close-up of bridal shoes and accessories",
                 "wedding invitations and stationery arranged as a flat lay",
                 "a wedding dress hanging by a window",
                 "a close-up of a bridal bouquet and boutonniere",
                 "decorative wedding details: flowers, candles and table settings"],
    "lugar": ["an empty decorated wedding venue",
              "a wide establishing shot of the wedding venue exterior",
              "an empty reception hall with decorated tables before the guests arrive",
              "an empty outdoor ceremony site with rows of chairs and flowers",
              "a landscape view of the wedding location"],
    "ceremonia": ["a wedding ceremony with guests seated facing the altar",
                  "the bride walking down the aisle",
                  "an officiant leading a wedding ceremony",
                  "the bride and groom standing at the altar holding hands",
                  "guests watching a church wedding ceremony"],
    "votos": ["the bride reading her wedding vows to the groom",
              "the groom reading his vows from a piece of paper",
              "the couple facing each other and saying their vows at the altar",
              "an emotional groom speaking his vows with tears in his eyes"],
    "anillos": ["the groom putting a wedding ring on the bride's finger",
                "the couple exchanging wedding rings during the ceremony",
                "a close-up of hands exchanging rings at the altar"],
    "beso": ["the bride and groom kissing",
             "the first kiss at the wedding ceremony",
             "a newlywed couple kissing while guests applaud"],
    "cóctel": ["wedding guests mingling with drinks at a cocktail hour",
               "waiters serving drinks and appetizers to wedding guests",
               "guests chatting and laughing outdoors holding glasses"],
    "retratos": ["a romantic portrait of the bride and groom posing together",
                 "the newlyweds walking hand in hand at golden hour",
                 "a portrait of the bride posing alone",
                 "a group photo of the bridal party posing",
                 "the couple embracing outdoors for their wedding photos"],
    "entrada": ["the newlyweds making a grand entrance into the reception hall",
                "guests cheering as the bride and groom enter the party",
                "the bridal party entering the reception while guests applaud"],
    "primer baile": ["the bride and groom dancing their first dance alone on the dance floor",
                     "a couple slow dancing while wedding guests watch around them",
                     "the newlyweds' first dance under soft lights"],
    "discursos": ["a person giving a toast with a microphone at a wedding reception",
                  "the best man giving a speech to the wedding guests",
                  "the father of the bride giving a speech",
                  "wedding guests raising their glasses for a toast"],
    "banquete": ["wedding guests seated at round tables having dinner",
                 "waiters serving dinner at a wedding reception",
                 "a long banquet table with guests eating at a wedding"],
    "pastel": ["the bride and groom cutting the wedding cake",
               "a tiered white wedding cake on a table",
               "the couple feeding each other wedding cake"],
    "fiesta": ["guests dancing at a wedding party with colored lights",
               "a crowded dance floor at a wedding reception at night",
               "the bride tossing her bouquet to a group of women",
               "a DJ playing music at a wedding party",
               "people celebrating and jumping on the dance floor"],
    "salida": ["the newlyweds leaving through a tunnel of sparklers",
               "guests throwing petals at the couple as they exit",
               "the bride and groom driving away in a decorated car",
               "the couple running out of the ceremony while guests throw rice"],
}

OPSET = 17


def export_world(weights: Path, out: Path, name: str) -> None:
    """YOLO-World con los sujetos de boda incrustados; entrada dinámica (lote, alto, ancho múltiplos de 32)."""
    from ultralytics import YOLOWorld
    from ultralytics.nn.modules.head import Detect

    model = YOLOWorld(str(weights))
    model.set_classes(list(PROMPTS))
    m = model.model.float().eval().cpu()
    m = m.fuse(verbose=False) if hasattr(m, "fuse") else m
    for mod in m.modules():
        if isinstance(mod, Detect):
            # cabeza en modo export: devuelve solo [lote, 4 + clases, anclas] (xywh + sigmoide), como el
            # exportador de Ultralytics, y recalcula las anclas para cada tamaño de entrada
            mod.export, mod.format, mod.dynamic = True, "onnx", True
    dst = out / f"{name}.onnx"
    # El exportador TorchScript solo importa el paquete `onnx` para insertar funciones onnxscript propias,
    # que este modelo no tiene: se salta ese paso y se escribe el protobuf tal cual.
    from torch.onnx._internal.torchscript_exporter import onnx_proto_utils

    onnx_proto_utils._add_onnxscript_fn = lambda model_bytes, custom_opsets: model_bytes
    with torch.no_grad():
        dummy = torch.zeros(1, 3, 544, 960)
        y = m(dummy)
        y = y[0] if isinstance(y, (list, tuple)) else y
        print("salida de prueba", tuple(y.shape))
        torch.onnx.export(m, dummy, str(dst), opset_version=OPSET, input_names=["images"], output_names=["output0"],
                          dynamic_axes={"images": {0: "batch", 2: "height", 3: "width"},
                                        "output0": {0: "batch", 2: "anchors"}}, dynamo=False)
    (out / f"{name}.classes.json").write_text(json.dumps([PROMPTS[k] for k in PROMPTS], ensure_ascii=False),
                                              encoding="utf-8")
    print("ok", dst)


def export_moments(out: Path, clip_root: str | None) -> None:
    """Embeddings de texto de los momentos: media de las frases de cada uno, renormalizada (MomentClassifier)."""
    import clip

    model, _ = clip.load("ViT-B/32", device="cpu", jit=False, download_root=clip_root)
    model = model.float().eval()
    labels, embs = [], []
    with torch.no_grad():
        for m, phrases in MOMENT_TEXT.items():
            t = model.encode_text(clip.tokenize(phrases)).float()
            t = t / t.norm(dim=-1, keepdim=True)
            v = t.mean(dim=0)
            labels.append(m)
            embs.append((v / v.norm()).tolist())
    data = {"model": "ViT-B/32", "logit_scale": 100.0, "labels": labels, "embeddings": embs,
            "phrases": MOMENT_TEXT}
    (out / "clip_moments.json").write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
    print("ok", out / "clip_moments.json")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--weights", required=True, help="carpeta con COPIAS de los .pt de YOLO-World")
    ap.add_argument("--out", required=True)
    ap.add_argument("--clip-root", default=None, help="carpeta con ViT-B-32.pt (si no, la caché de clip)")
    ap.add_argument("--only", default="world_x,world_m,moments")
    a = ap.parse_args()
    w, out = Path(a.weights), Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    only = set(a.only.split(","))
    if "moments" in only:
        export_moments(out, a.clip_root)
    if "world_x" in only:
        export_world(w / "yolov8x-worldv2.pt", out, "yolo_world_bodas_x")
    if "world_m" in only:
        export_world(w / "yolov8m-worldv2.pt", out, "yolo_world_bodas_m")


if __name__ == "__main__":
    sys.exit(main())
