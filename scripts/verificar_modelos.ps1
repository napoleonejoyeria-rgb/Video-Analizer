<#
Comprueba que los modelos ONNX y las bibliotecas de runtime\ son exactamente los de la version del repositorio.

Los .onnx (unos 1.6 GB) y los .dll no van en Git (ver .gitignore). Se copian aparte, desde el paquete
instalado o la carpeta compartida de la empresa, y este script los compara con models\MANIFIESTO.sha256
(SHA-256 y tamano de cada archivo).

Uso:
  powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1            # comprobar
  powershell -ExecutionPolicy Bypass -File scripts\verificar_modelos.ps1 -Generar   # rehacer el manifiesto
                                                                                    # (solo al cambiar un modelo)
Sale con codigo 1 si falta un archivo o su hash no coincide.
#>
param([switch]$Generar)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$manifiesto = Join-Path $repo 'models\MANIFIESTO.sha256'

function Relativa([string]$ruta) { $ruta.Substring($repo.Length + 1) }

if ($Generar) {
    $archivos = @(Get-ChildItem (Join-Path $repo 'models') -Recurse -File -Filter *.onnx) +
                @(Get-ChildItem (Join-Path $repo 'runtime') -File -Filter *.dll)
    $lineas = foreach ($a in ($archivos | Sort-Object FullName)) {
        $h = (Get-FileHash $a.FullName -Algorithm SHA256).Hash.ToLower()
        '{0}  {1}  {2}' -f $h, $a.Length, (Relativa $a.FullName)
    }
    $cabecera = @(
        '# SHA-256, tamano en bytes y ruta (desde la raiz del repositorio) de cada modelo y biblioteca nativa.',
        '# Generado con scripts\verificar_modelos.ps1 -Generar. Comprobar con scripts\verificar_modelos.ps1.'
    )
    [IO.File]::WriteAllLines($manifiesto, [string[]]($cabecera + $lineas), (New-Object Text.UTF8Encoding($false)))
    Write-Output ("Manifiesto escrito: {0} archivos" -f $lineas.Count)
    exit 0
}

if (-not (Test-Path $manifiesto)) { Write-Output "Falta $manifiesto"; exit 1 }
$fallos = 0; $ok = 0
foreach ($l in Get-Content $manifiesto) {
    if ($l -match '^\s*(#|$)') { continue }
    $partes = $l -split '\s{2}', 3
    $hash = $partes[0]; $tam = [int64]$partes[1]; $rel = $partes[2]
    $ruta = Join-Path $repo $rel
    if (-not (Test-Path $ruta)) { Write-Output "FALTA      $rel"; $fallos++; continue }
    $largo = (Get-Item $ruta).Length
    if ($largo -ne $tam) { Write-Output "TAMANO MAL $rel ($largo en vez de $tam bytes)"; $fallos++; continue }
    $h = (Get-FileHash $ruta -Algorithm SHA256).Hash.ToLower()
    if ($h -ne $hash) { Write-Output "HASH MAL   $rel"; $fallos++; continue }
    $ok++
}
Write-Output ("{0} correctos, {1} con problemas" -f $ok, $fallos)
if ($fallos) { exit 1 }
