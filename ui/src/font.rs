/// A character: code point and advance in 1/16 px. Its bitmaps are `phases` consecutive entries
/// of `Font::bitmaps`, rendered at sub-pixel shifts 0, 1/phases, ...
pub struct Glyph {
    pub cp: u32,
    pub adv: u16,
}

/// One rendering: ink box size, offset of its top-left from the pen on the baseline,
/// and the offset of its 4-bit alpha data.
pub struct Bitmap {
    pub w: u8,
    pub h: u8,
    pub x: i8,
    pub y: i8,
    pub off: u32,
}

pub struct Font {
    pub glyphs: &'static [Glyph],
    pub bitmaps: &'static [Bitmap],
    pub phases: u8,
    pub data: &'static [u8],
    /// kerning: (left glyph index << 8 | right glyph index, adjustment in 1/16 px), sorted
    pub kern: &'static [(u16, i8)],
    pub ascent: u8,
    pub line: u8,
}

/// Text layout with a 1/16 px pen and kerning; each glyph gets the bitmap of the sub-pixel
/// shift nearest to the pen, so spacing stays even like FreeType's own layout.
pub struct Layout<'a> {
    font: &'a Font,
    chars: core::str::Chars<'a>,
    pen: i32,
    prev: Option<usize>,
}

impl<'a> Iterator for Layout<'a> {
    /// (bitmap, pen x in whole px relative to the start)
    type Item = (&'a Bitmap, i32);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let ch = self.chars.next()?;
            let Some(i) = self.font.index(ch) else { continue };
            if let Some(p) = self.prev {
                self.pen += self.font.kerning(p, i);
            }
            let phases = self.font.phases.max(1) as i32;
            // nearest of `phases` shifts; rounding past the last one moves to the next whole pixel
            let steps = (self.pen * phases + 8) >> 4;
            let (x, phase) = (steps.div_euclid(phases), steps.rem_euclid(phases));
            self.pen += self.font.glyphs[i].adv as i32;
            self.prev = Some(i);
            return Some((&self.font.bitmaps[i * phases as usize + phase as usize], x));
        }
    }
}

impl Font {
    pub fn index(&self, ch: char) -> Option<usize> {
        let cp = ch as u32;
        self.glyphs.binary_search_by_key(&cp, |g| g.cp).ok()
    }

    pub fn glyph(&self, ch: char) -> Option<&Glyph> {
        self.index(ch).map(|i| &self.glyphs[i])
    }

    fn kerning(&self, left: usize, right: usize) -> i32 {
        let key = (left as u16) << 8 | right as u16;
        self.kern.binary_search_by_key(&key, |k| k.0).map_or(0, |i| self.kern[i].1 as i32)
    }

    /// Bitmaps of `s` with their pen x (whole px, relative to the start).
    pub fn layout<'a>(&'a self, s: &'a str) -> Layout<'a> {
        Layout { font: self, chars: s.chars(), pen: 0, prev: None }
    }

    /// Alpha 0..=15 of a bitmap pixel.
    #[inline]
    pub fn alpha(&self, b: &Bitmap, col: i32, row: i32) -> u32 {
        let i = b.off as usize * 2 + (row * b.w as i32 + col) as usize;
        let v = self.data[i / 2];
        (if i % 2 == 0 { v >> 4 } else { v & 15 }) as u32
    }

    /// Advance width of `s` in whole px.
    pub fn width(&self, s: &str) -> i32 {
        let mut l = self.layout(s);
        while l.next().is_some() {}
        (l.pen + 8) >> 4
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn kerning_and_fractional_advances() {
        let f = &crate::BODY;
        // Roboto kerns "Te": the pair is narrower than its glyphs' advances
        let sum = f.width("T") + f.width("e");
        assert!(f.width("Te") < sum, "{} vs {}", f.width("Te"), sum);
        // fractional advances do not accumulate rounding error over a word
        let w: i32 = "tttttttt".chars().map(|c| f.glyph(c).unwrap().adv as i32).sum();
        assert_eq!(f.width("tttttttt"), (w + 8) >> 4);
    }

    #[test]
    fn subpixel_phase_follows_the_pen() {
        let f = &crate::BODY;
        assert_eq!(f.phases, 2);
        // pen positions of a run of one glyph: whole px plus the half-pixel bitmap when needed
        let adv = f.glyph('i').unwrap().adv as i32;
        for (n, (b, x)) in f.layout("iiiiiiii").enumerate() {
            let pen = n as i32 * adv; // 1/16 px
            let steps = (pen * 2 + 8) >> 4; // in half pixels
            assert_eq!(x, steps / 2);
            let i = f.index('i').unwrap();
            assert!(core::ptr::eq(b, &f.bitmaps[i * 2 + (steps % 2) as usize]));
        }
    }
}
