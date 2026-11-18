//! The app: screens, navigation and drawing. Hardware state comes in through `Inputs` and
//! measurement frames; what the hardware must do (send a chip command, save settings, apply
//! backlight/volume) is read back from `App`.
use crate::font_data::*;
use crate::games::{self, Game};
use crate::meter::{group_of, Frame, Mode, Reading, GROUP_DOWN, MODES};
use crate::{rgb, Canvas, Txt, BATT, BODY, HUGE, LARGE, SMALL, TITLE, W};
use core::fmt::Write;

// Material 3 dark scheme, blue seed. Greys keep R/B on multiples of 8 and G of 4 so RGB565 stays neutral.
pub const BG: u16 = rgb(0x10, 0x10, 0x18);
pub const SURFACE: u16 = rgb(0x20, 0x20, 0x28);
pub const SURFACE_HI: u16 = rgb(0x38, 0x38, 0x40);
pub const ON_SURFACE: u16 = rgb(0xE3, 0xE2, 0xE6);
pub const ON_SURFACE_VAR: u16 = rgb(0xA9, 0xAC, 0xB6);
pub const PRIMARY: u16 = rgb(0xA8, 0xC7, 0xFA);
pub const ON_PRIMARY: u16 = rgb(0x06, 0x2E, 0x6F);
pub const PRIMARY_CONT: u16 = rgb(0x26, 0x3D, 0x66);
pub const ON_PRIMARY_CONT: u16 = rgb(0xD3, 0xE3, 0xFD);
pub const TERTIARY_CONT: u16 = rgb(0x5A, 0x3F, 0x20);
pub const ON_TERTIARY_CONT: u16 = rgb(0xFF, 0xDC, 0xB8);
pub const GREEN: u16 = rgb(0x5B, 0xD1, 0x7A);
pub const RED: u16 = rgb(0xFF, 0x6E, 0x62);
const BATT_EMPTY: u16 = rgb(0x78, 0x78, 0x80);

/// Firmware version, taken from ui/Cargo.toml (the release workflow checks the git tag against it).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHOR: &str = "Vladimir Bely";
pub const REPO: &str = "github.com/vlw";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    /// Up held: REL on the measure screen
    UpLong,
    Down,
    Left,
    /// Left held: DC/AC of the current range
    LeftLong,
    Right,
    /// Right held: DC/AC of the voltage range
    RightLong,
    Ok,
    /// OK held: settings / back
    Back,
}

#[derive(Clone, Copy, Default)]
pub struct Inputs {
    pub battery_mv: u32,
    pub charging: bool,
}

/// User settings; persisted by the device as `to_bytes()`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    /// 1..=5
    pub brightness: u8,
    /// index into AUTO_OFF_MIN
    pub auto_off: u8,
    /// UI language: false = Russian, true = English
    pub english: bool,
    pub off_beep: bool,
    /// 1..=4
    pub volume: u8,
    pub fahrenheit: bool,
    /// index into MODES
    pub mode: u8,
    /// mask the measurement chip's beep on every mode change and at power-on
    pub quiet_switch: bool,
    /// start in the last used mode instead of AUTO
    pub remember_mode: bool,
    /// mute the chip in NCV and play our own beeps by field level
    pub own_ncv: bool,
    /// mute the chip in continuity mode and play our own tone (adds the frame latency, ~170 ms)
    pub own_continuity: bool,
    /// the bar under the reading that shows how much of the range is used
    pub range_bar: bool,
}

pub const AUTO_OFF_MIN: [u32; 4] = [0, 5, 15, 30];
const VOLUME_DUTY: [u32; 4] = [6, 20, 60, 191]; // of 384; 191 = stock loudness
const FORMAT: u8 = 3;

impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: 4,
            auto_off: 2,
            english: false,
            off_beep: false,
            volume: 3,
            fahrenheit: false,
            mode: 0,
            quiet_switch: true,
            remember_mode: true,
            own_ncv: false,
            own_continuity: false,
            range_bar: true,
        }
    }
}

impl Settings {
    pub fn to_bytes(&self) -> [u8; 8] {
        let flags = self.english as u8
            | (self.off_beep as u8) << 1
            | (self.fahrenheit as u8) << 2
            | (self.quiet_switch as u8) << 3
            | (self.remember_mode as u8) << 4
            | (self.own_ncv as u8) << 5
            | (self.own_continuity as u8) << 6
            | (!self.range_bar as u8) << 7; // inverted: settings saved before this bit existed keep the bar
        [FORMAT, self.brightness, self.auto_off, self.volume, self.mode, flags, 0, 0]
    }
    pub fn from_bytes(b: &[u8; 8]) -> Option<Self> {
        let f = |bit: u8| b[5] >> bit & 1 != 0;
        let s = Self {
            brightness: b[1],
            auto_off: b[2],
            volume: b[3],
            mode: b[4],
            english: f(0),
            off_beep: f(1),
            fahrenheit: f(2),
            quiet_switch: f(3),
            remember_mode: f(4),
            own_ncv: f(5),
            own_continuity: f(6),
            range_bar: !f(7),
        };
        let ok = b[0] == FORMAT
            && (1..=5).contains(&s.brightness)
            && (s.auto_off as usize) < AUTO_OFF_MIN.len()
            && (1..=4).contains(&s.volume)
            && (s.mode as usize) < MODES.len();
        ok.then_some(s)
    }
    pub fn auto_off_minutes(&self) -> u32 {
        AUTO_OFF_MIN[self.auto_off as usize]
    }
    /// Buzzer PWM compare for the chosen volume.
    pub fn beep_duty(&self) -> u32 {
        VOLUME_DUTY[self.volume as usize - 1]
    }
}

/// What the buzzer should do right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound {
    /// leave it to the measurement chip
    Free,
    /// hold it on so the chip cannot sound
    Mask,
    /// our tone over the masked chip
    Tone { hz: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Measure,
    Menu,
    /// list of games
    Games,
    /// difficulty for the selected game
    Level,
    Play,
}

const ITEMS: u8 = 14;
const LABEL_X: i32 = 58;

/// The control on the right of a menu row.
enum Control {
    Switch(bool),
    Slider(u8, u8),
    Text(Txt<16>),
}

impl Control {
    /// Width it takes at the right edge of the row.
    #[cfg(test)]
    fn width(&self) -> i32 {
        match self {
            Control::Switch(_) => 52,
            Control::Slider(..) => 96,
            Control::Text(v) => BODY.width(v.as_str()),
        }
    }
}
const VISIBLE: u8 = 4;

pub struct App {
    screen: Screen,
    sel: u8,
    top: u8,
    pub s: Settings,
    reading: Option<Reading>,
    hold: bool,
    /// REL reference: (mantissa, divisor, unit)
    rel: Option<(i32, i32, u8)>,
    command: Option<[u8; 10]>,
    /// highlight slide in the mode strip: steps left and the slot it starts from
    anim: u8,
    anim_from: usize,
    /// voltage / current variant to return to when the key comes back to its group
    last_v: Mode,
    last_i: Mode,
    /// mode command not yet confirmed by the chip, and the retries it has had
    awaiting: Option<u8>,
    retries: u8,
    /// "О прошивке" dialog is open
    about: bool,
    /// the running game, the selected row of the games list, best results this session
    game: Option<Game>,
    game_sel: u8,
    /// difficulty row on the Level screen, and the last level picked per game
    level_sel: u8,
    levels: [u8; games::COUNT],
    best: [[Option<u32>; games::LEVELS]; games::COUNT],
    /// current time in 10 ms ticks, from `tick`
    now: u32,
    /// reset confirmation dialog: Some(selected button), 0 = Отмена, 1 = Да
    confirm: Option<u8>,
    unacked_frames: u8,
}

