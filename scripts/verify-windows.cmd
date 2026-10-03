@echo off
setlocal
set "ai_usage_toolchain=%~1"
if not defined ai_usage_toolchain set "ai_usage_toolchain=stable-x86_64-pc-windows-msvc"
pushd "%~dp0.."
if errorlevel 1 exit /b 1
call cargo +%ai_usage_toolchain% fmt --check
if errorlevel 1 goto failed
call cargo +%ai_usage_toolchain% clippy --locked --all-targets -- -D warnings
if errorlevel 1 goto failed
call cargo +%ai_usage_toolchain% test --locked
if errorlevel 1 goto failed
call cargo +%ai_usage_toolchain% build --release --locked
if errorlevel 1 goto failed
target\release\aiUsage.exe --version
if errorlevel 1 goto failed
echo WINDOWS_VERIFICATION_PASSED
popd
exit /b 0
:failed
set "ai_usage_exit=%errorlevel%"
popd
exit /b %ai_usage_exit%
