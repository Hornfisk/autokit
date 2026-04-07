@echo off
REM Launch Autokit standalone on Windows with a safe buffer size.
REM WASAPI in shared mode delivers variable buffer sizes that can exceed
REM the configured size, crashing nih-plug's CPAL backend assertion.
REM 2048 is large enough to accommodate WASAPI's actual delivery (typically
REM 480-1056 samples depending on the audio device and system settings).
set "DIR=%~dp0"
"%DIR%autokit-standalone.exe" --period-size 2048 %*
