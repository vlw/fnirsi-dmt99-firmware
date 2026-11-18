#!/bin/bash
# Flash firmware onto the FNIRSI DMT-99 from Linux.
#
#   ./flash-dmt99.sh                  flashes DMT99-custom.BIN (run ./build.sh first)
#   ./flash-dmt99.sh other.BIN        flashes another image, e.g. the stock FNIRSI firmware
#   ./flash-dmt99.sh --install-launcher
#                                     adds a "DMT-99 Flash" icon to the desktop and the app menu:
#                                     double-click it to flash, or drop a .BIN on it
#
# What it does: waits for the meter's bootloader disk, unmounts it, and asks for sudo to let
# tools/bootdisk.py write the file onto it (a backup of the disk goes to ./backups first).
# Then you press AUTO on the meter to install the firmware.
cd "$(dirname "$(readlink -f "$0")")" || exit 1

if [ "$1" = "--install-launcher" ]; then
    # Exec path quoted per the desktop-entry spec: \ " ` $ escaped, backslashes doubled in the file
    exec_path=$(printf '%s' "$PWD/flash-dmt99.sh" | sed 's/[\\"`$]/\\\\&/g')
    entry="[Desktop Entry]
Type=Application
Name=DMT-99 Flash
Comment=Flash firmware onto the FNIRSI DMT-99 (drop a .BIN here to choose another file)
Exec=\"$exec_path\" %f
Icon=media-flash
Terminal=true
Categories=Utility;"
    desktop_dir=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")
    mkdir -p "$HOME/.local/share/applications"
    for f in "$HOME/.local/share/applications/dmt99-flash.desktop" "$desktop_dir/DMT-99 Flash.desktop"; do
        [ -d "$(dirname "$f")" ] || continue
        printf '%s\n' "$entry" > "$f" && chmod +x "$f"
        gio set "$f" metadata::trusted true 2>/dev/null # GNOME: allow launching without a prompt
        echo "Installed: $f"
    done
    exit 0
fi

FW=$(readlink -f "${1:-DMT99-custom.BIN}")

finish() { echo; read -rp "Press Enter to close…" _; exit "$1"; }

[ -f "$FW" ] || { echo "Firmware file not found: $FW"; echo "Build it first with ./build.sh"; finish 1; }
command -v python3 >/dev/null || { echo "python3 is not installed (sudo apt install python3)"; finish 1; }
command -v sudo >/dev/null || { echo "sudo is required to write to the meter's disk"; finish 1; }
echo "Firmware: $FW ($(stat -c %s "$FW") bytes)"

# The bootloader disk: model "USB Special Disk", 3200 sectors of 512 bytes (1.6 MB).
find_dev() {
    for d in /sys/block/sd*; do
        [ "$(xargs < "$d/device/model" 2>/dev/null)" = "USB Special Disk" ] && [ "$(cat "$d/size")" = 3200 ] && { echo "${d##*/}"; return; }
    done
}

if [ -z "$(find_dev)" ]; then
    echo
    echo "The meter is not in update mode yet:"
    echo "  1. Switch the meter off."
    echo "  2. Hold the centre button (AUTO/MENU) and the power button."
    echo "  3. Connect the USB cable."
    echo -n "Waiting (up to 2 minutes)"
    for _ in $(seq 120); do
        [ -n "$(find_dev)" ] && break
        echo -n "."
        sleep 1
    done
    echo
    [ -n "$(find_dev)" ] || { echo "The meter did not appear. Try again."; finish 1; }
fi
DEV=$(find_dev)
echo "Found the bootloader disk: /dev/$DEV"
sleep 2 # let the desktop finish its automount, then take the disk back
for p in /dev/"$DEV"?*; do [ -b "$p" ] && udisksctl unmount -b "$p" >/dev/null 2>&1; done

echo "Writing needs root access to /dev/$DEV:"
if sudo python3 -I "$PWD/tools/bootdisk.py" flash "$FW" "$PWD/backups"; then
    echo
    echo "Done: the file is on the meter and verified."
    echo ">>> Now press AUTO on the meter. It installs the firmware in about 10 s and restarts."
    finish 0
else
    echo
    echo "Failed. Nothing was installed: the meter only flashes after you press AUTO."
    finish 1
fi
