"""Genera los números de referencia de Python para tests/python_numbers.rs (métricas y color, solo CPU).

Escribe en tests/data/numbers/: imágenes de entrada (.bin crudos BGR), salidas de imagen de Python
(.bin) y un numbers.json con los escalares. Uso:

    C:\\Users\\Usuario\\Tools\\camcuts\\.venv\\Scripts\\python.exe gen_python_numbers.py
"""
import json
import sys
from pathlib import Path

import cv2
import numpy as np

sys.path.insert(0, r"C:\Users\Usuario\Tools\camcuts_next")
from camcuts import color, metrics  # noqa: E402

OUT = Path(__file__).resolve().parent / "numbers"
OUT.mkdir(parents=True, exist_ok=True)
rng = np.random.RandomState(7)


def scene(h, w):
    """Escena sintética: degradados, ruido, rectángulos nítidos, una zona oscura y un objeto suave."""
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    img = np.stack([60 + 120 * x / w, 40 + 150 * y / h, 90 + 60 * np.sin(x / 17.0) * np.cos(y / 11.0)], -1)
    img += rng.normal(0, 6, img.shape)
    img[h // 5:h // 5 + 30, w // 6:w // 6 + 50] = (240, 235, 250)          # rectángulo nítido claro
    img[h // 2:h // 2 + 25, w // 2:w // 2 + 40] = (10, 20, 15)             # rectángulo oscuro
    img[:, -w // 8:] *= 0.15                                               # franja oscura (ruido en sombras)
    blob = cv2.GaussianBlur(np.zeros((h, w), np.float32), (0, 0), 1)
    cv2.circle(blob, (int(w * 0.75), int(h * 0.3)), h // 8, 1.0, -1)
    blob = cv2.GaussianBlur(blob, (0, 0), 4)
    img = img * (1 - blob[..., None]) + 200 * blob[..., None]
    return np.clip(img, 0, 255).astype(np.uint8)


def save(name, arr):
    np.ascontiguousarray(arr).tofile(OUT / f"{name}.bin")


nums = {}
H, W = 180, 320
a = scene(H, W)
b = np.roll(a, (-4, 6), axis=(0, 1)).copy()
b = np.clip(b.astype(np.int16) + rng.randint(-3, 4, b.shape), 0, 255).astype(np.uint8)
save("frame_a", a)
save("frame_b", b)
nums["size"] = [W, H]

ga = cv2.cvtColor(a, cv2.COLOR_BGR2GRAY)
gb = cv2.cvtColor(b, cv2.COLOR_BGR2GRAY)
save("gray_a", ga)
mask, lap = metrics.edges_and_laplacian(ga)
save("mask_a", mask.astype(np.uint8))
rect = (40, 30, 150, 120)          # x0, y0, x1, y1
region = np.zeros_like(mask)
region[rect[1]:rect[3], rect[0]:rect[2]] = True
nums["edge_density"] = metrics.density(mask)
nums["lap_var"] = float(lap.var())
nums["rect"] = list(rect)
nums["rect_density"] = metrics.density(mask, region)
nums["rect_lap_var"] = metrics.laplacian_variance(ga, region, lap=lap)
nums["rect_lap_p99"] = metrics.laplacian_peak(ga, region, lap=lap)
nums["rect_anisotropy"] = metrics.blur_anisotropy(ga, rect)
nums["exposure"] = metrics.exposure(ga)
nums["camera_shift"] = list(metrics.camera_shift(ga, gb))
save("thumb_a", metrics.thumb(ga).astype(np.uint8))
nums["is_flat_a"] = bool(metrics.is_flat(a))
flat = np.clip(a.astype(np.float32) * 0.35 + 85, 0, 255).astype(np.uint8)
save("flat", flat)
nums["is_flat_flat"] = bool(metrics.is_flat(flat))
save("normalized_flat", metrics.normalize_contrast(flat))
# imágenes de 960x540 para comprobar el desplazamiento y la miniatura al tamaño real de análisis
big = scene(540, 960)
big2 = np.roll(big, (3, -11), axis=(0, 1))
save("big_a", big)
save("big_b", big2)
gba, gbb = cv2.cvtColor(big, cv2.COLOR_BGR2GRAY), cv2.cvtColor(big2, cv2.COLOR_BGR2GRAY)
nums["big_camera_shift"] = list(metrics.camera_shift(gba, gbb))
save("big_thumb", metrics.thumb(gba).astype(np.uint8))
m2, l2 = metrics.edges_and_laplacian(gba)
nums["big_edge_density"] = metrics.density(m2)
nums["big_lap_var"] = float(l2.var())

# color: cámaras con la decodificación de ffmpeg de un Sony "pc" sin matriz (full, bt601 → bt709) y la de cv2
c = scene(90, 160)
save("color_in", c)
for tid in ["slog3_sgamut3cine", "slog3_sgamut3", "slog2_sgamut", "hlg", "vlog", "clog3", "applelog"]:
    save(f"color_{tid}_ffmpeg", color.make_converter(tid, decoded_range="full", decoded_matrix="bt601", source_matrix="bt709")(c))
    save(f"color_{tid}_cv2", color.make_converter(tid)(c))
    nums[f"gamut_{tid}"] = color.gamut_to_709(color._CAMERA[tid][2]).tolist()
save("out_lut", color._out_lut())

# LUT .cube: 3D 9³ con preparación 1D de 16, "full in"; y una 1D pura con salida legal
n = 9
cube = OUT / "test3d.cube"
lines = ["# LUT de prueba de CAMCUTS", "# full in", 'TITLE "prueba 3D"', "LUT_1D_SIZE 16", "LUT_3D_SIZE 9"]
for i in range(16):
    v = (i / 15) ** 0.8
    lines.append(f"{v:.6f} {min(1, v * 1.02):.6f} {v * 0.97:.6f}")
for bb in range(n):
    for gg in range(n):
        for rr in range(n):
            r, g, bl = rr / (n - 1), gg / (n - 1), bb / (n - 1)
            lines.append(f"{min(1, 0.9 * r + 0.1 * g):.6f} {g ** 1.1:.6f} {min(1, 0.2 * r + 0.8 * bl):.6f}")
cube.write_text("\n".join(lines) + "\n", encoding="utf-8")
save("cube3d_out", color.make_converter("cube", str(cube))(c))
save("cube3d_out_ffmpeg", color.make_converter("cube", str(cube), decoded_range="full", decoded_matrix="bt601", source_matrix="bt709")(c))
cube1 = OUT / "test1d.cube"
lines = ["LUT_1D_SIZE 32", "LUT_OUT_VIDEO_RANGE"]
for i in range(32):
    v = i / 31
    lines.append(f"{v ** 0.6:.6f} {v ** 0.9:.6f} {v ** 1.3:.6f}")
cube1.write_text("\n".join(lines) + "\n", encoding="utf-8")
save("cube1d_out", color.make_converter("cube", str(cube1))(c))

(OUT / "numbers.json").write_text(json.dumps(nums, indent=1), encoding="utf-8")
print("ok", len(nums), "valores")
