@echo off
REM Build caprust-otio-import and install it into CapRust's importer
REM directory so the File menu picks it up.
setlocal
set HERE=%~dp0
set ROOT=%HERE%..\..
set DEST=%APPDATA%\CapRust\importers

echo Building...
cargo build --release --manifest-path "%ROOT%\Cargo.toml" -p caprust-otio-import
if errorlevel 1 exit /b 1

if not exist "%DEST%" mkdir "%DEST%"
copy /Y "%ROOT%\target\release\caprust-otio-import.exe" "%DEST%\caprust-otio-import.exe" >nul
if errorlevel 1 exit /b 1

> "%DEST%\caprust-otio-import.json" echo {
>> "%DEST%\caprust-otio-import.json" echo   "id": "org.caprust.otio",
>> "%DEST%\caprust-otio-import.json" echo   "name": "OTIO (.otio)...",
>> "%DEST%\caprust-otio-import.json" echo   "version": "0.1.0",
>> "%DEST%\caprust-otio-import.json" echo   "protocol": 1,
>> "%DEST%\caprust-otio-import.json" echo   "kinds": ["otio"]
>> "%DEST%\caprust-otio-import.json" echo }

echo.
echo Installed in %DEST%
dir "%DEST%"
endlocal