/// Display frames to wait for the chip's mode event before resending (it may still be booting).
const ACK_FRAMES: u8 = 3;
const MAX_RETRIES: u8 = 3;

const ANIM_STEPS: u8 = 6;
/// Screen rows the mode strip occupies (redrawn alone while it animates).
pub const CAROUSEL_ROWS: (i32, i32) = (188, 240);

/// Battery charge in percent; stock firmware's linear model, 3.40 V = 0 %, 4.00 V = 100 %.
pub fn battery_percent(mv: u32) -> u32 {
    (mv.saturating_sub(3400) / 6).min(100)
}

impl App {
    pub fn new(mut s: Settings) -> Self {
        if !s.remember_mode {
            s.mode = 0; // AUTO
        }
        let mode = MODES[s.mode as usize];
        Self {
            screen: Screen::Measure,
            sel: 0,
            top: 0,
            s,
            reading: None,
            hold: false,
            rel: None,
            command: Some(mode.command()),
            anim: 0,
            anim_from: 0,
            last_v: if mode == Mode::AcV { Mode::AcV } else { Mode::DcV },
            last_i: if matches!(mode, Mode::AcMa | Mode::DcA | Mode::AcA) { mode } else { Mode::DcMa },
            awaiting: Some(mode as u8),
            retries: 0,
            about: false,
            game: None,
            game_sel: 0,
            level_sel: 1,
            levels: [1; games::COUNT],
            best: [[None; games::LEVELS]; games::COUNT],
            now: 0,
            confirm: None,
            unacked_frames: 0,
        }
    }

    pub fn mode(&self) -> Mode {
        MODES[self.s.mode as usize]
    }

    /// A chip command the device must send now (mode selection).
    pub fn take_command(&mut self) -> Option<[u8; 10]> {
        self.command.take()
    }

    /// Advance time to `t` (10 ms ticks); returns true if the screen must be redrawn.
    pub fn tick(&mut self, t: u32) -> bool {
        self.now = t;
        let dirty = match (self.screen, &mut self.game) {
            (Screen::Play, Some(g)) => g.tick(t),
            _ => false,
        };
        self.collect_result();
        dirty
    }

    /// A game is on screen: arrows should act on press, without long-press variants.
    pub fn in_game(&self) -> bool {
        self.screen == Screen::Play
    }

    fn collect_result(&mut self) {
        if let Some(g) = &mut self.game {
            let (i, l) = (g.index(), g.level());
            if let Some(r) = g.take_result() {
                let better = match self.best[i][l] {
                    None => true,
                    Some(b) if games::lower_is_better(i) => r < b,
                    Some(b) => r > b,
                };
                if better && (r > 0 || games::lower_is_better(i)) {
                    self.best[i][l] = Some(r);
                }
            }
        }
    }

    /// A mode command is out and the chip has not confirmed it yet.
    pub fn awaiting_ack(&self) -> bool {
        self.awaiting.is_some()
    }

    /// Buzzer state for own NCV / continuity sounds at time `t10` (10 ms ticks).
    pub fn sound(&self, t10: u32) -> Sound {
        if let (Screen::Play, Some(g)) = (self.screen, &self.game) {
            return g.sound(t10);
        }
        let r = self.reading;
        match self.mode() {
            Mode::Ncv if self.s.own_ncv => {
                let level = r.map(|r| r.symbols.iter().filter(|&&s| s == 16).count()).unwrap_or(0);
                // more bars: higher pitch, faster pulses, continuous at full scale
                let (hz, period) = match level {
                    0 => return Sound::Mask,
                    1 => (2500, 60),
                    2 => (3000, 35),
                    3 => (3500, 18),
                    _ => (4000, 0),
                };
                if period == 0 || t10 % period < 6 {
                    Sound::Tone { hz }
                } else {
                    Sound::Mask
                }
            }
            Mode::Diode if self.s.own_continuity => {
                // shorted probes: the chip shows a small number of volts
                let short = r.and_then(|r| r.number()).is_some_and(|(m, d)| m.abs() * 1000 < 50 * d);
                if short {
                    Sound::Tone { hz: 2700 }
                } else {
                    Sound::Mask
                }
            }
            _ => Sound::Free,
        }
    }

    pub fn animating(&self) -> bool {
        self.anim > 0
    }

    /// Advance the carousel animation by one frame (call every ~20 ms while `animating`).
    pub fn step_anim(&mut self) {
        self.anim = self.anim.saturating_sub(1);
    }

    pub fn in_menu(&self) -> bool {
        self.screen == Screen::Menu
    }

    /// A frame from the measurement chip; returns true if the screen must be redrawn.
    pub fn on_frame(&mut self, f: Frame) -> bool {
        match f {
            Frame::Event(id) if self.awaiting == Some(id) => {
                self.awaiting = None;
                false
            }
            // other events while a command is pending are the chip's own start-up chatter
            Frame::Event(_) if self.awaiting.is_some() => false,
            Frame::Event(id) => match Mode::from_id(id) {
                Some(m) if m != self.mode() => {
                    self.s.mode = MODES.iter().position(|x| *x == m).unwrap_or(0) as u8;
                    self.reading = None;
                    self.rel = None;
                    self.screen == Screen::Measure
                }
                _ => false,
            },
            Frame::Display(_) if self.awaiting.is_some() => {
                // the chip is talking but has not confirmed our mode: it missed the command
                self.unacked_frames += 1;
                if self.unacked_frames >= ACK_FRAMES {
                    self.unacked_frames = 0;
                    if self.retries < MAX_RETRIES {
                        self.retries += 1;
                        self.command = Some(self.mode().command());
                    } else {
                        self.awaiting = None; // never confirmed (e.g. a mode without an event): show what it sends
                    }
                }
                false
            }
            Frame::Display(r) => {
                if self.hold || self.reading == Some(r) {
                    return false;
                }
                // the chip blanks its digits while it changes range: keep the last value on screen
                // (NCV is exempt: there a blank display means "no field")
                if r.is_blank() && self.reading.is_some() && self.mode() != Mode::Ncv {
                    return false;
                }
                self.reading = Some(r);
                self.screen == Screen::Measure
            }
        }
    }

    /// Handle a key; returns true if the screen must be redrawn.
    pub fn key(&mut self, k: Key) -> bool {
        match self.screen {
            Screen::Measure => self.measure_key(k),
            Screen::Menu => self.menu_key(k),
            Screen::Games => self.games_key(k),
            Screen::Level => self.level_key(k),
            Screen::Play => {
                if k == Key::Back {
                    self.screen = Screen::Games;
                    self.game = None;
                    return true;
                }
                let now = self.now;
                let dirty = self.game.as_mut().is_some_and(|g| g.key(k, now));
                self.collect_result();
                dirty
            }
        }
    }

