//! PC simulator: runs the same `ui` code as the firmware in an SDL2 window (raw FFI, system libSDL2).
//! A fake measurement chip produces real protocol bytes, decoded by the firmware's parser.
//!
//!   cargo run --release               window; arrows, Enter = OK, Backspace = long OK (menu/back),
//!                                     Shift+Up/Left/Right = hold (REL, DC/AC),
//!                                     +/- battery voltage, C charging, Q quit
//!   cargo run -- --shot out.ppm ok,down,right   headless: press keys (waitN: N ticks), save the frame
use std::ffi::{c_char, c_int, c_void};
use ui::app::{App, Inputs, Key, Settings};
use ui::meter::{Mode, Stream};
use ui::{H, W};

#[link(name = "SDL2")]
extern "C" {
    fn SDL_Init(flags: u32) -> c_int;
    fn SDL_CreateWindow(t: *const c_char, x: c_int, y: c_int, w: c_int, h: c_int, f: u32) -> *mut c_void;
    fn SDL_CreateRenderer(w: *mut c_void, i: c_int, f: u32) -> *mut c_void;
    fn SDL_CreateTexture(r: *mut c_void, fmt: u32, access: c_int, w: c_int, h: c_int) -> *mut c_void;
    fn SDL_UpdateTexture(t: *mut c_void, rect: *const c_void, px: *const c_void, pitch: c_int) -> c_int;
    fn SDL_RenderCopy(r: *mut c_void, t: *mut c_void, s: *const c_void, d: *const c_void) -> c_int;
    fn SDL_RenderPresent(r: *mut c_void);
    fn SDL_PollEvent(e: *mut [u8; 64]) -> c_int;
    fn SDL_Delay(ms: u32);
    fn SDL_Quit();
}

const PIXELFORMAT_RGB565: u32 = 0x1515_1002;

/// LCD segment patterns of the chip, inverse of `meter::decode`.
fn pattern(c: u8) -> u8 {
    const P: [u8; 20] = [
        0xEB, 0x0A, 0xAD, 0x8F, 0x4E, 0xC7, 0xE7, 0x8A, 0xEF, 0xCF, 0xEE, 0x23, 0x65, 0x27, 0x61, 0xE5, 0x04, 0x00, 0xE1,
        0xEC,
    ];
    let i = match c {
        b'0'..=b'9' => c - b'0',
        b'A' => 10,
        b'U' => 11,
        b'T' => 12,
        b'O' => 13,
        b'L' => 14,
        b'E' => 15,
        b'-' => 16,
        _ => 17,
    };
    P[i as usize]
}

/// Build a 12-byte display frame: 4 LCD chars, decimal point after `dp` digits, unit flag bits.
fn frame(chars: &[u8; 4], dp: u8, neg: bool, acdc: u8, unit: (u8, u8, u8)) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0] = 0x5A;
    b[1] = 0xA5;
    for (i, &ch) in chars.iter().enumerate() {
        let p = pattern(ch);
        b[2 + i] |= p & 0xE0;
        b[3 + i] |= p & 0x0F;
    }
    if neg {
        b[2] |= 0x10;
    }
    if (1..=3).contains(&dp) {
        b[2 + dp as usize] |= 0x10;
    }
    b[2] |= acdc;
    b[6] |= unit.0;
    b[7] |= unit.1;
    b[8] |= unit.2;
    b
}

