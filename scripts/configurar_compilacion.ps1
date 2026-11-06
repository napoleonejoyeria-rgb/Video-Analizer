<#
Ajusta .cargo\config.toml a la carpeta donde se clono el repositorio.

Por que: con la toolchain stable-x86_64-pc-windows-gnu sin MinGW completo, rustc necesita un sustituto de
dlltool (.cargo\dlltool-lld.cmd). cargo pasa la opcion "-C dlltool=..." tal cual a rustc, sin resolverla
contra la carpeta del repositorio, asi que la ruta tiene que ser absoluta. Este script la reescribe con la
ruta del clon actual. Solo hace falta una vez despues de clonar (o de mover la carpeta).

Con la toolchain MSVC no hace falta: esa seccion de config.toml solo aplica a x86_64-pc-windows-gnu.

Uso:  powershell -ExecutionPolicy Bypass -File scripts\configurar_compilacion.ps1
#>
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$cfg = Join-Path $repo '.cargo\config.toml'
$texto = [IO.File]::ReadAllText($cfg, [Text.Encoding]::UTF8)
$ruta = (Join-Path $repo '.cargo\dlltool-lld.cmd') -replace '\\', '/'
$nuevo = [regex]::Replace($texto, 'dlltool=[^"]+', [Text.RegularExpressions.MatchEvaluator] { param($m) "dlltool=$ruta" })
if ($nuevo -ceq $texto) {
    Write-Output "config.toml ya apunta a esta carpeta: $ruta"
} else {
    [IO.File]::WriteAllText($cfg, $nuevo, (New-Object Text.UTF8Encoding($false)))
    Write-Output "config.toml actualizado: dlltool=$ruta"
    Write-Output "Nota: cambiar las opciones de rustc obliga a cargo a recompilar todo la primera vez."
}
