"""Rasterize the UI fonts and Material Symbols icons into 4-bit alpha glyphs as Rust source.

usage: python3 tools/gen_font.py > ui/src/font_data.rs
Icons: Material Symbols Rounded (Apache-2.0), tools/fonts/MaterialSymbolsRounded.ttf
"""
import os

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROBOTO = "/usr/share/fonts/truetype/roboto/unhinted/RobotoTTF/Roboto-{}.ttf"
FALLBACK = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"  # for symbols Roboto lacks
SYMBOLS = os.path.join(HERE, "fonts", "MaterialSymbolsRounded.ttf")
# Roboto's Ω is narrow and tall; Noto Sans (same family of Google designs) has a wider one
NOTO = "/usr/share/fonts/truetype/noto/NotoSans-{}.ttf"
WIDE_GLYPHS = {"Ω"}

TEXT = "".join(map(chr, range(32, 127))) + "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя°Ωµ–—…⚡◀▶▲▼~›·"
BIG = " 0123456789.-OL"
# big status words: live wire ("Фаза!" / "Нет фазы") and the chip's AUTO screen
LARGE_TEXT = "".join(sorted(set("Фаза!Нет фазыLive!No live wireAUTO")))

# Material Symbols code points used by the UI (names from fonts.google.com/icons)
ICONS = {
    0xE7F6: "notifications_off",
    0xEA0B: "bolt",
    0xE034: "pause",
    0xE3AB: "brightness_6",
    0xE425: "timer",
    0xE7F4: "notifications",
    0xE8AC: "power_settings_new",
    0xE050: "volume_up",
    0xF076: "thermostat",
    0xF053: "restart_alt",
    0xE88E: "info",
    0xE39E: "exposure",  # REL
    0xE51E: "sensors",
    0xEFE6: "cable",
    0xE889: "history",
    0xE894: "language",
    0xEA28: "sports_esports",
    0xE9B0: "grid_view",
    0xE566: "directions_run",
    0xE260: "linear_scale",  # range bar
}
ICON_CHARS = "".join(map(chr, ICONS))

FONTS = [  # name, file, px, charset, variation axes (Fill, Grade, opsz, wght), subpixel phases
    ("SMALL", ROBOTO.format("Regular"), 14, TEXT, None, 2),
    ("BODY", ROBOTO.format("Medium"), 18, TEXT, None, 2),
    ("TITLE", ROBOTO.format("Medium"), 24, TEXT, None, 1),  # large: whole-px placement is not visible
    ("LARGE", ROBOTO.format("Medium"), 40, LARGE_TEXT, None, 1),  # only the letters of the big status words
    ("HUGE", ROBOTO.format("Light"), 88, BIG, None, 1),
    ("BATT", ROBOTO.format("Bold"), 12, " 0123456789", None, 1),
    ("ICON_FILL", SYMBOLS, 18, ICON_CHARS, [1, 0, 20, 500], 1),
    ("ICON", SYMBOLS, 22, ICON_CHARS, [0, 0, 24, 400], 1),
    ("BRAND", "/usr/share/fonts/truetype/font-awesome/fontawesome-webfont.ttf", 20, "\uf09b", None, 1),  # GitHub mark
]

# Extra letter spacing in px; a little keeps Roboto's "tt"/"ti" apart at these sizes.
TRACKING = {"SMALL": 0.25, "BODY": 0.3, "TITLE": 0.2}
SS = 4  # supersampling: glyphs are drawn 4x large and box-filtered, which also allows sub-pixel shifts
KERN_MIN = 0.25  # px


def load(path, px, axes):
    f = ImageFont.truetype(path, px)
    if axes:
        f.set_variation_by_axes(axes)
    return f


def raster(big, ch, shift):
    """Glyph `ch` of the SS-times-larger font, moved right by `shift` px and reduced to real size.
    Returns (alpha image cropped to ink, x, y of its top-left relative to pen/baseline) or None."""
    x0, y0, x1, y1 = big.getbbox(ch, anchor="ls")
    pad = 3 * SS
    ox = pad - x0 + (-(pad - x0)) % SS  # origin on a multiple of SS: one small pixel = SS big ones
    oy = pad - y0 + (-(pad - y0)) % SS
    w = ox + x1 + pad + SS
    h = oy + y1 + pad
    img = Image.new("L", (w + (-w) % SS, h + (-h) % SS), 0)
    ImageDraw.Draw(img).text((ox + round(shift * SS), oy), ch, font=big, fill=255, anchor="ls")
    small = img.reduce(SS)
    ink = small.getbbox()
    if not ink:
        return None
    return small.crop(ink), ink[0] - ox // SS, ink[1] - oy // SS


