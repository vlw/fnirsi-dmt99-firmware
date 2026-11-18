//! Three small games: Snake and Runner (with three difficulty levels) and the classic 15 puzzle.
//! Time is counted in 10 ms ticks (`t`), the same clock the firmware's main loop runs on.
use crate::app::{Key, Sound, BG, GREEN, ON_PRIMARY, ON_SURFACE, ON_SURFACE_VAR, ON_TERTIARY_CONT, PRIMARY, PRIMARY_CONT, SURFACE, SURFACE_HI, TERTIARY_CONT};
use crate::{Canvas, Txt, BODY, SMALL, TITLE, W};
use core::fmt::Write;

pub const COUNT: usize = 3;
pub const LEVELS: usize = 3;

pub fn name(i: usize, en: bool) -> &'static str {
    match (i, en) {
        (0, false) => "Змейка",
        (0, true) => "Snake",
        (1, false) => "Бегун",
        (1, true) => "Runner",
        (_, false) => "Пятнашки",
        (_, true) => "15 Puzzle",
    }
}

pub fn level_name(level: usize, en: bool) -> &'static str {
    match (level, en) {
        (0, false) => "Лёгкий",
        (0, true) => "Easy",
        (1, false) => "Средний",
        (1, true) => "Medium",
        (_, false) => "Сложный",
        (_, true) => "Hard",
    }
}

/// Snake and Runner have difficulty levels; the puzzle does not.
pub fn has_levels(i: usize) -> bool {
    i < 2
}

/// The puzzle keeps the fewest moves; the others the highest score.
pub fn lower_is_better(i: usize) -> bool {
    i == 2
}

/// xorshift32
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Rng(seed.wrapping_mul(2_654_435_761) | 1)
    }
    fn below(&mut self, n: u32) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x % n
    }
}

/// A short beep: pitch and the tick it ends at.
#[derive(Clone, Copy, Default)]
struct Beep {
    hz: u32,
    until: u32,
}

impl Beep {
    fn sound(&self, t: u32) -> Sound {
        if t < self.until {
            Sound::Tone { hz: self.hz }
        } else {
            Sound::Free
        }
    }
}

pub enum Game {
    Snake(Snake),
    Runner(Runner),
    Fifteen(Fifteen),
}

impl Game {
    pub fn new(i: usize, level: usize, t: u32) -> Game {
        match i {
            0 => Game::Snake(Snake::new(level, t)),
            1 => Game::Runner(Runner::new(level, t)),
            _ => Game::Fifteen(Fifteen::new(t)),
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Game::Snake(_) => 0,
            Game::Runner(_) => 1,
            Game::Fifteen(_) => 2,
        }
    }

    pub fn level(&self) -> usize {
        match self {
            Game::Snake(g) => g.level,
            Game::Runner(g) => g.level,
            Game::Fifteen(_) => 0,
        }
    }

    /// Returns true if the screen must be redrawn.
    pub fn key(&mut self, k: Key, t: u32) -> bool {
        match self {
            Game::Snake(g) => g.key(k, t),
            Game::Runner(g) => g.key(k, t),
            Game::Fifteen(g) => g.key(k, t),
        }
    }

    /// Advance to tick `t`; returns true if the screen must be redrawn.
    pub fn tick(&mut self, t: u32) -> bool {
        match self {
            Game::Snake(g) => g.tick(t),
            Game::Runner(g) => g.tick(t),
            Game::Fifteen(_) => false,
        }
    }

    /// A finished round's result (score, or moves for the puzzle), once per round.
    pub fn take_result(&mut self) -> Option<u32> {
        match self {
            Game::Snake(g) => g.result.take(),
            Game::Runner(g) => g.result.take(),
            Game::Fifteen(g) => g.result.take(),
        }
    }

    pub fn sound(&self, t: u32) -> Sound {
        match self {
            Game::Snake(g) => g.beep.sound(t),
            Game::Runner(g) => g.beep.sound(t),
            Game::Fifteen(g) => g.beep.sound(t),
        }
    }