    fn games_key(&mut self, k: Key) -> bool {
        let n = games::COUNT as u8;
        match k {
            Key::Up | Key::UpLong => self.game_sel = (self.game_sel + n - 1) % n,
            Key::Down => self.game_sel = (self.game_sel + 1) % n,
            Key::Ok if games::has_levels(self.game_sel as usize) => {
                self.level_sel = self.levels[self.game_sel as usize];
                self.screen = Screen::Level;
            }
            Key::Ok => self.start_game(0),
            Key::Back => self.screen = Screen::Menu,
            _ => return false,
        }
        true
    }

    fn level_key(&mut self, k: Key) -> bool {
        let n = games::LEVELS as u8;
        match k {
            Key::Up | Key::UpLong | Key::Left | Key::LeftLong => self.level_sel = self.level_sel.saturating_sub(1),
            Key::Down | Key::Right | Key::RightLong => self.level_sel = (self.level_sel + 1).min(n - 1),
            Key::Ok => {
                self.levels[self.game_sel as usize] = self.level_sel;
                self.start_game(self.level_sel as usize);
            }
            Key::Back => self.screen = Screen::Games,
        }
        true
    }

    fn start_game(&mut self, level: usize) {
        self.game = Some(Game::new(self.game_sel as usize, level, self.now));
        self.screen = Screen::Play;
    }

    fn set_mode(&mut self, m: Mode) {
        let (_, old_n, old_i) = self.slots();
        let old_group = group_of(self.mode());
        self.s.mode = MODES.iter().position(|x| *x == m).unwrap_or(0) as u8;
        self.reading = None;
        self.rel = None;
        self.hold = false;
        self.command = Some(m.command());
        self.awaiting = Some(m as u8);
        self.retries = 0;
        self.unacked_frames = 0;
        // slide the highlight when it moves inside the same strip
        let (_, n, i) = self.slots();
        let same_strip = n == old_n && group_of(m) == old_group;
        self.anim = if same_strip && i != old_i { ANIM_STEPS } else { 0 };
        self.anim_from = old_i;
    }

    /// The strip under the reading: modes of the current key, how many, and the selected slot.
    /// Voltage and current slots stand for their DC or AC variant, whichever is selected.
    fn slots(&self) -> ([Mode; 4], usize, usize) {
        let m = self.mode();
        let pos = |a: &[Mode]| a.iter().position(|x| *x == m).unwrap_or(0);
        match m {
            Mode::DcV | Mode::AcV | Mode::Ohm | Mode::Cap | Mode::Diode => {
                let v = if m == Mode::AcV { Mode::AcV } else { self.last_v };
                let a = [v, Mode::Ohm, Mode::Cap, Mode::Diode];
                (a, 4, if matches!(m, Mode::DcV | Mode::AcV) { 0 } else { pos(&a) })
            }
            Mode::DcMa | Mode::DcA => ([Mode::DcMa, Mode::DcA, Mode::Auto, Mode::Auto], 2, pos(&[Mode::DcMa, Mode::DcA])),
            Mode::AcMa | Mode::AcA => ([Mode::AcMa, Mode::AcA, Mode::Auto, Mode::Auto], 2, pos(&[Mode::AcMa, Mode::AcA])),
            Mode::Temp | Mode::Freq | Mode::Ncv | Mode::Live => (GROUP_DOWN, 4, pos(&GROUP_DOWN)),
            Mode::Auto => ([Mode::Auto; 4], 1, 0),
        }
    }

    fn measure_key(&mut self, k: Key) -> bool {
        let m = self.mode();
        match k {
            // right: V -> Ω -> C -> diode, voltage keeps its DC/AC choice; held: DC <-> AC
            Key::Right => self.set_mode(match m {
                Mode::DcV | Mode::AcV => Mode::Ohm,
                Mode::Ohm => Mode::Cap,
                Mode::Cap => Mode::Diode,
                _ => self.last_v,
            }),
            Key::RightLong => match m {
                Mode::DcV => self.set_mode(Mode::AcV),
                Mode::AcV => self.set_mode(Mode::DcV),
                _ => return false,
            },
            // left: mA <-> A, keeping DC/AC; held: DC <-> AC
            Key::Left => self.set_mode(match m {
                Mode::DcMa => Mode::DcA,
                Mode::DcA => Mode::DcMa,
                Mode::AcMa => Mode::AcA,
                Mode::AcA => Mode::AcMa,
                _ => self.last_i,
            }),
            Key::LeftLong => match m {
                Mode::DcMa => self.set_mode(Mode::AcMa),
                Mode::AcMa => self.set_mode(Mode::DcMa),
                Mode::DcA => self.set_mode(Mode::AcA),
                Mode::AcA => self.set_mode(Mode::DcA),
                _ => return false,
            },
            Key::Down => {
                let i = GROUP_DOWN.iter().position(|x| *x == m).map_or(0, |i| (i + 1) % GROUP_DOWN.len());
                self.set_mode(GROUP_DOWN[i]);
            }
            Key::Ok => self.set_mode(Mode::Auto),
            Key::Up => self.hold = !self.hold,
            Key::UpLong => {
                self.rel = match (self.rel, self.reading.and_then(|r| r.number().map(|(m, d)| (m, d, r.unit)))) {
                    (None, Some(v)) => Some(v),
                    _ => None,
                }
            }
            Key::Back => {
                self.screen = Screen::Menu;
                self.sel = 0;
                self.top = 0;
            }
        }
        match self.mode() {
            v @ (Mode::DcV | Mode::AcV) => self.last_v = v,
            i @ (Mode::DcMa | Mode::AcMa | Mode::DcA | Mode::AcA) => self.last_i = i,
            _ => {}
        }
        true
    }

    fn menu_key(&mut self, k: Key) -> bool {
        if self.about {
            if matches!(k, Key::Ok | Key::Back) {
                self.about = false;
                return true;
            }
            return false;
        }
        if let Some(b) = self.confirm {
            match k {
                Key::Left | Key::LeftLong => self.confirm = Some(0),
                Key::Right | Key::RightLong => self.confirm = Some(1),
                Key::Ok => {
                    if b == 1 {
                        self.s = Settings { mode: self.s.mode, ..Settings::default() };
                    }
                    self.confirm = None;
                }
                Key::Back => self.confirm = None,
                _ => return false,
            }
            return true;
        }
        let s = &mut self.s;
        let step = |v: u8, lo: u8, hi: u8, k: Key| if k == Key::Left { v.saturating_sub(1).max(lo) } else { (v + 1).min(hi) };
        match k {
            Key::Back => self.screen = Screen::Measure,
            Key::Up | Key::UpLong => self.sel = (self.sel + ITEMS - 1) % ITEMS,
            Key::LeftLong => return self.menu_key(Key::Left),
            Key::RightLong => return self.menu_key(Key::Right),
            Key::Down => self.sel = (self.sel + 1) % ITEMS,
            Key::Left | Key::Right | Key::Ok => match self.sel {
                0 if k != Key::Ok => s.brightness = step(s.brightness, 1, 5, k),
                1 if k == Key::Left => s.auto_off = (s.auto_off + 3) % 4,
                1 => s.auto_off = (s.auto_off + 1) % 4,
                2 => s.off_beep = !s.off_beep,
                3 => s.quiet_switch = !s.quiet_switch,
                4 => s.own_ncv = !s.own_ncv,
                5 => s.own_continuity = !s.own_continuity,
                6 => s.remember_mode = !s.remember_mode,
                7 if k != Key::Ok => s.volume = step(s.volume, 1, 4, k),
                8 => s.range_bar = !s.range_bar,
                9 => s.fahrenheit = !s.fahrenheit,
                10 => s.english = !s.english,
                11 if k == Key::Ok => self.confirm = Some(0), // ask first; "Отмена" preselected
                12 if k == Key::Ok => {
                    self.screen = Screen::Games;
                    self.game_sel = 0;
                }
                13 if k == Key::Ok => self.about = true,
                _ => return false,
            },
        }
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + VISIBLE {
            self.top = self.sel + 1 - VISIBLE;
        }
        true
    }

