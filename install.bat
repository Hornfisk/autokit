@echo off
setlocal

set "SCRIPT_DIR=%~dp0"
set "PLUGIN=autokit.dll"
set "STANDALONE=autokit-standalone.exe"

if not exist "%SCRIPT_DIR%%PLUGIN%" (
    echo Error: %PLUGIN% not found in %SCRIPT_DIR%
    echo Make sure install.bat is in the same folder as the built binaries.
    exit /b 1
)

echo Installing Autokit plugins...

:: VST3
set "VST3_DIR=%CommonProgramFiles%\VST3\autokit.vst3\Contents\x86_64-win"
if not exist "%VST3_DIR%" mkdir "%VST3_DIR%"
copy /y "%SCRIPT_DIR%%PLUGIN%" "%VST3_DIR%\autokit.vst3" >nul
echo   VST3 -^> %VST3_DIR%\autokit.vst3

:: CLAP
set "CLAP_DIR=%CommonProgramFiles%\CLAP"
if not exist "%CLAP_DIR%" mkdir "%CLAP_DIR%"
copy /y "%SCRIPT_DIR%%PLUGIN%" "%CLAP_DIR%\autokit.clap" >nul
echo   CLAP -^> %CLAP_DIR%\autokit.clap

:: Standalone
if exist "%SCRIPT_DIR%%STANDALONE%" (
    set "BIN_DIR=%LocalAppData%\Autokit"
    if not exist "%BIN_DIR%" mkdir "%BIN_DIR%"
    copy /y "%SCRIPT_DIR%%STANDALONE%" "%BIN_DIR%\autokit.exe" >nul
    echo   Standalone -^> %BIN_DIR%\autokit.exe
)

echo.
echo Done! Rescan plugins in your DAW to find Autokit.
echo Note: Windows may require running this as Administrator for the VST3/CLAP paths.
pause