    pub fn draw(&self, c: &mut Canvas, en: bool, best: Option<u32>) {
        c.clear(BG);
        let mut title = Txt::<40>::new();
        let _ = if has_levels(self.index()) {
            write!(title, "{} · {}", name(self.index(), en), level_name(self.level(), en))
        } else {
            write!(title, "{}", name(self.index(), en))
        };
        match self {
            Game::Snake(g) => g.draw(c, title.as_str(), en, best),
            Game::Runner(g) => g.draw(c, title.as_str(), en, best),
            Game::Fifteen(g) => g.draw(c, title.as_str(), en, best),
        }
    }
}

fn header(c: &mut Canvas, title: &str, score: u32, best: Option<u32>, en: bool) {
    c.text(&BODY, 16, 23, title, ON_SURFACE);
    let mut t = Txt::<24>::new();
    let _ = write!(t, "{score}");
    c.text_right(&BODY, W - 16, 23, t.as_str(), ON_SURFACE);
    if let Some(b) = best {
        let mut r = Txt::<24>::new();
        let _ = write!(r, "{} {b}", if en { "best" } else { "рекорд" });
        c.text_right(&SMALL, W - 16 - BODY.width(t.as_str()) - 12, 22, r.as_str(), ON_SURFACE_VAR);
    }
}

/// Dimmed overlay with a message and the restart hint.
fn game_over(c: &mut Canvas, title: &str, en: bool) {
    c.dim(140);
    c.round_rect(32, 76, W - 64, 108, 24, SURFACE_HI);
    c.text_centered(&TITLE, W / 2, 114, title, ON_SURFACE);
    c.text_centered(&SMALL, W / 2, 144, if en { "OK: play again" } else { "OK: заново" }, ON_SURFACE_VAR);
    c.text_centered(&SMALL, W / 2, 164, if en { "hold OK: exit" } else { "удерживать OK: выход" }, ON_SURFACE_VAR);
}

fn circle(c: &mut Canvas, cx: i32, cy: i32, r: i32, col: u16) {
    c.round_rect(cx - r, cy - r, 2 * r, 2 * r, r, col);
}

// ---------------------------------------------------------------- Snake

const CELL: i32 = 12;
const COLS: usize = 26;
const ROWS: usize = 16;
const FIELD_X: i32 = (W - COLS as i32 * CELL) / 2;
const FIELD_Y: i32 = 38;

/// The snake collects food; every piece makes it longer and faster.
pub struct Snake {
    level: usize,
    body: [(u8, u8); COLS * ROWS],
    head: usize,
    len: usize,
    dir: (i8, i8),
    next: (i8, i8),
    charge: (u8, u8),
    score: u32,
    last: u32,
    over: bool,
    rng: Rng,
    beep: Beep,
    result: Option<u32>,
}

impl Snake {
    fn new(level: usize, t: u32) -> Self {
        let mut g = Snake {
            level,
            body: [(0, 0); COLS * ROWS],
            head: 2,
            len: 3,
            dir: (1, 0),
            next: (1, 0),
            charge: (0, 0),
            score: 0,
            last: t,
            over: false,
            rng: Rng::new(t),
            beep: Beep::default(),
            result: None,
        };
        for i in 0..3 {
            g.body[i] = (4 + i as u8, ROWS as u8 / 2);
        }
        g.place_charge();
        g
    }

    fn segment(&self, k: usize) -> (u8, u8) {
        self.body[(self.head + self.body.len() - k) % self.body.len()]
    }

    fn occupied(&self, p: (u8, u8)) -> bool {
        (0..self.len).any(|k| self.segment(k) == p)
    }

    fn place_charge(&mut self) {
        loop {
            let p = (self.rng.below(COLS as u32) as u8, self.rng.below(ROWS as u32) as u8);
            if !self.occupied(p) {
                self.charge = p;
                return;
            }
        }
    }

    /// Ticks per move: starts slow and speeds up with the score, down to the level's limit.
    fn interval(&self) -> u32 {
        // (start, fastest) in 10 ms: easy 260 → 140 ms, medium 170 → 80 ms, hard 120 → 50 ms
        let (start, fastest) = [(26, 14), (17, 8), (12, 5)][self.level];
        (start as u32).saturating_sub(self.score / 3).max(fastest)
    }