    pub fn draw(&self, c: &mut Canvas, inp: &Inputs) {
        let en = self.s.english;
        if let (Screen::Play, Some(g)) = (self.screen, &self.game) {
            return g.draw(c, en, self.best[g.index()][g.level()]);
        }
        c.clear(BG);
        let title = match self.screen {
            Screen::Menu => tr(en, "Настройки", "Settings"),
            Screen::Games => tr(en, "Игры", "Games"),
            Screen::Level => games::name(self.game_sel as usize, en),
            _ => self.mode().name(en),
        };
        c.text(&BODY, STATUS_MARGIN, status_baseline(), title, ON_SURFACE);
        self.status_icons(c, inp);
        match self.screen {
            Screen::Measure => self.measure(c),
            Screen::Games | Screen::Play => self.games_list(c),
            Screen::Level => self.level_list(c),
            Screen::Menu => {
                self.menu(c);
                if let Some(b) = self.confirm {
                    confirm_dialog(c, b, self.s.english);
                }
                if self.about {
                    about_dialog(c, self.s.english);
                }
            }
        }
    }

    /// Right side of the status bar: battery at the edge, then status icons, all on one centre line.
    fn status_icons(&self, c: &mut Canvas, inp: &Inputs) {
        let mut x = battery_icon(c, W - STATUS_MARGIN, STATUS_CY, inp) - STATUS_GAP;
        let icons = [
            (self.s.quiet_switch, IC_NOTIFICATIONS_OFF),
            (self.hold && self.screen == Screen::Measure, IC_PAUSE),
        ];
        for (shown, icon) in icons {
            if shown {
                x = status_icon(c, x, icon) - STATUS_GAP;
            }
        }
    }

    fn measure(&self, c: &mut Canvas) {
        let mode = self.mode();
        c.round_rect(12, 36, W - 24, 148, 28, SURFACE);
        // chips: AC/DC, HOLD, REL
        let mut cx = 28;
        let mut chip = |c: &mut Canvas, t: &str, bg: u16, fg: u16| {
            let w = SMALL.width(t) + 24;
            c.round_rect(cx, 50, w, 26, 13, bg);
            c.text(&SMALL, cx + 12, 68, t, fg);
            cx += w + 8;
        };
        match self.reading.map(|r| r.ac_dc) {
            Some(1) => chip(c, "AC", PRIMARY_CONT, ON_PRIMARY_CONT),
            Some(2) => chip(c, "DC", PRIMARY_CONT, ON_PRIMARY_CONT),
            _ => {}
        }
        if self.hold {
            chip(c, "HOLD", TERTIARY_CONT, ON_TERTIARY_CONT);
        }
        if self.rel.is_some() {
            chip(c, "REL", TERTIARY_CONT, ON_TERTIARY_CONT);
        }
        match (mode, self.reading) {
            (_, None) => {} // waiting for the chip to confirm the mode: the card stays empty
            (Mode::Ncv, Some(r)) => {
                let level = r.symbols.iter().filter(|&&s| s == 16).count() as i32;
                for i in 0..4 {
                    let h = 20 + i * 18;
                    let col = if i < level { if level >= 3 { RED } else { PRIMARY } } else { SURFACE_HI };
                    c.round_rect(40 + i * 40, 164 - h, 28, h, 8, col);
                }
                c.text_right(&BODY, W - 32, 164, if level > 0 { tr(self.s.english, "Поле есть", "Field") } else { tr(self.s.english, "Нет поля", "No field") }, ON_SURFACE_VAR);
            }
            (Mode::Live, Some(r)) => {
                let active = r.symbols[0] != 14;
                let en = self.s.english;
                let (t, col) = if active { (tr(en, "Фаза!", "Live!"), RED) } else { (tr(en, "Нет фазы", "No live wire"), ON_SURFACE_VAR) };
                big_word(c, t, CARD_CY, col);
            }
            (_, Some(r)) => self.value(c, &r),
        }
        self.carousel(c);
    }

    /// The current key's modes in fixed cells; the selected one is highlighted, and the
    /// highlight slides over when the selection moves.
    pub fn carousel(&self, c: &mut Canvas) {
        let (slots, n, sel) = self.slots();
        let (x0, cy, h) = (12, 214, 36);
        let cw = (W - 24) / n as i32;
        let cell_x = |i: usize| x0 + i as i32 * cw;
        c.round_rect(x0, cy - h / 2, W - 24, h, h / 2, SURFACE);
        // ease-out from the previous cell to the selected one
        let a = self.anim as i32;
        let t = ANIM_STEPS as i32;
        let hx = cell_x(sel) + (cell_x(self.anim_from) - cell_x(sel)) * a * a / (t * t);
        c.round_rect(hx + 3, cy - h / 2 + 3, cw - 6, h - 6, (h - 6) / 2, PRIMARY);
        for (i, m) in slots.iter().take(n).enumerate() {
            let col = if i == sel { ON_PRIMARY } else { ON_SURFACE_VAR };
            mode_label(c, *m, cell_x(i) + cw / 2, cy, col, self.s.fahrenheit, self.s.english);
        }
    }

    fn value(&self, c: &mut Canvas, r: &Reading) {
        let mut unit = r.unit_str();
        let mut t = Txt::<16>::new();
        let mut neg = false;
        match r.number() {
            Some((mut m, d)) => {
                if r.unit == 21 && self.s.fahrenheit {
                    m = m * 9 / 5 + 32 * d;
                    unit = "°F";
                }
                if let Some((rm, rd, ru)) = self.rel {
                    if ru == r.unit && rd == d {
                        m -= rm;
                    }
                }
                neg = m < 0;
                let a = m.unsigned_abs();
                let _ = match d as u32 {
                    1 => write!(t, "{a}"),
                    10 => write!(t, "{}.{}", a / 10, a % 10),
                    100 => write!(t, "{}.{:02}", a / 100, a % 100),
                    _ => write!(t, "{}.{:03}", a / 1000, a % 1000),
                };
            }
            None if r.is_overload() => {
                let _ = write!(t, "OL");
            }
            None => {
                let mut b = [0u8; 8];
                let n = r.text(&mut b);
                let s = core::str::from_utf8(&b[..n]).unwrap_or("").trim();
                if s == "AUTO" {
                    big_word(c, s, CARD_CY - 12, ON_SURFACE);
                    c.text_centered(&BODY, W / 2, CARD_CY + 36, tr(self.s.english, "Приложите щупы к цепи", "Touch the probes to a circuit"), ON_SURFACE_VAR);
                } else {
                    big_word(c, s, CARD_CY, ON_SURFACE);
                }
                return;
            }
        }
        // Centre "−digits" with a minus slot that is always reserved: Roboto digits are tabular,
        // so the number stays put when the sign or the value changes.
        let mw = HUGE.width("-");
        let x = (W - mw - HUGE.width(t.as_str())) / 2;
        let y = digits_y(self.s.range_bar);
        if neg {
            c.text(&HUGE, x, y, "-", ON_SURFACE);
        }
        c.text(&HUGE, x + mw, y, t.as_str(), ON_SURFACE);
        c.text_right(&TITLE, W - 32, 72, unit, PRIMARY);
        if let Some((fill, full)) = r.range().filter(|_| self.s.range_bar) {
            range_bar(c, fill, full, r.unit_str());
        }
    }

