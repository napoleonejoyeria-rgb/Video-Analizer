<#
Arma los paquetes listos para usar de las dos ediciones desde target\release, models\ y runtime\:
  <Destino>\CAMCUTS\         camcuts.exe, camcuts-cli.exe, models\, runtime\          (joyas)
  <Destino>\CAMCUTS Bodas\   camcuts-bodas.exe, models\, runtime\, README.md, CLAUDE.md (bodas)

Antes: cargo build --release -p camcuts-gui -p camcuts-core

Uso:
  powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1                    # las dos, en dist\
  powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Edicion bodas
  powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Destino D:\otra   # otra carpeta
  powershell -ExecutionPolicy Bypass -File scripts\armar_dist.ps1 -Prueba            # solo dice que haria

Reglas:
  - Copia un archivo solo si cambio (tamano o SHA-256); los modelos iguales no se vuelven a copiar.
  - Si un .exe esta abierto, Windows no deja sobrescribirlo pero si renombrarlo: se renombra a
    <nombre>_anterior.exe y se copia el nuevo. La app abierta sigue funcionando con el viejo.
  - Despues de copiar comprueba el SHA-256 de cada archivo.
  - La edicion Bodas no puede llevar la palabra "Napoleone": si aparece en camcuts-bodas.exe, se detiene
    antes de copiar.
#>
param(
    [ValidateSet('joyas', 'bodas', 'ambas')][string]$Edicion = 'ambas',
    [string]$Destino,
    [switch]$Prueba
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Destino) { $Destino = Join-Path $repo 'dist' }
$release = Join-Path $repo 'target\release'

$paquetes = @(
    @{ Clave = 'joyas'; Carpeta = 'CAMCUTS'
       Exes = @('camcuts.exe', 'camcuts-cli.exe')
       Modelos = @('clip_text.json', 'clip_visual_fp16.onnx', 'jewel_det.classes.json', 'jewel_det_fp16.onnx',
                   'mobile_sam_decoder_fp16.onnx', 'mobile_sam_encoder_fp16.onnx', 'silero_vad.onnx',
                   'yolo11n_pose_fp16.onnx', 'yolo_world_m.classes.json', 'yolo_world_m.onnx',
                   'yolo_world_x.classes.json', 'yolo_world_x.onnx')
       Extras = @() },
    @{ Clave = 'bodas'; Carpeta = 'CAMCUTS Bodas'
       Exes = @('camcuts-bodas.exe')
       Modelos = @('bodas\clip_moments.json', 'clip_text.json', 'clip_visual_fp16.onnx',
                   'mobile_sam_decoder_fp16.onnx', 'mobile_sam_encoder_fp16.onnx', 'silero_vad.onnx',
                   'yolo11n_pose_fp16.onnx', 'bodas\yolo_world_bodas_m.classes.json', 'bodas\yolo_world_bodas_m.onnx',
                   'bodas\yolo_world_bodas_x.classes.json', 'bodas\yolo_world_bodas_x.onnx')
       Extras = @('empaque\bodas\README.md', 'empaque\bodas\CLAUDE.md') }
)

function Hash([string]$ruta) { (Get-FileHash $ruta -Algorithm SHA256).Hash }

function SinNapoleone([string]$exe) {
    $bytes = [IO.File]::ReadAllBytes($exe)
    $textos = @([Text.Encoding]::ASCII.GetString($bytes),
                [Text.Encoding]::Unicode.GetString($bytes),
                [Text.Encoding]::Unicode.GetString($bytes, 1, $bytes.Length - 1))
    foreach ($t in $textos) {
        if ($t.IndexOf('napoleone', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw "camcuts-bodas.exe contiene la palabra 'Napoleone': no se empaqueta."
        }
    }
}

function Copiar([string]$origen, [string]$destino) {
    if (-not (Test-Path $origen)) { throw "Falta $origen" }
    $nombre = Split-Path -Leaf $destino
    if ((Test-Path $destino) -and ((Get-Item $destino).Length -eq (Get-Item $origen).Length) -and
        ((Hash $destino) -eq (Hash $origen))) {
        Write-Output "  igual       $nombre"; return
    }
    if ($Prueba) { Write-Output "  copiaria    $nombre"; return }
    New-Item -ItemType Directory -Force (Split-Path -Parent $destino) | Out-Null
    try {
        Copy-Item $origen $destino -Force
    } catch {
        if ([IO.Path]::GetExtension($destino) -ne '.exe') { throw }
        # exe abierto: se renombra el viejo y se copia el nuevo al lado
        $anterior = [IO.Path]::ChangeExtension($destino, $null).TrimEnd('.') + '_anterior.exe'
        if (Test-Path $anterior) {
            try { Remove-Item $anterior -Force } catch { throw "$nombre esta abierto y tambien $(Split-Path -Leaf $anterior): cierra la app y vuelve a intentar." }
        }
        Rename-Item $destino (Split-Path -Leaf $anterior)
        Copy-Item $origen $destino -Force
        Write-Output "  (estaba abierto: el anterior quedo como $(Split-Path -Leaf $anterior))"
    }
    if ((Hash $destino) -ne (Hash $origen)) { throw "El hash de $destino no coincide despues de copiar" }
    Write-Output "  copiado     $nombre"
}

foreach ($p in $paquetes) {
    if ($Edicion -ne 'ambas' -and $Edicion -ne $p.Clave) { continue }
    $dir = Join-Path $Destino $p.Carpeta
    Write-Output "== $($p.Carpeta)  ->  $dir"
    if ($p.Clave -eq 'bodas') { SinNapoleone (Join-Path $release 'camcuts-bodas.exe') }
    foreach ($e in $p.Exes) { Copiar (Join-Path $release $e) (Join-Path $dir $e) }
    foreach ($dll in 'onnxruntime.dll', 'DirectML.dll') { Copiar (Join-Path $repo "runtime\$dll") (Join-Path $dir "runtime\$dll") }
    foreach ($m in $p.Modelos) { Copiar (Join-Path $repo "models\$m") (Join-Path $dir ("models\" + (Split-Path -Leaf $m))) }
    foreach ($x in $p.Extras) { Copiar (Join-Path $repo $x) (Join-Path $dir (Split-Path -Leaf $x)) }
}
Write-Output 'Listo.'