    fn step(&mut self, t: u32) {
        self.dir = self.next;
        let (hx, hy) = self.segment(0);
        let (nx, ny) = (hx as i32 + self.dir.0 as i32, hy as i32 + self.dir.1 as i32);
        let eat = (nx, ny) == (self.charge.0 as i32, self.charge.1 as i32);
        let tail = self.segment(self.len - 1);
        let hits_self = (0..self.len).any(|k| {
            let s = self.segment(k);
            (s.0 as i32, s.1 as i32) == (nx, ny) && (eat || s != tail)
        });
        if nx < 0 || ny < 0 || nx >= COLS as i32 || ny >= ROWS as i32 || hits_self {
            self.over = true;
            self.result = Some(self.score);
            self.beep = Beep { hz: 400, until: t + 30 };
            return;
        }
        self.head = (self.head + 1) % self.body.len();
        self.body[self.head] = (nx as u8, ny as u8);
        if eat {
            self.len += 1;
            self.score += 1;
            self.beep = Beep { hz: 3000, until: t + 4 };
            if self.len < self.body.len() {
                self.place_charge();
            }
        }
    }

    fn key(&mut self, k: Key, t: u32) -> bool {
        if self.over {
            if k == Key::Ok {
                *self = Snake::new(self.level, t);
                return true;
            }
            return false;
        }
        let d = match k {
            Key::Up => (0, -1),
            Key::Down => (0, 1),
            Key::Left => (-1, 0),
            Key::Right => (1, 0),
            _ => return false,
        };
        if d != (-self.dir.0, -self.dir.1) {
            self.next = d; // no turning back into itself
        }
        false
    }

    fn tick(&mut self, t: u32) -> bool {
        let mut dirty = false;
        while !self.over && t.wrapping_sub(self.last) >= self.interval() {
            self.last = self.last.wrapping_add(self.interval());
            self.step(t);
            dirty = true;
        }
        dirty
    }

    fn cell(p: (u8, u8)) -> (i32, i32) {
        (FIELD_X + p.0 as i32 * CELL + CELL / 2, FIELD_Y + p.1 as i32 * CELL + CELL / 2)
    }

    fn draw(&self, c: &mut Canvas, title: &str, en: bool, best: Option<u32>) {
        header(c, title, self.score, best, en);
        c.round_rect(FIELD_X - 4, FIELD_Y - 4, COLS as i32 * CELL + 8, ROWS as i32 * CELL + 8, 14, SURFACE);
        // charge
        let (cx, cy) = Self::cell(self.charge);
        circle(c, cx, cy, 6, GREEN);
        circle(c, cx, cy, 2, BG);
        // cable: links between segments, then the beads, head last
        for k in 1..self.len {
            let (ax, ay) = Self::cell(self.segment(k - 1));
            let (bx, by) = Self::cell(self.segment(k));
            let (x0, y0) = (ax.min(bx) - 3, ay.min(by) - 3);
            c.fill_rect(x0, y0, (ax - bx).abs() + 6, (ay - by).abs() + 6, PRIMARY);
        }
        for k in (0..self.len).rev() {
            let (x, y) = Self::cell(self.segment(k));
            if k == 0 {
                circle(c, x, y, 6, ON_SURFACE); // the plug, with its pin facing forward
                circle(c, x + 2 * self.dir.0 as i32, y + 2 * self.dir.1 as i32, 2, ON_PRIMARY);
            } else {
                circle(c, x, y, 5, PRIMARY);
            }
        }
        if self.over {
            game_over(c, if en { "Game over!" } else { "Игра окончена!" }, en);
        }
    }
}

// ---------------------------------------------------------------- Runner

const GROUND: i32 = 204;
const SPARK_X: i32 = 64;
const SPARK_R: i32 = 9;
const PULSES: usize = 4;

#[derive(Clone, Copy, Default)]
struct Pulse {
    /// left edge in 1/16 px
    x: i32,
    w: i32,
    h: i32,
    live: bool,
}

