@echo off
rem Sustituto de dlltool para la toolchain GNU sin MinGW completo.
rem
rem Los crates con enlace "raw-dylib" (windows-link, libloading 0.9, ort-sys...) hacen que rustc llame a
rem dlltool para crear bibliotecas de importacion. El dlltool que trae rustup necesita el ensamblador "as"
rem de MinGW, que no esta instalado. Este script recibe los mismos argumentos que rustc le pasa a dlltool
rem (-d archivo.def -D nombre.dll -l salida.lib ...) y crea la biblioteca con rust-lld (modo llvm-lib).
rem El trabajo lo hace dlltool-lld.ps1 (ver alli por que cada biblioteca cambia las mayusculas del DLL).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0dlltool-lld.ps1" %*
exit /b %errorlevel%
