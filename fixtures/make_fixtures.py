"""Genera las salidas de referencia del pipeline PYTHON (camcuts_next) para los fotogramas de fixtures/.

Corre el código real de la app (solo lectura: se importa, no se modifica) en CUDA, como en producción,
y guarda en fixtures/python_ref.json:
  - detecciones crudas de YOLO-World (conf >= 0.02) para los pesos x y m
  - candidatos débiles con la probabilidad del verificador CLIP y las detecciones ya verificadas
  - pose (cajas, confianza y 17 puntos) de yolo11n-pose
  - siluetas de MobileSAM (PNG en fixtures/sil/) y su puntuación, por caja de grupo de joya
  - lap_p99 y demás medidas por joya, de jewel.measure()
  - probabilidades de voz de Silero VAD por ventana de 512 muestras del WAV de prueba
  - tiempos por modelo en PyTorch CUDA (ms), en tandas cortas

Uso:  python fixtures/make_fixtures.py --weights <carpeta con copias de los .pt>
(variables: PYTHONDONTWRITEBYTECODE=1 para no dejar .pyc en camcuts_next)
"""
from __future__ import annotations

import argparse
import copy
import gc
import json
import sys
import time
import wave
from pathlib import Path

import cv2
import numpy as np
import torch

REF = Path(r"C:\Users\Usuario\Tools\camcuts_next")
sys.path.insert(0, str(REF))

from camcuts import jewel  # noqa: E402
from camcuts.classify import Embedder, JewelVerifier  # noqa: E402

HERE = Path(__file__).resolve().parent
FRAMES = ["f1_cara", "f2_dije_pulsera", "f3_seis_joyas", "f4_blandas", "f5_persona", "f6_sin_joya"]
MIN_CONF = 0.15        # preset "joyas" (scoring.py)
VERIFY_MIN, VERIFY_ACCEPT = 0.02, 0.6   # __main__.py
REPS = 10              # repeticiones por medida de tiempo (tandas cortas: la GPU es compartida)


def ms(fn, reps=REPS) -> float:
    """Milisegundos medios de fn() tras un calentamiento, sincronizando CUDA."""
    fn()
    torch.cuda.synchronize()
    t = time.perf_counter()
    for _ in range(reps):
        fn()
    torch.cuda.synchronize()
    return (time.perf_counter() - t) * 1000 / reps


def free():
    gc.collect()
    torch.cuda.empty_cache()