    /// Menu row `i`: icon, label and the control on its right.
    fn item(&self, i: u8) -> (&'static str, &'static str, Control) {
        let en = self.s.english;
        let s = &self.s;
        let text = |t: &str| {
            let mut v = Txt::<16>::new();
            let _ = v.write_str(t);
            Control::Text(v)
        };
        match i {
            0 => (IC_BRIGHTNESS_6, tr(en, "Яркость", "Brightness"), Control::Slider(s.brightness, 5)),
            1 => {
                let m = s.auto_off_minutes();
                let mut v = Txt::<16>::new();
                let _ = if m == 0 { write!(v, "{}", tr(en, "Выкл", "Off")) } else { write!(v, "{m} {}", tr(en, "мин", "min")) };
                (IC_TIMER, tr(en, "Автовыключение", "Auto power-off"), Control::Text(v))
            }
            2 => (IC_POWER_SETTINGS_NEW, tr(en, "Звук выключения", "Power-off beep"), Control::Switch(s.off_beep)),
            3 => (IC_NOTIFICATIONS_OFF, tr(en, "Тихая смена", "Quiet switching"), Control::Switch(s.quiet_switch)),
            4 => (IC_SENSORS, tr(en, "Свой звук NCV", "Own NCV sound"), Control::Switch(s.own_ncv)),
            5 => (IC_CABLE, tr(en, "Своя прозвонка", "Own continuity"), Control::Switch(s.own_continuity)),
            6 => (IC_HISTORY, tr(en, "Помнить режим", "Remember mode"), Control::Switch(s.remember_mode)),
            7 => (IC_VOLUME_UP, tr(en, "Громкость", "Volume"), Control::Slider(s.volume, 4)),
            8 => (IC_LINEAR_SCALE, tr(en, "Полоса диапазона", "Range bar"), Control::Switch(s.range_bar)),
            9 => (IC_THERMOSTAT, tr(en, "Температура", "Temperature"), text(if s.fahrenheit { "°F" } else { "°C" })),
            10 => (IC_LANGUAGE, tr(en, "Язык", "Language"), text(tr(en, "Русский", "English"))),
            11 => (IC_RESTART_ALT, tr(en, "Сброс настроек", "Reset settings"), text("OK")),
            12 => (IC_SPORTS_ESPORTS, tr(en, "Игры", "Games"), text("›")),
            _ => {
                let mut v = Txt::<16>::new();
                let _ = write!(v, "v{VERSION}");
                (IC_INFO, tr(en, "О прошивке", "About"), Control::Text(v))
            }
        }
    }

    /// A list row: icon, label and a value on the right.
    fn row(&self, c: &mut Canvas, y: i32, sel: bool, icon: &str, label: &str, value: &str) {
        let (bg, fg) = if sel { (PRIMARY_CONT, ON_PRIMARY_CONT) } else { (SURFACE, ON_SURFACE) };
        c.round_rect(12, y, W - 24, 46, 16, bg);
        if !icon.is_empty() {
            c.text(&ICON, 26, y + 34, icon, if sel { ON_PRIMARY_CONT } else { ON_SURFACE_VAR });
        }
        c.text(&BODY, if icon.is_empty() { 30 } else { LABEL_X }, y + 30, label, fg);
        c.text_right(&BODY, W - 28, y + 30, value, if sel { ON_PRIMARY_CONT } else { ON_SURFACE_VAR });
    }

    /// This session's best for game `i` on `level`, as text.
    fn best_text(&self, i: usize, level: usize) -> Txt<16> {
        let mut v = Txt::<16>::new();
        let en = self.s.english;
        let _ = match self.best[i][level] {
            Some(b) if games::lower_is_better(i) => write!(v, "{b} {}", tr(en, "ход.", "moves")),
            Some(b) => write!(v, "{b}"),
            None => write!(v, "—"),
        };
        v
    }

    /// Games list: icon, name and the best result (on the last level played).
    fn games_list(&self, c: &mut Canvas) {
        let en = self.s.english;
        for i in 0..games::COUNT {
            let icon = [IC_CABLE, IC_DIRECTIONS_RUN, IC_GRID_VIEW][i];
            let best = self.best_text(i, if games::has_levels(i) { self.levels[i] as usize } else { 0 });
            self.row(c, 36 + i as i32 * 50, self.game_sel as usize == i, icon, games::name(i, en), best.as_str());
        }
        c.text_centered(&SMALL, W / 2, 222, tr(en, "OK: играть   удерживать OK: назад", "OK: play   hold OK: back"), ON_SURFACE_VAR);
    }

    /// Difficulty choice with the best result on each level.
    fn level_list(&self, c: &mut Canvas) {
        let en = self.s.english;
        let g = self.game_sel as usize;
        for l in 0..games::LEVELS {
            let best = self.best_text(g, l);
            self.row(c, 36 + l as i32 * 50, self.level_sel as usize == l, "", games::level_name(l, en), best.as_str());
        }
        c.text_centered(&SMALL, W / 2, 222, tr(en, "OK: начать   удерживать OK: назад", "OK: start   hold OK: back"), ON_SURFACE_VAR);
    }

    fn menu(&self, c: &mut Canvas) {
        for i in self.top..(self.top + VISIBLE).min(ITEMS) {
            let y = 36 + (i - self.top) as i32 * 50;
            let sel = self.sel == i;
            let (bg, fg) = if sel { (PRIMARY_CONT, ON_PRIMARY_CONT) } else { (SURFACE, ON_SURFACE) };
            c.round_rect(12, y, W - 24, 46, 16, bg);
            let (icon, label, ctl) = self.item(i);
            c.text(&ICON, 26, y + 34, icon, if sel { ON_PRIMARY_CONT } else { ON_SURFACE_VAR });
            c.text(&BODY, LABEL_X, y + 30, label, fg);
            let rx = W - 28;
            match ctl {
                Control::Switch(on) => switch(c, rx - 52, y + 7, on),
                Control::Slider(v, max) => slider(c, rx - 96, y + 23, 96, v as i32, max as i32, sel),
                Control::Text(v) => c.text_right(&BODY, rx, y + 30, v.as_str(), if sel { ON_PRIMARY_CONT } else { ON_SURFACE_VAR }),
            }
        }
        // scroll indicator
        let track = VISIBLE as i32 * 50 - 4;
        let th = track * VISIBLE as i32 / ITEMS as i32;
        let ty = 36 + (track - th) * self.top as i32 / (ITEMS - VISIBLE) as i32;
        c.round_rect(W - 6, ty, 3, th, 1, SURFACE_HI);
    }
}