/// A spark runs along an oscilloscope trace and jumps over voltage pulses; it keeps speeding up.
pub struct Runner {
    level: usize,
    /// height above the ground in 1/64 px, and its velocity
    y: i32,
    vy: i32,
    pulses: [Pulse; PULSES],
    /// speed in 1/16 px per tick
    speed: i32,
    distance: i32,
    gap: i32,
    last: u32,
    over: bool,
    rng: Rng,
    beep: Beep,
    result: Option<u32>,
}

const GRAVITY: i32 = 10; // 1/64 px per tick²
const JUMP: i32 = 300; // 1/64 px per tick: about 70 px high, 0.6 s in the air

/// (start, top) speed per level in 1/16 px per tick
const RUNNER_SPEED: [(i32, i32); LEVELS] = [(24, 56), (30, 72), (38, 92)];

impl Runner {
    fn new(level: usize, t: u32) -> Self {
        Runner {
            level,
            y: 0,
            vy: 0,
            pulses: [Pulse::default(); PULSES],
            speed: RUNNER_SPEED[level].0,
            distance: 0,
            gap: 120 * 16,
            last: t,
            over: false,
            rng: Rng::new(t ^ 0x5A5A),
            beep: Beep::default(),
            result: None,
        }
    }

    fn score(&self) -> u32 {
        (self.distance / 16 / 10) as u32
    }

    fn key(&mut self, k: Key, t: u32) -> bool {
        if self.over {
            if k == Key::Ok {
                *self = Runner::new(self.level, t);
                return true;
            }
            return false;
        }
        if matches!(k, Key::Ok | Key::Up) && self.y == 0 {
            self.vy = JUMP;
            self.beep = Beep { hz: 2000, until: t + 3 };
        }
        false
    }

    fn step(&mut self, t: u32) {
        // jump physics
        if self.y > 0 || self.vy > 0 {
            self.y += self.vy;
            self.vy -= GRAVITY;
            if self.y <= 0 {
                self.y = 0;
                self.vy = 0;
            }
        }
        // pulses move left; a new one spawns once the gap behind the last one is open
        let mut rightmost = i32::MIN;
        for p in self.pulses.iter_mut().filter(|p| p.live) {
            p.x -= self.speed;
            if p.x + p.w * 16 < 0 {
                p.live = false;
            } else {
                rightmost = rightmost.max(p.x + p.w * 16);
            }
        }
        if rightmost < (W * 16) - self.gap {
            if let Some(p) = self.pulses.iter_mut().find(|p| !p.live) {
                *p = Pulse { x: W * 16, w: 10 + self.rng.below(10) as i32, h: 18 + self.rng.below(34) as i32, live: true };
                // gaps shrink as the speed grows, but always leave room to land and jump again
                self.gap = (110 + self.rng.below(150) as i32) * 16 + self.speed * 30;
            }
        }
        self.distance += self.speed;
        if self.speed < RUNNER_SPEED[self.level].1 && self.distance % (16 * 400) < self.speed {
            self.speed += 2;
        }
        // collision with the spark's box (a little forgiving)
        let sy = GROUND - SPARK_R - self.y / 64;
        for p in self.pulses.iter().filter(|p| p.live) {
            let (px0, px1) = (p.x / 16, p.x / 16 + p.w);
            let hit_x = SPARK_X + SPARK_R - 3 > px0 && SPARK_X - SPARK_R + 3 < px1;
            let hit_y = sy + SPARK_R - 3 > GROUND - p.h;
            if hit_x && hit_y {
                self.over = true;
                self.result = Some(self.score());
                self.beep = Beep { hz: 400, until: t + 30 };
                return;
            }
        }
    }

    fn tick(&mut self, t: u32) -> bool {
        let mut steps = 0;
        while !self.over && t.wrapping_sub(self.last) > 0 && steps < 8 {
            self.last = self.last.wrapping_add(1);
            self.step(t);
            steps += 1;
        }
        self.last = t; // after a long redraw, skip ahead instead of running a burst of frames
        steps > 0
    }