def weak_candidates(frame, dets):
    """Igual que __main__.verify_weak pero devolviendo los recortes que mira CLIP."""
    strong = [d["box"] for d in dets if d["label"] in jewel.JEWEL_LABELS and d["conf"] >= MIN_CONF]
    weak = sorted((d for d in dets if d["label"] in jewel.JEWEL_LABELS and VERIFY_MIN <= d["conf"] < MIN_CONF
                   and not any(jewel._overlap(d["box"], b) > 0.5 for b in strong)), key=lambda d: -d["conf"])[:4]
    h, w = frame.shape[:2]
    out = []
    for d in weak:
        x0, y0, x1, y1 = d["box"]
        px, py = int((x1 - x0) * 0.15), int((y1 - y0) * 0.15)
        out.append((d, [max(0, x0 - px), max(0, y0 - py), min(w, x1 + px), min(h, y1 + py)]))
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--weights", required=True)
    a = ap.parse_args()
    W = Path(a.weights)
    dev = "cuda:0"
    frames = {n: cv2.imread(str(HERE / f"{n}.png")) for n in FRAMES}
    ref: dict = {"frames": {}, "speed_ms_torch_cuda": {}, "gpu": torch.cuda.get_device_name(0),
                 "min_conf": MIN_CONF, "verify_min": VERIFY_MIN, "verify_accept": VERIFY_ACCEPT}
    for n, f in frames.items():
        ref["frames"][n] = {"png": f"{n}.png", "width": f.shape[1], "height": f.shape[0]}
    speed = ref["speed_ms_torch_cuda"]
    f0 = frames["f2_dije_pulsera"]

    # ---- YOLO-World: x (perfil normal) y m (perfil con poca VRAM libre) ----
    for tag, wname in (("m", "yolov8m-worldv2.pt"), ("x", "yolov8x-worldv2.pt")):
        det = jewel.JewelDetector(str(W / wname), device=dev)
        for n, f in frames.items():
            ref["frames"][n][f"detections_{tag}"] = det.detect([f], conf=VERIFY_MIN)[0]
        speed[f"yolo_world_{tag}_960_b1"] = ms(lambda: det.detect([f0], conf=VERIFY_MIN))
        speed[f"yolo_world_{tag}_960_b6_per_frame"] = ms(lambda: det.detect(list(frames.values()), conf=VERIFY_MIN), 3) / 6
        del det
        free()

    # ---- CLIP: verificador sobre los candidatos débiles y sobre cada caja de joya (más puntos de prueba) ----
    emb = Embedder(dev)
    ver = JewelVerifier(emb)
    for n, f in frames.items():
        dets = ref["frames"][n]["detections_x"]
        clip_items = []
        for d, crop in weak_candidates(f, dets):
            clip_items.append({"kind": "weak", "box": d["box"], "crop": crop})
        h, w = f.shape[:2]
        for d in dets:   # también las joyas fuertes, con el mismo recorte (+15 %)
            if d["label"] in jewel.JEWEL_LABELS and d["conf"] >= MIN_CONF:
                x0, y0, x1, y1 = d["box"]
                px, py = int((x1 - x0) * 0.15), int((y1 - y0) * 0.15)
                clip_items.append({"kind": "strong", "box": d["box"],
                                   "crop": [max(0, x0 - px), max(0, y0 - py), min(w, x1 + px), min(h, y1 + py)]})
        crops = [f[c["crop"][1]:c["crop"][3], c["crop"][0]:c["crop"][2]] for c in clip_items]
        for c, p in zip(clip_items, ver.jewel_probability([c for c in crops if c.size])):
            c["p"] = p
        ref["frames"][n]["clip"] = clip_items
        # detecciones tras la segunda opinión de CLIP, como hace analyse()
        from camcuts.__main__ import verify_weak

        verified = copy.deepcopy(dets)
        verify_weak(f, verified, MIN_CONF, ver)
        ref["frames"][n]["detections_x_verified"] = verified
    crops4 = [f0[100:300, 100:300]] * 4
    speed["clip_visual_b4_per_crop"] = ms(lambda: ver.jewel_probability(crops4)) / 4
    del emb, ver
    free()

    # ---- Pose (en todos los fotogramas, para tener más puntos de prueba) ----
    pose = jewel.PoseFinder(str(W / "yolo11n-pose.pt"), device=dev)
    kps_by_frame = {}
    for n, f in frames.items():
        r = pose.model.predict([f], imgsz=640, conf=0.4, device=dev, verbose=False)[0]
        people = []
        if r.keypoints is not None and r.keypoints.data is not None:
            for b, c, k in zip(r.boxes.xyxy.tolist(), r.boxes.conf.tolist(), r.keypoints.data.cpu().numpy()):
                people.append({"bbox": b, "conf": c, "keypoints": k.tolist()})
        ref["frames"][n]["pose"] = people
        kps_by_frame[n] = [np.array(p["keypoints"], np.float32) for p in people]
    speed["yolo11n_pose_640_b1"] = ms(lambda: pose.keypoints([f0]))
    del pose
    free()

    # ---- MobileSAM: una silueta por caja de grupo de joya; y measure() completo para lap_p99 ----
    seg = jewel.JewelSegmenter(str(W / "mobile_sam.pt"), device=dev)
    (HERE / "sil").mkdir(exist_ok=True)
    for n, f in frames.items():
        h, w = f.shape[:2]
        dets = ref["frames"][n]["detections_x_verified"]
        sils = []
        for i, (main, _extras) in enumerate(jewel._groups(dets, MIN_CONF)):
            x0, y0, x1, y1 = main["box"]
            box = [max(0, x0), max(0, y0), min(w, x1), min(h, y1)]
            r = seg.model.predict(f, bboxes=[box], device=dev, verbose=False)[0]
            item = {"box": box, "score": None, "png": None}
            if r.masks is not None and len(r.masks.data):
                m = r.masks.data[0].cpu().numpy().astype(np.uint8) * 255
                item["png"] = f"sil/{n}_{i}.png"
                item["score"] = float(r.boxes.conf[0])
                cv2.imwrite(str(HERE / item["png"]), m)
            sils.append(item)
        ref["frames"][n]["silhouettes"] = sils
        s, _ = jewel.measure(0.0, f, copy.deepcopy(dets), None, None, 0.25, MIN_CONF, seg, None,
                             face_seen=jewel.has_face(kps_by_frame[n]), lap_scale=1.0, sil_cache=None)
        ref["frames"][n]["measure"] = {
            "jewel": s.jewel, "label": s.label, "n_jewels": s.n_jewels, "n_sharp": s.n_sharp,
            "person": s.person, "face": s.face,
            "jewels": [{k: j[k] for k in ("label", "conf", "box", "lap_p99", "lap_var", "masked", "mask_frac",
                                         "edge", "area_frac")} for j in s.jewels]}
    box0 = ref["frames"]["f2_dije_pulsera"]["silhouettes"][0]["box"]
    speed["mobile_sam_1box"] = ms(lambda: seg.masks(f0, [box0]))
    speed["mobile_sam_3box"] = ms(lambda: seg.masks(f0, [box0] * 3))
    del seg
    free()

    # ---- Silero VAD (modelo JIT en CPU, como audio.py): una probabilidad por ventana de 512 muestras ----
    from silero_vad import get_speech_timestamps, load_silero_vad

    with wave.open(str(HERE / "voz_c9174.wav")) as wf:
        pcm = np.frombuffer(wf.readframes(wf.getnframes()), np.int16).astype(np.float32) / 32768.0
    model = load_silero_vad()
    wav = torch.from_numpy(pcm)
    model.reset_states()
    probs = []
    for i in range(0, len(wav), 512):
        chunk = wav[i:i + 512]
        if len(chunk) < 512:
            chunk = torch.nn.functional.pad(chunk, (0, 512 - len(chunk)))
        probs.append(model(chunk, 16000).item())
    ts = get_speech_timestamps(wav, model, sampling_rate=16000, return_seconds=True, min_speech_duration_ms=400)
    ref["speech"] = {"wav": "voz_c9174.wav", "window": 512, "probs": probs,
                     "intervals": [[round(t["start"], 2), round(t["end"], 2)] for t in ts]}
    t0 = time.perf_counter()
    model.reset_states()
    for i in range(0, 512 * 200, 512):
        model(wav[i:i + 512], 16000)
    ref["speed_ms_torch_cpu"] = {"silero_vad_per_512": (time.perf_counter() - t0) * 1000 / 200}

    (HERE / "python_ref.json").write_text(json.dumps(ref, indent=1, ensure_ascii=False), encoding="utf-8")
    print(json.dumps({"speed": speed, "vad": ref["speed_ms_torch_cpu"]}, indent=1))


if __name__ == "__main__":
    main()
