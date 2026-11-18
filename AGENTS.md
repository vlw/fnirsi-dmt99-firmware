# Contributing guide for agents and people

Custom Rust firmware for the FNIRSI DMT-99 multimeter. Read README.md first for building,
flashing and the architecture. This file lists the conventions the project follows; keep
to them so changes stay consistent.

## What goes into the repository

- Only this project's own firmware and tools. Never commit the stock FNIRSI firmware or
  anything derived from it (patched images, dumps, disassembly), nor bootloader-disk images.
  .gitignore keeps `*.BIN`, `*.img`, `backups/` and `local/` out of git.
- The firmware runs on an unmodified meter. No changes that need opening the case,
  soldering or extra wiring.
- Games must be original: no clones of existing titles, their names, art or look.

## Languages

- The UI is bilingual. Every user-visible string exists in Russian and English:
  `tr(en, "русский", "English")`.
- Code, comments and commit messages are in English.
- There are two READMEs: README.md (English) and README.ru.md (Russian). They must say the
  same thing; change both together.

## UI rules

- New characters (letters, symbols, icons) must be added to the charsets in
  `tools/gen_font.py`, then `python3 tools/gen_font.py > ui/src/font_data.rs`. The glyph
  coverage test fails if a UI string uses a missing glyph.
- Material 3 look: rounded cards, Material Symbols icons, Roboto. Centre text and icons on
  their ink (`text_centered`, `text_mid`, `Font::ink`), not on advance widths.
- Layout is checked by tests (labels fit their controls in both languages, the reading is
  centred, the range bar fits). Add or update such a test when changing layout.
- Settings live in an 8-byte flash record (`Settings::to_bytes`, `FORMAT`). A new option must
  read the same as before from settings saved by older versions: for a new flag pick the bit
  meaning so that 0 is the old behaviour (see `range_bar`, stored inverted). Bump `FORMAT`
  only when the layout really changes, since that resets everyone's settings.
- Check every visual change in the simulator (`sim`, or `--shot` headless). Say what was not
  checked on a real meter.

## Before every commit

```bash
(cd ui && cargo test)
./build.sh
```

Both must pass. If the UI changed, also run `tools/screenshots.sh` (see below).

## Commit messages

- Subject: short and imperative, or "Version X.Y.Z: <main changes>" for a release commit.
- Body: what changes for the person using the meter, as a bullet list, with enough detail to
  follow the history without the diff (what was wrong before, what it does now, defaults,
  compatibility with saved settings). Technical notes go after that.

## Releasing version X.Y.Z

1. Bump the version everywhere it appears:
   - `version` in `ui/Cargo.toml`, `device/Cargo.toml`, `sim/Cargo.toml` and the three
     `Cargo.lock` files;
   - the version badge and the `vX.Y.Z` examples in both README.md and README.ru.md.
2. Update both READMEs for new features and settings (Highlights / Главное, the Settings /
   Настройки table, Controls if buttons changed).
3. Regenerate all screenshots in both languages: `tools/screenshots.sh`. They must show the
   version being released (the About and Reset screens print it). If a new screen is worth
   showing, add it to the script and to both READMEs.
4. Run the tests and `./build.sh`, then commit as "Version X.Y.Z: …" with a detailed body.
5. Tag with an annotated tag and push:
   ```bash
   git tag -a vX.Y.Z -m "Version X.Y.Z" && git push origin main && git push origin vX.Y.Z
   ```
6. Wait for the GitHub Actions run of the tag to finish (`gh run watch`). It builds the
   image, signs the provenance attestation and publishes the release.
7. Verify the published file: download it, check the `.sha256`, run
   `gh attestation verify DMT99-custom-vX.Y.Z.BIN -R <owner>/<repo>`, and compare its SHA-256
   with the local `./build.sh` output (builds are reproducible).
8. Then edit the release notes by hand (`gh release edit vX.Y.Z --notes-file …`). Keep the
   generated part (flashing, verification, full changelog link) and put above it:
   - `## What's new`: the changes in English;
   - `## Что нового`: the same list in Russian;
   - a `---` line, then the generated part.

   List only what a user of the meter will notice: new features and settings, changed
   screens, fixed bugs (say what was wrong). Leave out refactoring, tests, CI and other
   internals unless they matter to users (e.g. signed releases). Group with bold
   subheadings when there are many items. Do not change anything else in the notes.

## Useful facts

- The app runs at 0x6400 behind the stock bootloader; `build.sh` checks the image header.
- Flashing writes the bootloader's disk directly and needs root (`./flash-dmt99.sh` asks for
  sudo); after it, press AUTO on the meter.
- Measurement chip: USART1 9600 baud, commands `AA 55 05 <mode> … <mode+5>`, 12-byte
  `5A A5 …` display frames. The chip blanks its digits while it changes range; the UI keeps
  the last value then (except NCV, where blank means "no field").
- The range bar's full scale comes from the decimal point position (10^digits); the chip does
  not report its range otherwise.
- Screen tearing is a known limitation: the panel's TE (vsync) pin is not wired to the MCU.
