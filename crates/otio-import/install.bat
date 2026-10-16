@echo off
REM Build caprust-otio-import and install it into CapRust's importer
REM directory so the File menu picks it up.
setlocal
set HERE=%~dp0
set ROOT=%HERE%..\..
set DEST=%APPDATA%\CapRust\importers
set NAME=caprust-otio-import

echo Building...
cargo build --release --manifest-path "%ROOT%\Cargo.toml" -p %NAME%
if errorlevel 1 exit /b 1

REM The target directory is wherever cargo puts it, which is not
REM necessarily next to the source tree -- it can be shared across
REM projects. Ask rather than guess a relative path.
set TARGET=
for /f "delims=" %%i in ('powershell -NoProfile -Command "(cargo metadata --manifest-path '%ROOT%\Cargo.toml' --format-version 1 --no-deps | ConvertFrom-Json).target_directory"') do set TARGET=%%i
if not defined TARGET (
  echo Could not find the cargo target directory.
  exit /b 1
)

set EXE=%TARGET%\release\%NAME%.exe
if not exist "%EXE%" (
  echo Expected %EXE% and it is not there.
  exit /b 1
)

if not exist "%DEST%" mkdir "%DEST%"

copy /Y "%EXE%" "%DEST%\%NAME%.exe" >nul
if errorlevel 1 exit /b 1

> "%DEST%\%NAME%.json" echo {
>> "%DEST%\%NAME%.json" echo   "id": "org.caprust.otio",
>> "%DEST%\%NAME%.json" echo   "name": "Import OTIO...",
>> "%DEST%\%NAME%.json" echo   "version": "0.1.0",
>> "%DEST%\%NAME%.json" echo   "protocol": 1,
>> "%DEST%\%NAME%.json" echo   "kinds": ["otio"],
>> "%DEST%\%NAME%.json" echo   "target": "file"
>> "%DEST%\%NAME%.json" echo }

echo.
echo Installed in %DEST%
dir "%DEST%"
endlocal