def glyphs(path, px, chars, axes, track, phases):
    """Supersampled glyphs in `phases` sub-pixel shifts, advances in 1/16 px and kerning pairs,
    so the firmware lays text out with a fractional pen like FreeType does."""
    main = load(path, px, axes)
    style = os.path.basename(path).split("-")[-1].split(".")[0]
    roboto = path.startswith(ROBOTO.split("{")[0])
    wide = ImageFont.truetype(NOTO.format(style), px) if roboto and os.path.exists(NOTO.format(style)) else None
    alt = ImageFont.truetype(FALLBACK, px)
    notdef = bytes(main.getmask("\uffff"))  # Roboto draws a box for missing glyphs instead of failing
    out, font_of = [], {}
    for ch in sorted(set(chars)):
        if ch != " " and bytes(main.getmask(ch)) == notdef:
            f, fpath, faxes = alt, FALLBACK, None
        elif wide and ch in WIDE_GLYPHS:
            f, fpath, faxes = wide, NOTO.format(style), None
        else:
            f, fpath, faxes = main, path, axes
        font_of[ch] = f
        big = load(fpath, px * SS, faxes)
        bitmaps = []
        for p in range(phases):
            r = raster(big, ch, p / phases)
            if r:
                img, gx, gy = r
                px4 = [(v * 15 + 127) // 255 for v in img.getdata()]
                packed = bytes((px4[i] << 4) | (px4[i + 1] if i + 1 < len(px4) else 0) for i in range(0, len(px4), 2))
                bitmaps.append((img.width, img.height, gx, gy, packed))
            else:
                bitmaps.append((0, 0, 0, 0, b""))
        out.append((ord(ch), round((f.getlength(ch) + track) * 16), bitmaps))
    kern = []
    cs = [chr(g[0]) for g in out]
    for a, ca in enumerate(cs):
        for b, cb in enumerate(cs):
            if font_of[ca] is main and font_of[cb] is main:
                k = main.getlength(ca + cb) - main.getlength(ca) - main.getlength(cb)
                if abs(k) >= KERN_MIN:
                    kern.append((a << 8 | b, max(-128, min(127, round(k * 16)))))
    asc, desc = main.getmetrics()
    return out, kern, asc, asc + desc


print("// Generated by tools/gen_font.py from Roboto, Noto Sans, DejaVu Sans, Font Awesome and Material Symbols. Do not edit.")
print("use crate::font::{Bitmap, Font, Glyph};")
for name, path, px, chars, axes, phases in FONTS:
    gs, kern, asc, line = glyphs(path, px, chars, axes, TRACKING.get(name, 0.0), phases)
    assert len(gs) <= 256, "kerning keys hold 8-bit glyph indices"
    data, table, bms = bytearray(), [], []
    for cp, adv, bitmaps in gs:
        table.append(f"Glyph{{cp:{cp},adv:{adv}}}")
        for w, h, x, y, packed in bitmaps:
            bms.append(f"Bitmap{{w:{w},h:{h},x:{x},y:{y},off:{len(data)}}}")
            data += packed
    print(f"static {name}_G: [Glyph; {len(table)}] = [{','.join(table)}];")
    print(f"static {name}_B: [Bitmap; {len(bms)}] = [{','.join(bms)}];")
    print(f"static {name}_D: [u8; {len(data)}] = {list(data)};")
    print(f"static {name}_K: [(u16, i8); {len(kern)}] = [{','.join(f'({k},{v})' for k, v in kern)}];")
    print(f"pub static {name}: Font = Font {{ glyphs: &{name}_G, bitmaps: &{name}_B, phases: {phases}, data: &{name}_D, kern: &{name}_K, ascent: {asc}, line: {line} }};")
print('pub const IC_GITHUB: &str = "\\u{f09b}";')
for cp, name in ICONS.items():
    print(f"pub const IC_{name.upper()}: &str = \"\\u{{{cp:x}}}\";")