/// What a chip in `mode` might be showing at time `t` (10 ms ticks).
fn fake_chip(mode: Mode, t: u32) -> [u8; 12] {
    let j = (t / 30 % 7) as u8; // slow jitter in the last digit
    let d = |base: u8| b'0' + (base + j) % 10;
    match mode {
        Mode::Auto => frame(b"AUTO", 0, false, 0, (0, 0, 0)),
        Mode::DcV => frame(&[b'1', b'2', b'3', d(4)], 2, false, 2, (0, 0, 2)),
        Mode::AcV => frame(&[b'2', b'3', b'0', d(1)], 3, false, 1, (0, 0, 2)),
        Mode::Ohm => frame(&[b' ', b'9', b'9', d(7)], 2, false, 0, (0x40, 0x20, 0)),
        Mode::Cap => frame(&[b' ', b'4', b'7', d(0)], 3, false, 0, (0x10, 0x10, 0)),
        Mode::Freq => frame(&[b'5', b'0', b'0', d(0)], 2, false, 0, (0, 0, 4)),
        Mode::Temp => frame(&[b' ', b' ', b'2', d(4)], 0, false, 0, (0, 0, 0x20)),
        Mode::DcMa => frame(&[b'1', b'2', b'5', d(0)], 2, false, 2, (0, 1, 1)),
        Mode::AcMa => frame(&[b' ', b'0', b'3', d(1)], 2, false, 1, (0, 1, 1)),
        Mode::DcA => frame(&[b'1', b'2', b'3', d(4)], 1, true, 2, (0, 0, 1)),
        Mode::AcA => frame(b" 0L ", 2, false, 1, (0, 0, 1)),
        Mode::Ncv => {
            let n = (t / 60 % 5) as usize;
            let mut c = *b"    ";
            c[..n].fill(b'-');
            frame(&c, 0, false, 0, (0, 0, 0))
        }
        Mode::Live => frame(b"L1UE", 0, false, 0, (0, 0, 0)),
        Mode::Diode => frame(b" 0L ", 1, false, 0, (0, 0, 2)),
    }
}

/// Render through the same band path as the device, into a full-screen buffer.
fn render(app: &App, inp: &Inputs, fb: &mut [u16]) {
    let mut band = vec![0u16; (W * 24) as usize];
    ui::render_frame(&mut band, |c| app.draw(c, inp), |px, y0, rows| {
        fb[(y0 * W) as usize..((y0 + rows) * W) as usize].copy_from_slice(px);
    });
}

/// Screen as the user sees it: backlight levels 1..=5 dim the image.
fn backlit(fb: &[u16], level: u8) -> Vec<u16> {
    let k = 40 + level as u32 * 43; // 83..255
    fb.iter().map(|&p| ui::blend(0, p, k)).collect()
}