    fn draw(&self, c: &mut Canvas, title: &str, en: bool, best: Option<u32>) {
        header(c, title, self.score(), best, en);
        // oscilloscope graticule scrolling with the trace
        let shift = (self.distance / 16) % 32;
        let mut x = -shift;
        while x < W {
            c.fill_rect(x, 40, 1, GROUND - 40, SURFACE);
            x += 32;
        }
        let mut y = GROUND - 32;
        while y > 40 {
            c.fill_rect(0, y, W, 1, SURFACE);
            y -= 32;
        }
        c.fill_rect(0, GROUND, W, 2, SURFACE_HI);
        // voltage pulses
        for p in self.pulses.iter().filter(|p| p.live) {
            let x = p.x / 16;
            c.fill_rect(x, GROUND - p.h, p.w, p.h, PRIMARY_CONT);
            c.fill_rect(x, GROUND - p.h, p.w, 3, PRIMARY);
            c.fill_rect(x, GROUND - p.h, 2, p.h, PRIMARY);
            c.fill_rect(x + p.w - 2, GROUND - p.h, 2, p.h, PRIMARY);
        }
        // the spark: a glow, a body and a hot core
        let sy = GROUND - SPARK_R - self.y / 64;
        circle(c, SPARK_X, sy, SPARK_R + 4, TERTIARY_CONT);
        circle(c, SPARK_X, sy, SPARK_R, ON_TERTIARY_CONT);
        circle(c, SPARK_X, sy, 4, ON_SURFACE);
        if self.distance < 16 * 200 && !self.over {
            c.text_centered(&SMALL, W / 2, 228, if en { "OK or ▲: jump" } else { "OK или ▲: прыжок" }, ON_SURFACE_VAR);
        }
        if self.over {
            game_over(c, if en { "Game over!" } else { "Игра окончена!" }, en);
        }
    }
}

// ---------------------------------------------------------------- 15 puzzle

const TILE: i32 = 46;
const TILE_GAP: i32 = 4;
const BOARD: i32 = 4 * TILE + 3 * TILE_GAP;
const BOARD_X: i32 = (W - BOARD) / 2;
const BOARD_Y: i32 = 38;

/// The classic sliding puzzle: put 1..15 in order. Arrows slide a tile into the empty cell.
pub struct Fifteen {
    /// row-major, 0 = the empty cell
    tiles: [u8; 16],
    moves: u32,
    solved: bool,
    rng: Rng,
    beep: Beep,
    result: Option<u32>,
}

impl Fifteen {
    fn new(t: u32) -> Self {
        let mut g = Fifteen { tiles: [0; 16], moves: 0, solved: false, rng: Rng::new(t ^ 0x1515), beep: Beep::default(), result: None };
        for (i, v) in g.tiles.iter_mut().enumerate() {
            *v = ((i + 1) % 16) as u8;
        }
        // shuffle by random legal moves, so the puzzle is always solvable
        let mut last = 4;
        let mut n = 0;
        while n < 300 || g.is_solved() {
            let d = g.rng.below(4) as usize;
            if d == last ^ 1 {
                continue; // do not undo the previous move
            }
            if g.slide(d) {
                last = d;
                n += 1;
            }
        }
        g
    }

    fn empty(&self) -> usize {
        self.tiles.iter().position(|&v| v == 0).unwrap_or(15)
    }

    fn is_solved(&self) -> bool {
        self.tiles.iter().enumerate().all(|(i, &v)| v as usize == (i + 1) % 16)
    }

    /// Slide a tile in direction `d` (0 left, 1 right, 2 up, 3 down) into the empty cell.
    fn slide(&mut self, d: usize) -> bool {
        let e = self.empty();
        let (r, c) = (e / 4, e % 4);
        // the tile that moves sits on the opposite side of the empty cell
        let from = match d {
            0 if c < 3 => e + 1,
            1 if c > 0 => e - 1,
            2 if r < 3 => e + 4,
            3 if r > 0 => e - 4,
            _ => return false,
        };
        self.tiles.swap(e, from);
        true
    }

    fn key(&mut self, k: Key, t: u32) -> bool {
        if self.solved {
            if k == Key::Ok {
                *self = Fifteen::new(t);
                return true;
            }
            return false;
        }
        let d = match k {
            Key::Left => 0,
            Key::Right => 1,
            Key::Up => 2,
            Key::Down => 3,
            _ => return false,
        };
        if !self.slide(d) {
            return false;
        }
        self.moves += 1;
        self.beep = Beep { hz: 2500, until: t + 2 };
        if self.is_solved() {
            self.solved = true;
            self.result = Some(self.moves);
            self.beep = Beep { hz: 3500, until: t + 15 };
        }
        true
    }

