#!/bin/sh
# Render every README screenshot from the simulator, in English and Russian.
#
#   tools/screenshots.sh
#
# English UI goes to docs/screenshots/ (README.md), Russian UI to docs/screenshots/ru/
# (README.ru.md). Each line below is a file name and the keys pressed in the simulator
# (see sim/src/main.rs: "en" switches the UI to English, "waitN" lets N×10 ms of game time pass).
# Adding a screenshot: add a line here and reference the file in both READMEs.
set -e
cd "$(dirname "$0")/.."
command -v python3 >/dev/null || { echo "error: python3 is not installed" >&2; exit 1; }
python3 -c "import PIL" 2>/dev/null || { echo "error: Pillow is missing (sudo apt install python3-pil)" >&2; exit 1; }

(cd sim && cargo build --release -q)
SIM=sim/target/release/sim
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p docs/screenshots/ru

# name                 keys (without the language)
SHOTS="
dcv                    right
menu                   back,down,down,down
ohm                    right,right
amp                    left,left,leftlong
ncv                    down,down,down,wait150
reset                  back,up,up,up,ok,right
about                  back,up,ok
rel                    right,wait100,uplong,wait100
game-snake             back,up,up,ok,ok,down,ok,wait60
game-runner            back,up,up,ok,down,ok,ok,wait100,ok,wait20
game-fifteen           back,up,up,ok,down,down,ok,left,up,right
"

shot() { # name keys out.png
    "$SIM" --shot "$TMP/$1.ppm" "$2" >/dev/null
    # 2× nearest-neighbour: crisp pixels on GitHub
    python3 -I -c "import sys; from PIL import Image; Image.open(sys.argv[1]).resize((640, 480), Image.NEAREST).save(sys.argv[2], optimize=True)" "$TMP/$1.ppm" "$3"
}

echo "$SHOTS" | while read -r name keys; do
    [ -n "$name" ] || continue
    shot "$name" "en,$keys" "docs/screenshots/$name.png"
    shot "$name" "$keys" "docs/screenshots/ru/$name.png"
    echo "  $name"
done
echo "Done: docs/screenshots/*.png (English) and docs/screenshots/ru/*.png (Russian)"
