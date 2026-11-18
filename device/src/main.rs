//! DMT-99 custom firmware: boots from the stock bootloader, drives the LCD, keys, backlight,
//! buzzer, battery ADC and power latch, and runs the shared `ui` app. No measurements yet.
#![no_std]
#![no_main]

mod hw;

use hw::*;
use ui::app::{App, Inputs, Settings, Sound};
use ui::meter::Stream;

core::arch::global_asm!(
    r#"
    .section .text.header, "ax"
    .option push
    .option norvc
    .globl _start
_start:
    j reset
    .rept 12
    .word 0x00000013
    .endr
    .word 0x00100073
    .word _start            # vector table slot 0 sits at 0x6438, as in the stock image
reset:
    .option norelax         # keeps the .balign below honest (relaxation would shrink code after it)
    la gp, __global_pointer$
    la sp, _stack_top
    csrci mstatus, 8        # the bootloader may leave interrupts set up: mask and clear them
    li t0, -1
    li t1, 0xE000E180
    sw t0, 0(t1)
    sw t0, 4(t1)
    sw t0, 8(t1)
    sw t0, 12(t1)
    li t1, 0xE000E280
    sw t0, 0(t1)
    sw t0, 4(t1)
    sw t0, 8(t1)
    sw t0, 12(t1)
    li t1, 0xE000F000
    sw zero, 0(t1)
    csrw 0x804, zero        # INTSYSCR, as stock
    li t0, 0x1f
    csrw 0xbc0, t0          # CORECFGR, as stock
    la t0, trap
    csrw mtvec, t0
    la a0, _sidata
    la a1, _sdata
    la a2, _edata
1:  bgeu a1, a2, 2f
    lw t0, 0(a0)
    sw t0, 0(a1)
    addi a0, a0, 4
    addi a1, a1, 4
    j 1b
2:  la a0, _sbss
    la a1, _ebss
3:  bgeu a0, a1, 4f
    sw zero, 0(a0)
    addi a0, a0, 4
    j 3b
4:  call main
    j power_off_and_halt
    .balign 4
trap:
    j power_off_and_halt
    .option pop
"#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    power_off_and_halt()
}

const BAND_ROWS: usize = 24;
static mut BAND: [u16; ui::W as usize * BAND_ROWS] = [0; ui::W as usize * BAND_ROWS];

/// Backlight PWM compare per brightness level 1..=5 (PWM2: lower compare = brighter, 100 = off).
const BACKLIGHT: [u32; 5] = [88, 72, 52, 28, 0];

/// After a mode command the buzzer is held until the chip confirms the mode, then this much longer
/// (10 ms ticks) to cover the end of its beep.
const MASK_TAIL_TICKS: u32 = 25;
/// Upper bound of one masking stretch, should the chip never answer.
const MASK_MAX_TICKS: u32 = 300;

/// Drive the buzzer to `want` if it is not already there.
fn apply_sound(cur: &mut Sound, want: Sound, duty: u32) {
    if *cur == want {
        return;
    }
    match want {
        Sound::Free => buzzer::off(),
        Sound::Mask => buzzer::hold(),
        Sound::Tone { hz } => buzzer::tone(hz, duty),
    }
    *cur = want;
}

fn redraw(app: &App, inp: &Inputs) {
    // SAFETY: single-threaded, no interrupts; BAND is only used here.
    let band = unsafe { &mut *core::ptr::addr_of_mut!(BAND) };
    ui::render_frame(band, |c| app.draw(c, inp), lcd::push_rows);
}

fn redraw_rows(app: &App, inp: &Inputs, rows: (i32, i32)) {
    // SAFETY: as in `redraw`.
    let band = unsafe { &mut *core::ptr::addr_of_mut!(BAND) };
    ui::render_rows(band, rows, |c| app.draw(c, inp), lcd::push_rows);
}

fn read_inputs() -> Inputs {
    Inputs { battery_mv: battery::millivolts(), charging: battery::charging() }
}

