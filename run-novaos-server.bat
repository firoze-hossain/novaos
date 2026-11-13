@echo off
setlocal
cd /d "%~dp0"

set "QEMU=%LOCALAPPDATA%\Programs\qemu\qemu-system-x86_64.exe"
if not exist "%QEMU%" (
    where qemu-system-x86_64 >nul 2>&1
    if %errorlevel% equ 0 (
        set "QEMU=qemu-system-x86_64"
    ) else (
        echo [ERROR] QEMU executable not found in PATH or %LOCALAPPDATA%\Programs\qemu.
        pause
        exit /b 1
    )
)

echo ========================================================================
echo  NovaOS Server Edition (Headless) - Starting QEMU Window on Windows
echo ========================================================================
echo Launching NovaOS Server in QEMU with:
echo  - 2 SMP CPU Cores (-smp 2)
echo  - 512 MB RAM (-m 512M)
echo  - Live Boot ISO: novaos.iso
echo  - Partitioned Server Hard Disk: disk-server.img
echo  - RTL8139 10/100 PCI Network + TFTP Server
echo ========================================================================

start "NovaOS Server Edition" "%QEMU%" -M pc,smm=off -m 512M -smp 2 -cdrom novaos.iso -boot order=d -drive file=disk-server.img,format=raw,if=ide,index=0,media=disk -netdev user,id=net0,tftp=tools/fixtures/tftproot -device rtl8139,netdev=net0,mac=52:54:00:12:34:56 -audiodev dsound,id=snd0 -device intel-hda -device hda-duplex,audiodev=snd0 -device AC97,audiodev=snd0 -vga std

echo NovaOS Server QEMU window launched successfully.
