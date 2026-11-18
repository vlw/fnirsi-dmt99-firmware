//! Platform-independent UI: drawing into horizontal bands of an RGB565 screen, and the app itself.
//! Shared by the device firmware and the PC simulator.
#![cfg_attr(not(test), no_std)]

pub mod app;
pub mod games;
pub mod meter;
pub mod font;
pub mod font_data;

pub use font_data::{BATT, BODY, BRAND, HUGE, ICON, ICON_FILL, LARGE, SMALL, TITLE};

pub const W: i32 = 320;
pub const H: i32 = 240;

/// RGB565 from 8-bit channels.
pub const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// Mix `fg` over `bg` with alpha 0..=255.
pub fn blend(bg: u16, fg: u16, a: u32) -> u16 {
    if a >= 255 {
        return fg;
    }
    if a == 0 {
        return bg;
    }
    let ch = |c: u16, sh: u32, m: u32| (c as u32 >> sh) & m;
    let mix = |b: u32, f: u32| (f * a + b * (255 - a) + 127) / 255;
    let r = mix(ch(bg, 11, 31), ch(fg, 11, 31));
    let g = mix(ch(bg, 5, 63), ch(fg, 5, 63));
    let b = mix(ch(bg, 0, 31), ch(fg, 0, 31));
    ((r << 11) | (g << 5) | b) as u16
}

/// A horizontal slice of the screen: rows `y0 .. y0 + rows`, full width.
/// The whole scene is drawn once per band; everything outside the band is clipped.
pub struct Canvas<'a> {
    pub buf: &'a mut [u16],
    pub y0: i32,
    pub rows: i32,
    /// Horizontal clip range `[from, to)`.
    pub clip: (i32, i32),
}

fn isqrt(n: u32) -> u32 {
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

impl Canvas<'_> {
    #[inline]
    fn put(&mut self, x: i32, y: i32, c: u16, a: u32) {
        if x >= self.clip.0 && x < self.clip.1 && y >= self.y0 && y < self.y0 + self.rows {
            let i = ((y - self.y0) * W + x) as usize;
            self.buf[i] = blend(self.buf[i], c, a);
        }
    }

    /// Darken the band towards black by `a` (0..=255), e.g. a scrim behind a dialog.
    pub fn dim(&mut self, a: u32) {
        for p in self.buf.iter_mut() {
            *p = blend(*p, 0, a);
        }
    }

    pub fn clear(&mut self, c: u16) {
        self.buf.fill(c);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u16) {
        let (x0, x1) = (x.max(self.clip.0), (x + w).min(self.clip.1));
        let (y0, y1) = (y.max(self.y0), (y + h).min(self.y0 + self.rows));
        for yy in y0..y1 {
            let row = ((yy - self.y0) * W) as usize;
            if x0 < x1 {
                self.buf[row + x0 as usize..row + x1 as usize].fill(c);
            }
        }
    }

    /// Anti-aliased rounded rectangle; `r` is clamped to half the shorter side.
    pub fn round_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: u16) {
        let r = r.min(w / 2).min(h / 2).max(0);
        let (ry0, ry1) = (y.max(self.y0), (y + h).min(self.y0 + self.rows));
        for yy in ry0..ry1 {
            let in_top = yy < y + r;
            let in_bot = yy >= y + h - r;
            if !in_top && !in_bot {
                self.fill_rect(x, yy, w, 1, c);
                continue;
            }
            // distance (in 1/16 px) from pixel centre to the corner circle centre, per column
            let cy = if in_top { y + r } else { y + h - r };
            let dy = (yy * 16 + 8) - cy * 16;
            for side in 0..2 {
                let cx = if side == 0 { x + r } else { x + w - r };
                let (xs, xe) = if side == 0 { (x, x + r) } else { (x + w - r, x + w) };
                for xx in xs..xe {
                    let dx = (xx * 16 + 8) - cx * 16;
                    let d = isqrt((dx * dx + dy * dy) as u32) as i32;
                    let cov = (r * 16 + 8 - d).clamp(0, 16) as u32;
                    if cov > 0 {
                        self.put(xx, yy, c, cov * 255 / 16);
                    }
                }
            }
            self.fill_rect(x + r, yy, w - 2 * r, 1, c);
        }
    }

    /// Draw `s` with its baseline at `y`; returns the advance width.
    pub fn text(&mut self, f: &font::Font, x: i32, y: i32, s: &str, c: u16) -> i32 {
        for (b, pen) in f.layout(s) {
            let (gx, gy) = (x + pen + b.x as i32, y + b.y as i32);
            if gy >= self.y0 + self.rows || gy + b.h as i32 <= self.y0 {
                continue;
            }
            for row in 0..b.h as i32 {
                let py = gy + row;
                if py < self.y0 || py >= self.y0 + self.rows {
                    continue;
                }
                for col in 0..b.w as i32 {
                    let a = f.alpha(b, col, row);
                    if a > 0 {
                        self.put(gx + col, py, c, a * 17);
                    }
                }
            }
        }
        f.width(s)
    }

    /// Text with its baseline at `y`, its ink centred horizontally on `cx`.
    pub fn text_centered(&mut self, f: &font::Font, cx: i32, y: i32, s: &str, c: u16) {
        let (l, r) = f.ink(s).map_or((0, f.width(s)), |(l, r, _, _)| (l, r));
        self.text(f, cx - (l + r) / 2, y, s, c);
    }

    /// Text with its baseline at `y`, its ink ending exactly at `rx`.
    pub fn text_right(&mut self, f: &font::Font, rx: i32, y: i32, s: &str, c: u16) {
        let r = f.ink(s).map_or(f.width(s), |(_, r, _, _)| r);
        self.text(f, rx - r, y, s, c);
    }

    /// Text (or an icon) whose ink is centred on (cx, cy) both ways.
    pub fn text_mid(&mut self, f: &font::Font, cx: i32, cy: i32, s: &str, c: u16) {
        if let Some((l, r, t, b)) = f.ink(s) {
            self.text(f, cx - (l + r) / 2, cy - (t + b) / 2, s, c);
        }
    }
}

/// Render a whole frame band by band; `flush(band, y0, rows)` sends each band to the screen.
pub fn render_frame(band: &mut [u16], draw: impl FnMut(&mut Canvas), flush: impl FnMut(&[u16], i32, i32)) {
    render_rows(band, (0, H), draw, flush)
}

/// Render only screen rows `from..to` (e.g. an animating strip).
pub fn render_rows(band: &mut [u16], (from, to): (i32, i32), mut draw: impl FnMut(&mut Canvas), mut flush: impl FnMut(&[u16], i32, i32)) {
    let rows = (band.len() as i32 / W).max(1);
    let mut y0 = from;
    while y0 < to {
        let n = rows.min(to - y0);
        let buf = &mut band[..(n * W) as usize];
        let mut c = Canvas { buf, y0, rows: n, clip: (0, W) };
        draw(&mut c);
        flush(c.buf, y0, n);
        y0 += n;
    }
}

/// Fixed-capacity string for formatting numbers without alloc.
pub struct Txt<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Txt<N> {
    pub fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Write for Txt<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        let n = b.len().min(N - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&b[..n]);
        self.len += n;
        Ok(())
    }
}