    fn draw(&self, c: &mut Canvas, title: &str, en: bool, best: Option<u32>) {
        header(c, title, self.moves, best, en);
        for (i, &v) in self.tiles.iter().enumerate() {
            if v == 0 {
                continue;
            }
            let x = BOARD_X + (i % 4) as i32 * (TILE + TILE_GAP);
            let y = BOARD_Y + (i / 4) as i32 * (TILE + TILE_GAP);
            let home = v as usize == i + 1;
            let (bg, fg) = if home { (PRIMARY, ON_PRIMARY) } else { (SURFACE_HI, ON_SURFACE) };
            c.round_rect(x, y, TILE, TILE, 12, bg);
            let mut n = Txt::<4>::new();
            let _ = write!(n, "{v}");
            c.text_centered(&TITLE, x + TILE / 2, y + 32, n.as_str(), fg);
        }
        if self.solved {
            game_over(c, if en { "Solved!" } else { "Собрано!" }, en);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_over_texts_fit_the_dialog() {
        use crate::{SMALL, TITLE};
        let inner = W - 64 - 32; // dialog width minus padding
        for t in ["Игра окончена!", "Game over!", "Собрано!", "Solved!"] {
            assert!(TITLE.width(t) <= inner, "{t}");
        }
        for t in ["OK: заново", "удерживать OK: выход", "OK: play again", "hold OK: exit"] {
            assert!(SMALL.width(t) <= inner, "{t}");
        }
        for i in 0..COUNT {
            for l in 0..LEVELS {
                let t = format!("{} · {}", name(i, false), level_name(l, false));
                assert!(BODY.width(&t) <= 200, "{t}"); // leaves room for the score on the right
            }
        }
    }

    #[test]
    fn snake_grows_and_crashes() {
        let mut g = Snake::new(1, 0);
        g.charge = (8, ROWS as u8 / 2); // right in front of the snake (head at x = 6)
        let mut t = 0;
        while g.score == 0 {
            t += 1;
            g.tick(t);
            assert!(!g.over);
        }
        assert_eq!(g.len, 4);
        g.key(Key::Left, t); // reversing is ignored
        assert_eq!(g.next, (1, 0));
        while !g.over {
            t += 1;
            g.tick(t); // runs into the right wall
        }
        assert_eq!(g.result, Some(1));
    }

    #[test]
    fn snake_levels_differ_in_speed() {
        let ms = |level| Snake::new(level, 0).interval() * 10;
        assert_eq!([ms(0), ms(1), ms(2)], [260, 170, 120]);
        let mut fast = Snake::new(2, 0);
        fast.score = 100;
        assert_eq!(fast.interval() * 10, 50);
    }

    #[test]
    fn runner_jumps_and_lands() {
        let mut g = Runner::new(1, 0);
        g.key(Key::Ok, 0);
        let mut top = 0;
        for t in 1..200 {
            g.tick(t);
            top = top.max(g.y / 64);
        }
        assert!((60..90).contains(&top), "jump height {top}");
        assert_eq!(g.y, 0);
        assert!(Runner::new(0, 0).speed < Runner::new(2, 0).speed);
    }

    #[test]
    fn fifteen_is_shuffled_solvable_and_detects_the_win() {
        let mut g = Fifteen::new(7);
        assert!(!g.is_solved());
        let mut sorted = g.tiles;
        sorted.sort();
        assert_eq!(sorted, core::array::from_fn::<u8, 16, _>(|i| i as u8)); // a permutation of 0..15
        // set up one move from solved: the empty cell left of 15
        g.tiles = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 0, 15];
        assert!(!g.key(Key::Up, 0)); // nothing below the empty cell can move up into it
        assert!(g.key(Key::Left, 0)); // 15 slides left
        assert!(g.solved);
        assert_eq!(g.result, Some(g.moves));
    }
}