/// Material 3 dialog over a dimmed screen: "Сбросить настройки?" with Отмена / Да.
fn confirm_dialog(c: &mut Canvas, sel: u8, en: bool) {
    c.dim(150);
    let (x, y, w, h) = (28, 46, W - 56, 152);
    c.round_rect(x, y, w, h, 28, SURFACE_HI);
    c.text(&ICON, x + 24, y + 40, IC_RESTART_ALT, PRIMARY);
    c.text(&TITLE, x + 56, y + 38, tr(en, "Сбросить?", "Reset?"), ON_SURFACE);
    c.text(&SMALL, x + 24, y + 70, tr(en, "Все настройки вернутся", "All settings will return"), ON_SURFACE_VAR);
    c.text(&SMALL, x + 24, y + 88, tr(en, "к значениям по умолчанию.", "to their defaults."), ON_SURFACE_VAR);
    for (i, label) in [tr(en, "Отмена", "Cancel"), tr(en, "Да", "Yes")].iter().enumerate() {
        let bw = 100;
        let bx = x + w - 24 - bw * (2 - i as i32) - 8 * (1 - i as i32);
        let by = y + h - 50;
        let active = sel as usize == i;
        if active {
            c.round_rect(bx, by, bw, 36, 18, PRIMARY);
        }
        c.text_centered(&BODY, bx + bw / 2, by + 24, label, if active { ON_PRIMARY } else { PRIMARY });
    }
}

/// "О прошивке": version, author and repository, closed with ОК.
fn about_dialog(c: &mut Canvas, en: bool) {
    c.dim(150);
    let (x, y, w, h) = (28, 30, W - 56, 184);
    c.round_rect(x, y, w, h, 28, SURFACE_HI);
    c.text(&ICON, x + 24, y + 40, IC_INFO, PRIMARY);
    c.text(&TITLE, x + 56, y + 38, tr(en, "О прошивке", "About"), ON_SURFACE);
    let mut v = Txt::<24>::new();
    let _ = write!(v, "DMT-99 custom v{VERSION}");
    c.text(&BODY, x + 24, y + 72, v.as_str(), ON_SURFACE);
    let label_w = c.text(&SMALL, x + 24, y + 96, tr(en, "Автор:", "Author:"), ON_SURFACE_VAR);
    c.text(&SMALL, x + 24 + label_w + 6, y + 96, AUTHOR, ON_SURFACE);
    c.text(&crate::BRAND, x + 24, y + 124, IC_GITHUB, ON_SURFACE);
    c.text(&SMALL, x + 50, y + 122, REPO, PRIMARY);
    let (bw, bx, by) = (100, x + w - 24 - 100, y + h - 50);
    c.round_rect(bx, by, bw, 36, 18, PRIMARY);
    c.text_centered(&BODY, bx + bw / 2, by + 24, tr(en, "ОК", "OK"), ON_PRIMARY);
}

/// Pick the string for the UI language.
fn tr(en: bool, ru: &'static str, english: &'static str) -> &'static str {
    if en {
        english
    } else {
        ru
    }
}

/// Mode label centred at (cx, cy): drawn symbols for capacitor and diode, text otherwise.
fn mode_label(c: &mut Canvas, m: Mode, cx: i32, cy: i32, col: u16, fahrenheit: bool, en: bool) {
    match m {
        Mode::Cap => {
            // -| |-  two plates with leads
            c.fill_rect(cx - 14, cy - 1, 10, 2, col);
            c.fill_rect(cx - 5, cy - 9, 3, 18, col);
            c.fill_rect(cx + 2, cy - 9, 3, 18, col);
            c.fill_rect(cx + 4, cy - 1, 10, 2, col);
        }
        Mode::Diode => {
            // ->|-  triangle, bar, leads
            c.fill_rect(cx - 15, cy - 1, 8, 2, col);
            for dy in -8..=8 {
                let w = 14 - (dy as i32).abs() * 14 / 8;
                c.fill_rect(cx - 7, cy + dy, w, 1, col);
            }
            c.fill_rect(cx + 7, cy - 8, 2, 17, col);
            c.fill_rect(cx + 8, cy - 1, 7, 2, col);
        }
        Mode::Ohm => c.text_centered(&TITLE, cx, cy + 8, "Ω", col),
        Mode::Temp if fahrenheit => c.text_centered(&BODY, cx, cy + 6, "°F", col),
        _ => c.text_centered(&BODY, cx, cy + 6, m.short(en), col),
    }
}

/// Bottom of the chips row and the unit, the top of the space for the big reading.
const CHIPS_BOTTOM: i32 = 77;
/// Bottom of the space for the big reading when there is no range bar.
const CARD_CONTENT_BOTTOM: i32 = 174;

/// Baseline of the big reading: digit ink centred between the unit above and the range labels
/// (or the card's bottom) below, so the air above and below it is the same.
fn digits_y(range_bar: bool) -> i32 {
    let bottom = if range_bar { RANGE_LABEL_Y + SMALL.ink("0").map_or(-10, |(_, _, t, _)| t) } else { CARD_CONTENT_BOTTOM };
    let (_, _, t, b) = HUGE.ink("0").unwrap_or((0, 0, -64, 0));
    (CHIPS_BOTTOM + bottom - t - b) / 2
}
/// Vertical centre of the measurement card's content area (below the chips row).
const CARD_CY: i32 = 118;

/// A status word centred in the card: large where the large font has its letters, else the title font.
fn big_word(c: &mut Canvas, s: &str, cy: i32, col: u16) {
    let f = if s.chars().all(|ch| ch == ' ' || LARGE.glyph(ch).is_some()) { &LARGE } else { &TITLE };
    c.text_mid(f, W / 2, cy, s, col);
}

/// Range bar: spans the card with the same inset on both sides as the chips and the unit,
/// "0" and the full scale are written above its ends.
const RANGE_X: i32 = 28;
const RANGE_Y: i32 = 172;
const RANGE_LABEL_Y: i32 = 166;

/// Material 3 linear indicator under the reading: how much of the chip's current range is used
/// (`fill` of 10000), from "0" to the range's full scale.
fn range_bar(c: &mut Canvas, fill: u32, full: u32, unit: &str) {
    let w = W - 2 * RANGE_X;
    c.round_rect(RANGE_X, RANGE_Y, w, 4, 2, SURFACE_HI);
    let fw = (w as u32 * fill / 10_000) as i32;
    if fw > 0 {
        c.round_rect(RANGE_X, RANGE_Y, fw.max(4), 4, 2, PRIMARY);
    }
    let mut label = Txt::<16>::new();
    let _ = write!(label, "{full} {unit}");
    c.text(&SMALL, RANGE_X - SMALL.ink("0").map_or(0, |(l, ..)| l), RANGE_LABEL_Y, "0", ON_SURFACE_VAR);
    c.text_right(&SMALL, W - RANGE_X, RANGE_LABEL_Y, label.as_str(), ON_SURFACE_VAR);
}

/// Status bar geometry: everything is centred on one line, with equal outer margins and gaps.
const STATUS_CY: i32 = 16;
const STATUS_MARGIN: i32 = 16;
const STATUS_GAP: i32 = 6;