fn apply_backlight(s: &Settings) {
    backlight::set(BACKLIGHT[s.brightness as usize - 1]);
}

/// Save settings, play the power-off beep if enabled, cut power.
fn shutdown(app: &App) -> ! {
    settings_flash::save(&app.s.to_bytes());
    if app.s.off_beep {
        buzzer::on(app.s.beep_duty());
        delay_ms(500);
        buzzer::off();
    }
    power::off()
}

#[no_mangle]
extern "C" fn main() -> ! {
    let settings = settings_flash::load().and_then(|b| Settings::from_bytes(&b)).unwrap_or_default();
    if settings.quiet_switch {
        buzzer::hold(); // the measurement chip beeps as it powers up: mask it before anything else
    }
    clock::init();
    power::hold(); // keep the power latch on before anything slow
    keys::init();
    lcd::init();
    battery::init();
    meter_uart::init();
    timer::init();
    if !settings.quiet_switch {
        buzzer::off(); // PA1 floats after reset: hold the buzzer driver low
    }

    let mut app = App::new(settings);
    let mut inp = read_inputs();
    redraw(&app, &inp);
    apply_backlight(&app.s);

    let mut stream = Stream::default();
    let mut tick: u32 = 0;
    let mut idle: u32 = 0;
    let mut was_menu = false;
    // chip-beep masking: runs from power-on until the first mode is confirmed, then after each command
    let mut sound = if settings.quiet_switch { Sound::Mask } else { Sound::Free };
    let mut tail: u32 = if settings.quiet_switch { MASK_TAIL_TICKS } else { 0 };
    let mut masked_for: u32 = 0;
    loop {
        delay_ms(10);
        tick = tick.wrapping_add(1);
        idle = idle.saturating_add(1);
        let now = timer::now();
        let mut dirty = app.tick(now);
        keys::set_instant(app.in_game());
        match keys::poll() {
            Some(keys::Event::Power) => shutdown(&app),
            Some(keys::Event::Key(k)) => {
                idle = 0;
                let before = app.s.brightness;
                dirty |= app.key(k);
                if app.s.brightness != before {
                    apply_backlight(&app.s);
                }
            }
            None => {}
        }
        if was_menu && !app.in_menu() {
            settings_flash::save(&app.s.to_bytes()); // leaving the menu commits the settings
        }
        was_menu = app.in_menu();
        if let Some(cmd) = app.take_command() {
            if app.s.quiet_switch {
                apply_sound(&mut sound, Sound::Mask, 0); // the chip beeps on every mode command
                tail = MASK_TAIL_TICKS;
                masked_for = 0;
            }
            meter_uart::send(&cmd);
        }
        meter_uart::drain(|b| {
            if let Some(f) = stream.feed(b) {
                dirty |= app.on_frame(f);
            }
        });
        // keep masking while the chip has not confirmed the mode, then for the tail
        if tail > 0 {
            masked_for += 1;
            if !app.awaiting_ack() || masked_for >= MASK_MAX_TICKS {
                tail -= 1;
            }
        }
        let own = app.sound(now);
        let want = if tail > 0 && !matches!(own, Sound::Tone { .. }) { Sound::Mask } else { own };
        apply_sound(&mut sound, want, app.s.beep_duty());
        if tick % 200 == 0 {
            let now = read_inputs();
            if now.charging != inp.charging || now.battery_mv / 10 != inp.battery_mv / 10 {
                inp = now;
                dirty = true;
            }
        }
        let off_min = app.s.auto_off_minutes();
        if off_min != 0 && idle >= off_min * 60 * 100 {
            shutdown(&app);
        }
        if dirty {
            redraw(&app, &inp);
        } else if app.animating() {
            app.step_anim();
            redraw_rows(&app, &inp, ui::app::CAROUSEL_ROWS);
        }
    }
}

#[no_mangle]
extern "C" fn power_off_and_halt() -> ! {
    power::off()
}
