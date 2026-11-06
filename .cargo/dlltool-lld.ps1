# Sustituto de dlltool para la toolchain GNU sin MinGW completo (lo llama dlltool-lld.cmd).
# Crea la biblioteca de importación de cada crate "raw-dylib" con rust-lld (modo llvm-lib).
# Corrección (agente de la ventana): si dos bibliotecas de importación nombran la misma DLL con el mismo
# texto (p. ej. dos versiones de windows-sys importando gdi32.dll), ld las junta en un solo descriptor y
# el terminador nulo de la primera deja huérfanas las funciones de la segunda: el programa salta a una
# dirección sin resolver al arrancar (0xC0000005 antes de main). Por eso cada biblioteca lleva su propia
# combinación de mayúsculas en el nombre de la DLL (Windows no distingue mayúsculas al cargarla).
$a = $args
$def=$null; $out=$null; $dll=$null
for ($i=0; $i -lt $a.Count; $i++) { switch -CaseSensitive ($a[$i]) { '-d' { $def=$a[++$i] } '-l' { $out=$a[++$i] } '-D' { $dll=$a[++$i] } } }
if (-not $def -or -not $out) { exit 2 }
$lineas = Get-Content -LiteralPath $def
$lineas = $lineas | Where-Object { $_ -notmatch '^\s*LIBRARY' }
if (-not $dll) { $dll = ((Get-Content -LiteralPath $def | Where-Object { $_ -match '^\s*LIBRARY' } | Select-Object -First 1) -replace '^\s*LIBRARY\s+','').Trim('" ') }
$base = [IO.Path]::GetFileNameWithoutExtension($dll).ToLower()
# cada biblioteca generada lleva su propia combinación de mayúsculas (el cargador de Windows no
# distingue mayúsculas): así ld crea un descriptor por biblioteca y ninguna lista queda cortada por
# el terminador nulo de otra biblioteca del mismo DLL
$letras = @(); for ($k = 0; $k -lt $base.Length; $k++) { if ([char]::IsLetter($base[$k])) { $letras += $k } }
$sha = [Security.Cryptography.SHA1]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($out + (Get-Content -LiteralPath $def -Raw)))
$h = [BitConverter]::ToUInt32($sha, 0)
$variantes = [Math]::Pow(2, [Math]::Min($letras.Count, 30)) - 1
$patron = [uint64](($h % [Math]::Max($variantes, 1)) + 1)
$cs = $base.ToCharArray()
for ($k = 0; $k -lt $letras.Count -and $k -lt 30; $k++) { if ($patron -band ([uint64]1 -shl $k)) { $cs[$letras[$k]] = [char]::ToUpper($cs[$letras[$k]]) } }
$base = -join $cs
$nuevo = @("LIBRARY $base.dll") + $lineas
Set-Content -LiteralPath "$def.lld.def" -Value $nuevo -Encoding ascii
& "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\rust-lld.exe" -flavor link /lib "/def:$def.lld.def" "/out:$out" /machine:x64
exit $LASTEXITCODE