fn save_ppm(path: &str, fb: &[u16]) {
    let mut out = format!("P6\n{W} {H}\n255\n").into_bytes();
    for &p in fb {
        let (r, g, b) = ((p >> 11) & 31, (p >> 5) & 63, p & 31);
        out.extend([(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]);
    }
    std::fs::write(path, out).expect("write screenshot");
}

/// Fake chip: answers a mode command with an event, then streams display frames.
struct Chip {
    mode: Mode,
    stream: Stream,
}

impl Chip {
    fn command(&mut self, cmd: [u8; 10], app: &mut App) {
        if let Some(m) = Mode::from_id(cmd[3]) {
            self.mode = m;
            for b in [0xAA, 0x55, 0xFD, cmd[3], 0, 0, 0, 0xAA, 0x01, 0xBB] {
                if let Some(f) = self.stream.feed(b) {
                    app.on_frame(f);
                }
            }
        }
    }
    fn tick(&mut self, t: u32, app: &mut App) -> bool {
        let mut dirty = false;
        for b in fake_chip(self.mode, t) {
            if let Some(f) = self.stream.feed(b) {
                dirty |= app.on_frame(f);
            }
        }
        dirty
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut app = App::new(Settings::default());
    let mut chip = Chip { mode: Mode::Auto, stream: Stream::default() };
    let mut inp = Inputs { battery_mv: 3712, charging: false };
    let mut fb = vec![0u16; (W * H) as usize];

    if args.get(1).map(String::as_str) == Some("--shot") {
        let path = args.get(2).expect("--shot PATH [keys]");
        let keys = args.get(3).map(|s| s.split(',').filter(|s| !s.is_empty()).collect()).unwrap_or(Vec::<&str>::new());
        let mut now: u32 = 1000; // game clock in 10 ms ticks
        let mut muted = false;
        for (t, k) in keys.into_iter().enumerate() {
            if let Some(n) = k.strip_prefix("wait") {
                // "wait150": let 1.5 s of game time pass
                for _ in 0..n.parse::<u32>().expect("waitN") {
                    now += 1;
                    app.tick(now);
                }
                continue;
            }
            match k {
                "charge" => inp.charging = true,
                "low" => inp.battery_mv = 3480,
                "en" => app.s.english = true,
                "mute" => muted = true, // the fake chip stops answering: shows the waiting state
                k => {
                    let key = match k {
                        "up" => Key::Up,
                        "uplong" => Key::UpLong,
                        "leftlong" => Key::LeftLong,
                        "rightlong" => Key::RightLong,
                        "down" => Key::Down,
                        "left" => Key::Left,
                        "right" => Key::Right,
                        "ok" => Key::Ok,
                        "back" => Key::Back,
                        _ => panic!("unknown key {k}"),
                    };
                    app.tick(now);
                    app.key(key);
                }
            }
            if muted {
                app.take_command();
                continue;
            }
            if let Some(c) = app.take_command() {
                chip.command(c, &mut app);
            }
            chip.tick(t as u32 * 100, &mut app);
        }
        if !muted {
            if let Some(c) = app.take_command() {
                chip.command(c, &mut app);
            }
            chip.tick(500, &mut app);
        }
        let frames: u32 = std::env::var("ANIM_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(99);
        for _ in 0..frames {
            app.step_anim();
        }
        render(&app, &inp, &mut fb);
        save_ppm(path, &backlit(&fb, app.s.brightness));
        return;
    }

    unsafe {
        assert_eq!(SDL_Init(0x20), 0, "SDL_Init failed");
        let win = SDL_CreateWindow(c"DMT-99 simulator".as_ptr(), 0x2FFF_0000, 0x2FFF_0000, W * 3, H * 3, 0);
        let ren = SDL_CreateRenderer(win, -1, 2 | 4);
        let tex = SDL_CreateTexture(ren, PIXELFORMAT_RGB565, 1, W, H);
        assert!(!win.is_null() && !ren.is_null() && !tex.is_null(), "SDL setup failed");
        let mut dirty = true;
        let mut ev = [0u8; 64];
        let mut t: u32 = 0;
        let mut last_sound = ui::app::Sound::Free;
        'main: loop {
            while SDL_PollEvent(&mut ev) != 0 {
                let ty = u32::from_ne_bytes(ev[0..4].try_into().unwrap());
                if ty == 0x100 {
                    break 'main;
                }
                if ty != 0x300 {
                    continue;
                }
                let sym = i32::from_ne_bytes(ev[20..24].try_into().unwrap());
                let shift = u16::from_ne_bytes(ev[24..26].try_into().unwrap()) & 3 != 0; // Shift = hold the key
                let key = match sym {
                    0x4000_0052 if shift => Some(Key::UpLong),
                    0x4000_0052 => Some(Key::Up),
                    0x4000_0051 => Some(Key::Down),
                    0x4000_0050 if shift => Some(Key::LeftLong),
                    0x4000_004F if shift => Some(Key::RightLong),
                    0x4000_0050 => Some(Key::Left),
                    0x4000_004F => Some(Key::Right),
                    13 | 32 => Some(Key::Ok),
                    8 | 27 => Some(Key::Back),
                    _ => None,
                };
                match sym {
                    113 => break 'main,
                    61 | 43 => inp.battery_mv = (inp.battery_mv + 50).min(4200),
                    45 => inp.battery_mv = inp.battery_mv.saturating_sub(50).max(3300),
                    99 => inp.charging = !inp.charging,
                    _ => {}
                }
                if let Some(k) = key {
                    app.key(k);
                }
                dirty = true;
            }
            if let Some(c) = app.take_command() {
                println!("tx {:02X?}", c);
                chip.command(c, &mut app);
                dirty = true;
            }
            if t % 30 == 0 {
                dirty |= chip.tick(t, &mut app);
            }
            dirty |= app.tick(t);
            let snd = app.sound(t);
            if snd != last_sound {
                println!("buzzer: {snd:?}");
                last_sound = snd;
            }
            if !dirty && app.animating() {
                app.step_anim(); // sim redraws the whole frame; the device only the carousel rows
                dirty = true;
            }
            if dirty {
                render(&app, &inp, &mut fb);
                let shown = backlit(&fb, app.s.brightness);
                SDL_UpdateTexture(tex, std::ptr::null(), shown.as_ptr().cast(), W * 2);
                SDL_RenderCopy(ren, tex, std::ptr::null(), std::ptr::null());
                SDL_RenderPresent(ren);
                dirty = false;
            }
            SDL_Delay(10);
            t += 1;
        }
        SDL_Quit();
    }
}
