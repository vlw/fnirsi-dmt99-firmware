#!/bin/sh
# Build the DMT-99 firmware on Linux.
#
#   ./build.sh
#
# Result: ./DMT99-custom.BIN, an update image for the meter's stock bootloader.
# Flash it with ./flash-dmt99.sh (or copy it to the meter's BOOT disk on Windows).
set -e
cd "$(dirname "$0")"
TARGET=riscv32imac-unknown-none-elf
OUT=DMT99-custom.BIN

fail() { echo "error: $1" >&2; exit 1; }

# --- prerequisites ---------------------------------------------------------
command -v cargo >/dev/null || fail "Rust is not installed. Get it from https://rustup.rs"
command -v python3 >/dev/null || fail "python3 is not installed (sudo apt install python3)"
if command -v rustup >/dev/null; then
    rustup target list --installed | grep -qx "$TARGET" ||
        fail "the RISC-V target is missing. Install it with: rustup target add $TARGET"
fi
OBJCOPY=
for c in llvm-objcopy llvm-objcopy-20 llvm-objcopy-19 llvm-objcopy-18 llvm-objcopy-17 llvm-objcopy-16 llvm-objcopy-15 rust-objcopy; do
    command -v "$c" >/dev/null && { OBJCOPY=$c; break; }
done
[ -n "$OBJCOPY" ] || fail "llvm-objcopy not found. Install LLVM: sudo apt install llvm"

# --- build -----------------------------------------------------------------
echo "[1/3] Compiling (cargo build --release --target $TARGET)"
(cd device && cargo build --release)

echo "[2/3] Converting ELF to $OUT ($OBJCOPY)"
"$OBJCOPY" -O binary "device/target/$TARGET/release/device" "$OUT"

echo "[3/3] Checking the image"
# The stock bootloader installs the image at 0x6400 and jumps there: the image must start with a
# jump, keep vector slot 0 at offset 0x38, and end before the settings page.
python3 -I - "$OUT" <<'EOF'
import struct, sys
b = open(sys.argv[1], "rb").read()
assert b[:2] == b"\x6f\x00" and struct.unpack_from("<I", b, 0x38)[0] == 0x6400, "bad image header"
assert len(b) <= 0x30000 - 0x6400, f"image too large: {len(b)} bytes"
EOF

echo
echo "Done: $OUT ($(stat -c %s "$OUT") bytes)"
echo "Flash it with: ./flash-dmt99.sh"
