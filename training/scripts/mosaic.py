"""Mosaico rápido de cuadros (para echar un vistazo al material). Uso: mosaic.py salida.jpg img1 img2 ..."""
import sys

import cv2
import numpy as np

out, files = sys.argv[1], sys.argv[2:]
cell_w, cell_h, cols = 320, 180, 6
tiles = []
for f in files:
    im = cv2.imread(f)
    if im is None:
        continue
    h, w = im.shape[:2]
    s = min(cell_w / w, cell_h / h)
    im = cv2.resize(im, (int(w * s), int(h * s)), interpolation=cv2.INTER_AREA)
    t = np.full((cell_h, cell_w, 3), 40, np.uint8)
    oy, ox = (cell_h - im.shape[0]) // 2, (cell_w - im.shape[1]) // 2
    t[oy:oy + im.shape[0], ox:ox + im.shape[1]] = im
    cv2.putText(t, f.split("\\")[-1][:5], (3, 14), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 255, 255), 1)
    tiles.append(t)
while len(tiles) % cols:
    tiles.append(np.full((cell_h, cell_w, 3), 40, np.uint8))
rows = [np.hstack(tiles[i:i + cols]) for i in range(0, len(tiles), cols)]
cv2.imwrite(out, np.vstack(rows), [cv2.IMWRITE_JPEG_QUALITY, 85])
