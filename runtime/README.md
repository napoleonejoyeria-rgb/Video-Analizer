# runtime/: bibliotecas nativas de ONNX Runtime

Los `.dll` no van en Git (ver `.gitignore`). Esta carpeta queda en el repositorio solo para que exista después de
clonar.

| Archivo | Versión | Para qué |
|---|---|---|
| `onnxruntime.dll` | ONNX Runtime 1.24 (`1.24.20260316.2`), compilación con DirectML | `camcuts-infer` la carga en tiempo de ejecución (`ort`, carga dinámica) |
| `DirectML.dll` | DirectML 1.15.4 | El motor de GPU por DirectX 12 que usa ONNX Runtime |

**De dónde sacarlas:** del paquete instalado (`dist\CAMCUTS\runtime\`, el mismo par sirve para las dos ediciones)
o de la carpeta compartida de la empresa. Comprueba que son las correctas con:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1
```

El script compara su SHA-256 con `models\MANIFIESTO.sha256`. Sin estas dos bibliotecas, las pruebas de `camcuts-infer` y la app
fallan al abrir los modelos.