/// Baseline that puts capital letters of the status title on the status centre line.
fn status_baseline() -> i32 {
    BODY.ink("Н").map_or(23, |(_, _, t, b)| STATUS_CY - (t + b) / 2)
}

/// A status icon whose ink ends at `right`; returns its left edge.
fn status_icon(c: &mut Canvas, right: i32, icon: &str) -> i32 {
    let w = ICON_FILL.ink(icon).map_or(0, |(l, r, _, _)| r - l);
    c.text_mid(&ICON_FILL, right - w / 2, STATUS_CY, icon, ON_SURFACE);
    right - w
}

/// Android 16 style battery, right edge at `right`, centred on `cy`: filled pill with the bold
/// percentage inside, a small cap, and a bolt beside it while charging. Returns the left edge.
fn battery_icon(c: &mut Canvas, right: i32, cy: i32, inp: &Inputs) -> i32 {
    let (bw, bh) = (30, 16);
    let y = cy - bh / 2;
    let mut x = right;
    if inp.charging {
        x = status_icon(c, x, IC_BOLT) - 2;
    }
    x -= 3;
    c.round_rect(x, cy - 3, 3, 6, 1, BATT_EMPTY);
    x -= bw + 1;
    let p = battery_percent(inp.battery_mv) as i32;
    let fill = if inp.charging {
        GREEN
    } else if p <= 20 {
        RED
    } else {
        ON_SURFACE
    };
    c.round_rect(x, y, bw, bh, 5, BATT_EMPTY);
    let fw = bw * p / 100;
    if fw > 0 {
        let save = c.clip;
        c.clip = (x, x + fw);
        c.round_rect(x, y, bw, bh, 5, fill);
        c.clip = save;
    }
    let mut t = Txt::<4>::new();
    let _ = write!(t, "{}", p);
    c.text_mid(&BATT, x + bw / 2, cy, t.as_str(), BG);
    x
}

/// Material 3 slider with a gap around the handle.
fn slider(c: &mut Canvas, x: i32, cy: i32, w: i32, v: i32, max: i32, active: bool) {
    let hx = x + (w - 4) * (v - 1) / (max - 1);
    let on = if active { PRIMARY } else { ON_SURFACE_VAR };
    if hx - 4 > x {
        c.round_rect(x, cy - 6, hx - 4 - x, 12, 6, on);
    }
    if x + w > hx + 8 {
        c.round_rect(hx + 8, cy - 6, x + w - hx - 8, 12, 6, SURFACE_HI);
    }
    c.round_rect(hx, cy - 14, 4, 28, 2, on);
}

