"""Acceso a `metrics.py` de la app de referencia (solo se importa, nunca se edita)."""
import sys

sys.path.insert(0, r"C:\Users\Usuario\Tools\camcuts_next")
from camcuts.metrics import is_flat, normalize_contrast  # noqa: E402,F401