/// Material 3 switch.
fn switch(c: &mut Canvas, x: i32, y: i32, on: bool) {
    if on {
        c.round_rect(x, y, 52, 32, 16, PRIMARY);
        c.round_rect(x + 24, y + 4, 24, 24, 12, ON_PRIMARY);
    } else {
        c.round_rect(x, y, 52, 32, 16, ON_SURFACE_VAR);
        c.round_rect(x + 2, y + 2, 48, 28, 14, SURFACE_HI);
        c.round_rect(x + 8, y + 8, 16, 16, 8, ON_SURFACE_VAR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resends_mode_until_chip_confirms() {
        use crate::meter::Reading;
        let saved = Settings { mode: 3, ..Settings::default() }; // resistance
        let mut a = App::new(saved);
        assert_eq!(a.take_command(), Some(Mode::Ohm.command()));
        // chip was still booting: it announces AUTO and shows its AUTO screen
        a.on_frame(Frame::Event(Mode::Auto as u8));
        let auto = Frame::Display(Reading { symbols: [10, 11, 12, 13], ..Reading::default() });
        for _ in 0..3 {
            assert!(!a.on_frame(auto));
        }
        assert_eq!(a.mode(), Mode::Ohm);
        assert_eq!(a.take_command(), Some(Mode::Ohm.command())); // resent
        a.on_frame(Frame::Event(Mode::Ohm as u8));
        assert!(a.on_frame(auto)); // confirmed: readings are shown again
    }

    #[test]
    fn keeps_last_value_while_chip_changes_range() {
        use crate::meter::Reading;
        let mut a = App::new(Settings { mode: 1, ..Settings::default() });
        a.on_frame(Frame::Event(a.mode() as u8));
        let v = Reading { symbols: [0, 9, 9, 7], decimal_after: 1, unit: 6, ..Reading::default() };
        assert!(a.on_frame(Frame::Display(v)));
        let blank = Reading { symbols: [17; 4], ..Reading::default() };
        assert!(!a.on_frame(Frame::Display(blank)));
        assert_eq!(a.reading, Some(v));
    }

    #[test]
    fn own_sounds() {
        use crate::meter::Reading;
        let mut a = App::new(Settings { own_ncv: true, own_continuity: true, ..Settings::default() });
        assert_eq!(a.sound(0), Sound::Free); // AUTO: the chip's own sounds
        a.set_mode(Mode::Ncv);
        a.on_frame(Frame::Event(Mode::Ncv as u8));
        assert_eq!(a.sound(0), Sound::Mask); // no field
        a.on_frame(Frame::Display(Reading { symbols: [16, 16, 16, 16], ..Reading::default() }));
        assert_eq!(a.sound(7), Sound::Tone { hz: 4000 }); // full scale: continuous
        a.set_mode(Mode::Diode);
        a.on_frame(Frame::Event(Mode::Diode as u8));
        a.on_frame(Frame::Display(Reading { symbols: [0, 0, 0, 3], decimal_after: 1, unit: 6, ..Reading::default() }));
        assert_eq!(a.sound(0), Sound::Tone { hz: 2700 }); // 0.003 V: shorted
        a.on_frame(Frame::Display(Reading { symbols: [17, 0, 14, 17], decimal_after: 2, unit: 6, ..Reading::default() }));
        assert_eq!(a.sound(0), Sound::Mask); // open: "0L"
        let b = App::new(Settings { remember_mode: false, mode: 3, ..Settings::default() });
        assert_eq!(b.mode(), Mode::Auto);
    }

    #[test]
    fn reset_needs_confirmation() {
        let mut a = App::new(Settings { brightness: 1, ..Settings::default() });
        a.key(Key::Back); // menu
        for _ in 0..11 {
            a.key(Key::Down); // "Сброс настроек"
        }
        a.key(Key::Ok);
        a.key(Key::Ok); // "Отмена" is preselected
        assert_eq!(a.s.brightness, 1);
        a.key(Key::Ok);
        a.key(Key::Right); // to "Да"
        a.key(Key::Ok);
        assert_eq!(a.s.brightness, Settings::default().brightness);
        assert!(a.confirm.is_none());
    }

    #[test]
    fn menu_labels_fit() {
        // every label must end at least 8 px before its control, in both languages and all values
        for english in [false, true] {
            for auto_off in 0..4 {
                for fahrenheit in [false, true] {
                    let a = App::new(Settings { english, auto_off, fahrenheit, ..Settings::default() });
                    for i in 0..ITEMS {
                        let (_, label, ctl) = a.item(i);
                        let end = LABEL_X + BODY.width(label);
                        let ctl_x = W - 28 - ctl.width();
                        assert!(end + 8 <= ctl_x, "{label:?}: label ends at {end}, control starts at {ctl_x}");
                    }
                }
            }
        }
    }

    #[test]
    fn games_flow() {
        let mut a = App::new(Settings::default());
        a.key(Key::Back); // menu
        for _ in 0..12 {
            a.key(Key::Down); // "Игры", the second to last row
        }
        a.key(Key::Ok);
        a.key(Key::Ok); // Snake: asks for the difficulty, medium preselected
        assert!(!a.in_game());
        a.key(Key::Down); // hard
        a.key(Key::Ok);
        assert!(a.in_game());
        assert_eq!(a.game.as_ref().map(|g| g.level()), Some(2));
        a.key(Key::Back); // leave the game
        a.key(Key::Ok);
        assert_eq!(a.level_sel, 2); // the last level is remembered
        a.key(Key::Back); // back to the list
        a.key(Key::Down);
        a.key(Key::Down); // 15 puzzle: no levels, starts at once
        a.key(Key::Ok);
        assert!(a.in_game());
        a.key(Key::Back);
        a.key(Key::Back);
        assert!(a.in_menu());
    }

    #[test]
    fn range_bar_is_symmetric_and_hidden_over_range() {
        // the widest full-scale label and the "0" fit above the bar with room between them
        let w = |t: &str| SMALL.ink(t).map_or(0, |(l, r, ..)| r - l);
        assert!(w("0") + 24 + w("10000 kHz") <= W - 2 * RANGE_X);
        // the bar stays above the card's rounded bottom (card ends at 184, corner radius 28)
        assert!(RANGE_Y + 4 <= 184 - 8);
        // the reading has as much air above (to the unit) as below (to the range labels)
        let (_, _, t, b) = HUGE.ink("0").unwrap();
        let label_top = RANGE_LABEL_Y + SMALL.ink("0").unwrap().2;
        let y = digits_y(true);
        assert!(((y + t - CHIPS_BOTTOM) - (label_top - (y + b))).abs() <= 1);
        assert!(y + b + 6 <= label_top, "the reading crowds the range labels");
        // no bar over range
        let ol = Reading { symbols: [17, 0, 14, 17], decimal_after: 2, unit: 13, ..Reading::default() };
        assert!(ol.is_overload());
        assert_eq!(ol.range(), None);
    }

    /// Draw `s` with `text_mid` at the centre of a small canvas and return its ink box centre.
    fn mid_of(f: &crate::font::Font, s: &str) -> (f32, f32) {
        let mut buf = [0u16; W as usize * 40];
        let mut c = Canvas { buf: &mut buf, y0: 0, rows: 40, clip: (0, W) };
        c.text_mid(f, 100, 20, s, 0xFFFF);
        let (mut l, mut r, mut t, mut b) = (W, 0, 40, 0);
        for y in 0..40 {
            for x in 0..W {
                if c.buf[(y * W + x) as usize] != 0 {
                    (l, r, t, b) = (l.min(x), r.max(x + 1), t.min(y), b.max(y + 1));
                }
            }
        }
        ((l + r) as f32 / 2.0, (t + b) as f32 / 2.0)
    }

    #[test]
    fn icons_and_battery_digits_are_centred() {
        for s in ["100", "52", "7"] {
            let (x, y) = mid_of(&BATT, s);
            assert!((x - 100.0).abs() <= 0.5 && (y - 20.0).abs() <= 0.5, "{s}: centred at ({x}, {y})");
        }
        for icon in [IC_NOTIFICATIONS_OFF, IC_PAUSE, IC_BOLT] {
            let (x, y) = mid_of(&ICON_FILL, icon);
            assert!((x - 100.0).abs() <= 0.5 && (y - 20.0).abs() <= 0.5, "icon centred at ({x}, {y})");
        }
    }

    #[test]
    fn big_words_use_the_large_font() {
        for s in ["Фаза!", "Нет фазы", "Live!", "No live wire", "AUTO"] {
            assert!(s.chars().all(|ch| ch == ' ' || LARGE.glyph(ch).is_some()), "{s}");
            assert!(LARGE.width(s) <= W - 48, "{s} is wider than the card");
        }
    }

    #[test]
    fn defaults() {
        let d = Settings::default();
        assert!(!d.off_beep && d.quiet_switch && !d.english);
    }

    #[test]
    fn settings_roundtrip() {
        let s = Settings { brightness: 2, auto_off: 3, english: true, volume: 1, fahrenheit: true, mode: 5, own_ncv: true, range_bar: false, ..Settings::default() };
        assert_eq!(Settings::from_bytes(&s.to_bytes()), Some(s));
        assert_eq!(Settings::from_bytes(&[0xFF; 8]), None); // erased flash
        assert_eq!(Settings::from_bytes(&[4, 2, 1, 1, 3, 0, 0, 1]), None); // v0.3 layout: reset to defaults
        // settings saved by v1.2.0 (bit 7 unused) keep the range bar on
        assert_eq!(Settings::from_bytes(&Settings::default().to_bytes()).map(|s| s.range_bar), Some(true));
        assert_eq!(Settings::default().to_bytes()[5] & 0x80, 0);
    }

    #[test]
    fn mode_keys_send_commands() {
        let mut a = App::new(Settings::default());
        assert_eq!(a.take_command(), Some(Mode::Auto.command()));
        let press = |a: &mut App, k| {
            a.key(k);
            a.on_frame(Frame::Event(a.mode() as u8)); // chip confirms
            a.take_command();
            a.mode()
        };
        // right: V -> Ω -> C -> diode -> V, forward only
        let seq: std::vec::Vec<Mode> = (0..5).map(|_| press(&mut a, Key::Right)).collect();
        assert_eq!(seq, [Mode::DcV, Mode::Ohm, Mode::Cap, Mode::Diode, Mode::DcV]);
        assert_eq!(a.slots().1, 4); // all four cells on screen
        assert_eq!(press(&mut a, Key::RightLong), Mode::AcV); // held: DC <-> AC
        assert_eq!(press(&mut a, Key::Right), Mode::Ohm);
        assert_eq!(a.slots().0[0], Mode::AcV); // the voltage cell remembers AC
        assert_eq!(press(&mut a, Key::Right), Mode::Cap);
        assert_eq!(press(&mut a, Key::Right), Mode::Diode);
        assert_eq!(press(&mut a, Key::Right), Mode::AcV);
        // left: mA <-> A, held: DC <-> AC
        assert_eq!(press(&mut a, Key::Left), Mode::DcMa);
        assert_eq!(press(&mut a, Key::Left), Mode::DcA);
        assert_eq!(press(&mut a, Key::LeftLong), Mode::AcA);
        assert_eq!(press(&mut a, Key::Left), Mode::AcMa);
        assert_eq!(press(&mut a, Key::Down), Mode::Temp);
        assert_eq!(press(&mut a, Key::Left), Mode::AcMa); // back to the last current range
        assert_eq!(press(&mut a, Key::Ok), Mode::Auto);
        a.key(Key::Back);
        assert!(a.in_menu());
        // the chip reports a mode change of its own
        a.key(Key::Back);
        a.on_frame(Frame::Event(Mode::Ohm as u8));
        assert_eq!(a.mode(), Mode::Ohm);
    }
}
