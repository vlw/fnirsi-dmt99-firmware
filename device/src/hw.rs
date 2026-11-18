//! Bare-register drivers for the DMT-99 (CH32V303RCT6). Pin map and sequences follow the stock
//! V2.4 firmware (cross-checked against its disassembly) and smhgzll's hardware notes.
#![allow(dead_code)]

#[inline(always)]
fn rd(a: u32) -> u32 {
    unsafe { (a as *const u32).read_volatile() }
}
#[inline(always)]
fn wr(a: u32, v: u32) {
    unsafe { (a as *mut u32).write_volatile(v) }
}
#[inline(always)]
fn modify(a: u32, clear: u32, set: u32) {
    wr(a, (rd(a) & !clear) | set)
}

const RCC: u32 = 0x4002_1000;
const RCC_CTLR: u32 = RCC;
const RCC_CFGR0: u32 = RCC + 0x04;
const RCC_INTR: u32 = RCC + 0x08;
const RCC_APB2PCENR: u32 = RCC + 0x18;
const RCC_APB1PCENR: u32 = RCC + 0x1C;

const GPIOA: u32 = 0x4001_0800;
const GPIOB: u32 = 0x4001_0C00;
const GPIOC: u32 = 0x4001_1000;
const INDR: u32 = 0x08;
const OUTDR: u32 = 0x0C;
const BSHR: u32 = 0x10;
const BCR: u32 = 0x14;

const TIM2: u32 = 0x4000_0000;
const TIM8: u32 = 0x4001_3400;
const ADC1: u32 = 0x4001_2400;
const STK: u32 = 0xE000_F000;
const PFIC_CFGR: u32 = 0xE000_E048;

// GPIO CNF/MODE nibbles
const OUT_PP: u32 = 0x3;
const AF_PP: u32 = 0xB;
const IN_PULL: u32 = 0x8;
const IN_FLOAT: u32 = 0x4;
const ANALOG: u32 = 0x0;

fn pin_mode(port: u32, pin: u32, mode: u32) {
    let reg = port + if pin < 8 { 0 } else { 4 };
    let sh = (pin % 8) * 4;
    modify(reg, 0xF << sh, mode << sh);
}

/// Busy-wait in milliseconds, the same SysTick sequence as the stock firmware (HCLK/8 = 18 MHz).
pub fn delay_ms(ms: u32) {
    if ms == 0 {
        return;
    }
    modify(STK + 0x04, 1, 0);
    wr(STK + 0x10, 18_000 * ms);
    wr(STK + 0x14, 0);
    modify(STK, 0, 0x10);
    modify(STK, 0, 0x21);
    while rd(STK + 0x04) & 1 == 0 {}
    modify(STK, 1, 0);
}

pub mod clock {
    use super::*;

    /// 8 MHz HSE x18 = 144 MHz, APB1 72 MHz; register for register as stock SystemInit.
    pub fn init() {
        modify(RCC_CTLR, 0, 1);
        wr(RCC_CFGR0, rd(RCC_CFGR0) & 0xF0FF_0000);
        wr(RCC_CTLR, rd(RCC_CTLR) & 0xFEF6_FFFF);
        wr(RCC_CTLR, rd(RCC_CTLR) & 0xFFFB_FFFF);
        wr(RCC_CFGR0, rd(RCC_CFGR0) & 0xFF80_FFFF);
        wr(RCC_INTR, 0x009F_0000);
        modify(RCC_CTLR, 0, 1 << 16); // HSEON
        let mut n = 0;
        while rd(RCC_CTLR) & (1 << 17) == 0 && n < 0x1000 {
            n += 1;
        }
        if rd(RCC_CTLR) & (1 << 17) != 0 {
            modify(RCC_CFGR0, 0, 0x400); // HPRE /1, PPRE2 /1, PPRE1 /2
            modify(RCC_CFGR0, 0x003F_0000, 0x003D_0000); // PLL from HSE, x18
        } else {
            modify(RCC_CTLR, 1 << 16, 0); // no crystal: HSI/2 x18 = 72 MHz, delays run 2x long
            modify(RCC_CFGR0, 0, 0x400);
            modify(RCC_CFGR0, 0x003F_0000, 0x003C_0000);
        }
        modify(RCC_CTLR, 0, 1 << 24); // PLLON
        while rd(RCC_CTLR) & (1 << 25) == 0 {}
        modify(RCC_CFGR0, 3, 2); // SW = PLL
        while rd(RCC_CFGR0) & 0xC != 0x8 {}
        // GPIOA/B/C, AFIO, ADC1, TIM8; TIM2
        modify(RCC_APB2PCENR, 0, 1 | 1 << 2 | 1 << 3 | 1 << 4 | 1 << 9 | 1 << 13);
        modify(RCC_APB1PCENR, 0, 1);
    }

    /// Restart through the bootloader (PFIC system reset).
    pub fn reset() -> ! {
        wr(PFIC_CFGR, 0xBEEF_0000 | 1 << 7);
        loop {}
    }
}

pub mod power {
    use super::*;

    /// PC1 high holds the power supply on.
    pub fn hold() {
        wr(GPIOC + BSHR, 1 << 1); // set the level before switching to output: no glitch low
        pin_mode(GPIOC, 1, OUT_PP);
    }

    pub fn off() -> ! {
        super::buzzer::off();
        super::lcd::fill(0);
        super::backlight::set(100);
        wr(GPIOC + BCR, 1 << 1);
        // Still running means USB power: wait for the power key, then restart.
        while rd(GPIOC + INDR) & (1 << 2) == 0 {}
        delay_ms(50);
        while rd(GPIOC + INDR) & (1 << 2) != 0 {}
        super::clock::reset()
    }
}

pub mod backlight {
    use super::*;

    /// TIM8 CH1 on PC6, 300 Hz PWM2; `compare` 0 = full brightness, 100 = off (stock values).
    pub fn set(compare: u32) {
        pin_mode(GPIOC, 6, AF_PP);
        wr(TIM8 + 0x28, 4799); // PSC
        wr(TIM8 + 0x2C, 100); // ATRLR
        modify(TIM8 + 0x18, 0xFF, 0x78); // CH1: PWM2, preload
        wr(TIM8 + 0x34, compare);
        modify(TIM8 + 0x20, 0, 1); // CC1E
        modify(TIM8 + 0x44, 0, 1 << 15); // MOE
        if rd(TIM8) & 1 == 0 {
            wr(TIM8, 1 << 7);
            wr(TIM8 + 0x14, 1); // UG: load PSC
            wr(TIM8, 1 << 7 | 1);
        }
    }
}

pub mod buzzer {
    use super::*;

    /// TIM2 CH2 on PA1, 3.75 kHz PWM2 (stock tone); `duty` out of 384 (191 = stock loudness).
    pub fn on(duty: u32) {
        tone(3750, duty);
    }

    /// Tone of `hz` (timer clock 1.44 MHz); `duty` out of 384 keeps loudness comparable across pitches.
    pub fn tone(hz: u32, duty: u32) {
        let period = 1_440_000 / hz.max(100);
        let on_ticks = (duty.min(192) * period / 384).max(1);
        wr(TIM2 + 0x28, 99);
        wr(TIM2 + 0x2C, period - 1);
        modify(TIM2 + 0x18, 0xFF00, 0x7800); // CH2: PWM2, preload
        wr(TIM2 + 0x38, period - on_ticks);
        modify(TIM2 + 0x20, 0, 1 << 4); // CC2E
        wr(TIM2, 1 << 7);
        wr(TIM2 + 0x14, 1);
        wr(TIM2, 1 << 7 | 1);
        pin_mode(GPIOA, 1, AF_PP);
    }

    pub fn off() {
        wr(TIM2, 0);
        wr(GPIOA + BCR, 1 << 1);
        pin_mode(GPIOA, 1, OUT_PP); // PWM2 with compare 0 would hold the line high
    }

    /// Hold the buzzer driver permanently on: the measurement chip, which shares the buzzer,
    /// then cannot make it oscillate, so its mode-change beep is silenced. Keep it short
    /// (the coil draws DC while held) and end it with `off()`.
    pub fn hold() {
        modify(RCC_APB2PCENR, 0, 1 << 2); // may run before clock::init
        wr(TIM2, 0);
        wr(GPIOA + BSHR, 1 << 1);
        pin_mode(GPIOA, 1, OUT_PP);
    }

}

pub mod battery {
    use super::*;

    pub fn init() {
        pin_mode(GPIOC, 0, ANALOG);
        pin_mode(GPIOA, 8, IN_FLOAT);
        modify(RCC_CFGR0, 0, 3 << 14); // ADC clock PCLK2/8
        modify(ADC1 + 0x0C, 7, 7); // ch10 sample time 239.5 cycles
        wr(ADC1 + 0x2C, 0); // one conversion
        wr(ADC1 + 0x34, 10); // ch10 first
        wr(ADC1 + 0x08, 1); // ADON
        delay_ms(1);
        modify(ADC1 + 0x08, 0, 1 << 3); // RSTCAL
        while rd(ADC1 + 0x08) & (1 << 3) != 0 {}
        modify(ADC1 + 0x08, 0, 1 << 2); // CAL
        while rd(ADC1 + 0x08) & (1 << 2) != 0 {}
        modify(ADC1 + 0x08, 0, 7 << 17 | 1 << 20); // SWSTART trigger
    }

    fn sample() -> u32 {
        modify(ADC1 + 0x08, 0, 1 << 22);
        let mut n = 0;
        while rd(ADC1) & 2 == 0 {
            n += 1;
            if n > 100_000 {
                return 0;
            }
        }
        rd(ADC1 + 0x4C) & 0xFFF
    }

    /// Stock conversion: mV = raw * 6600 / 4096 (3.3 V reference, 1:2 divider).
    pub fn millivolts() -> u32 {
        let raw = (0..16).map(|_| sample()).sum::<u32>() / 16;
        raw * 6600 >> 12
    }

    pub fn charging() -> bool {
        rd(GPIOA + INDR) & (1 << 8) != 0
    }
}

pub mod keys {
    use super::*;
    use ui::app::Key;

    pub enum Event {
        Key(Key),
        Power,
    }

    // Stock event numbers: PC10=1 (down), PC11=2 (up), PC9=3 (right), PC12=4 (left), PC8 short=5 / long=6, PC2=power.
    /// Auto-repeating keys.
    const REPEAT: [(u32, Key); 1] = [(10, Key::Down)];
    /// Keys with a short press (on release) and a long press (at the threshold).
    const DUAL: [(u32, Key, Key); 4] = [
        (11, Key::Up, Key::UpLong),
        (9, Key::Right, Key::RightLong),
        (12, Key::Left, Key::LeftLong),
        (8, Key::Ok, Key::Back),
    ];
    const POWER_PIN: u32 = 2;
    const LONG: u16 = 60; // x10 ms
    const REPEAT_AFTER: u16 = 45;
    const REPEAT_EVERY: u16 = 12;

    static mut HELD: [u16; 6] = [0; 6]; // repeat keys, then dual keys, then power
    static mut POWER_ARMED: bool = false;

    pub fn init() {
        for pin in [2, 8, 9, 10, 11, 12] {
            wr(GPIOC + BSHR, 1 << pin); // pull-up
            pin_mode(GPIOC, pin, IN_PULL);
        }
    }

    /// Call every 10 ms. Arrows auto-repeat; OK is a short press, holding OK is Back.
    pub fn poll() -> Option<Event> {
        // SAFETY: called only from the main loop.
        let held = unsafe { &mut *core::ptr::addr_of_mut!(HELD) };
        let armed = unsafe { &mut *core::ptr::addr_of_mut!(POWER_ARMED) };
        let low = !rd(GPIOC + INDR);
        let mut ev = None;
        for (i, &(pin, key)) in REPEAT.iter().enumerate() {
            if low & (1 << pin) != 0 {
                held[i] = held[i].saturating_add(1);
                let h = held[i];
                if h == 2 || (h > REPEAT_AFTER && (h - REPEAT_AFTER) % REPEAT_EVERY == 0) {
                    ev = Some(Event::Key(key));
                }
            } else {
                held[i] = 0;
            }
        }
        for (i, &(pin, short, long)) in DUAL.iter().enumerate() {
            let h = &mut held[REPEAT.len() + i];
            if low & (1 << pin) != 0 {
                *h = h.saturating_add(1);
                if *h == LONG {
                    ev = Some(Event::Key(long));
                }
            } else {
                if (2..LONG).contains(h) {
                    ev = Some(Event::Key(short));
                }
                *h = 0;
            }
        }
        let h = &mut held[REPEAT.len() + DUAL.len()];
        if low & (1 << POWER_PIN) != 0 {
            *h = h.saturating_add(1);
            if *armed && *h == 30 {
                ev = Some(Event::Power);
            }
        } else {
            *h = 0;
            *armed = true; // the key that switched the meter on has been released
        }
        ev
    }
}

pub mod meter_uart {
    //! USART1 (PA9 TX, PA10 RX) at 9600 8N1 to the measurement chip; RX lands in a DMA ring
    //! (DMA1 channel 5, circular), so the main loop can poll without interrupts.
    use super::*;

    const USART1: u32 = 0x4001_3800;
    const DMA1_CH5: u32 = 0x4002_0058; // CFGR, CNTR +4, PADDR +8, MADDR +0xC
    const RING: usize = 128;
    static mut RX: [u8; RING] = [0; RING];
    static mut TAIL: usize = 0;

    pub fn init() {
        modify(RCC + 0x14, 0, 1); // DMA1 clock
        modify(RCC_APB2PCENR, 0, 1 << 14 | 1 << 2); // USART1, GPIOA
        pin_mode(GPIOA, 9, AF_PP);
        pin_mode(GPIOA, 10, IN_FLOAT);
        wr(USART1 + 0x08, 15_000); // 144 MHz / 9600
        wr(USART1 + 0x14, 1 << 6); // DMAR
        wr(DMA1_CH5, 0);
        wr(DMA1_CH5 + 0x04, RING as u32);
        wr(DMA1_CH5 + 0x08, USART1 + 0x04);
        wr(DMA1_CH5 + 0x0C, core::ptr::addr_of!(RX) as u32);
        wr(DMA1_CH5, 2 << 12 | 1 << 7 | 1 << 5 | 1); // high priority, MINC, CIRC, EN
        wr(USART1 + 0x0C, 1 << 13 | 1 << 3 | 1 << 2); // UE, TE, RE
    }

    pub fn send(bytes: &[u8]) {
        for &b in bytes {
            while rd(USART1) & (1 << 7) == 0 {}
            wr(USART1 + 0x04, b as u32);
        }
        while rd(USART1) & (1 << 6) == 0 {}
    }

    /// Bytes received since the last call.
    pub fn drain(mut f: impl FnMut(u8)) {
        // SAFETY: single-threaded; the DMA only writes RX, we only read behind its write index.
        let (rx, tail) = unsafe { (&*core::ptr::addr_of!(RX), &mut *core::ptr::addr_of_mut!(TAIL)) };
        let head = (RING - rd(DMA1_CH5 + 0x04) as usize) % RING;
        while *tail != head {
            f(unsafe { core::ptr::read_volatile(&rx[*tail]) });
            *tail = (*tail + 1) % RING;
        }
    }
}

pub mod settings_flash {
    //! Settings live in the stock settings page at 0x08040000 (256 bytes, fast erase), written
    //! with the same sequence as the stock firmware. Halfword 0 is kept non-zero so the stock
    //! firmware, if flashed back, treats the page as uninitialized and resets to its defaults.
    use super::*;

    const PAGE: u32 = 0x0804_0000;
    const FLASH: u32 = 0x4002_2000;
    const KEYR: u32 = FLASH + 0x04;
    const STATR: u32 = FLASH + 0x0C;
    const CTLR: u32 = FLASH + 0x10;
    const ADDR: u32 = FLASH + 0x14;
    const MODEKEYR: u32 = FLASH + 0x24;
    const MAGIC: u16 = 0xD99C;

    fn hw(i: u32) -> u16 {
        unsafe { ((PAGE + 2 * i) as *const u16).read_volatile() }
    }

    fn image(data: &[u8; 8]) -> [u16; 7] {
        let mut h = [1, MAGIC, 0, 0, 0, 0, 0];
        for i in 0..4 {
            h[2 + i] = data[2 * i] as u16 | (data[2 * i + 1] as u16) << 8;
        }
        h[6] = h[1..6].iter().fold(0x5A5A, |a, &x| a ^ x);
        h
    }

    pub fn load() -> Option<[u8; 8]> {
        let mut d = [0u8; 8];
        for i in 0..4 {
            let v = hw(2 + i as u32);
            d[2 * i] = v as u8;
            d[2 * i + 1] = (v >> 8) as u8;
        }
        let want = image(&d);
        (0..7).all(|i| hw(i as u32) == want[i]).then_some(d)
    }

    fn wait() {
        while rd(STATR) & 1 != 0 {}
    }

    pub fn save(data: &[u8; 8]) {
        let img = image(data);
        if (0..7).all(|i| hw(i as u32) == img[i]) {
            return; // unchanged: spare the flash
        }
        let cfgr = rd(RCC_CFGR0);
        modify(RCC_CFGR0, 0, 0x80); // HCLK = SYSCLK/2: required for flash writes above 120 MHz
        wr(KEYR, 0x4567_0123);
        wr(KEYR, 0xCDEF_89AB);
        wr(STATR, 0x31);
        wr(MODEKEYR, 0x4567_0123);
        wr(MODEKEYR, 0xCDEF_89AB);
        modify(CTLR, 0, 1 << 17); // fast page erase (256 B)
        wr(ADDR, PAGE);
        modify(CTLR, 0, 0x40);
        wait();
        modify(CTLR, 1 << 17, 0);
        modify(CTLR, 0, 1 << 15); // fast lock
        for i in 0..128u32 {
            let v = if (i as usize) < img.len() { img[i as usize] } else { 0 };
            wait();
            modify(CTLR, 0, 1); // PG
            unsafe { ((PAGE + 2 * i) as *mut u16).write_volatile(v) };
            wait();
            modify(CTLR, 1, 0);
        }
        modify(CTLR, 0, 0x80); // lock
        wr(RCC_CFGR0, (rd(RCC_CFGR0) & !0xF0) | (cfgr & 0xF0));
    }
}

pub mod lcd {
    //! ST7789-family panel on a 16-bit 8080 bus: PB0..15 data, PC3 RESET, PC4 CS, PC5 D/C, PC13 WR.
    use super::*;

    const RST: u32 = 1 << 3;
    const CS: u32 = 1 << 4;
    const DC: u32 = 1 << 5;
    const RD: u32 = 1 << 7;
    const WR_: u32 = 1 << 13;

    // cmd, n, data... (stock V2.4 init table)
    const INIT: &[u8] = &[
        0x36, 1, 0x00, 0x3A, 1, 0x05, 0xB2, 5, 0x0C, 0x0C, 0x00, 0x33, 0x33, 0xB7, 1, 0x35, 0xBB, 1, 0x28,
        0xC0, 1, 0x2C, 0xC2, 1, 0x01, 0xC3, 1, 0x0B, 0xC4, 1, 0x20, 0xC6, 1, 0x0F, 0xD0, 2, 0xA4, 0xA1,
        0xE0, 14, 0xD0, 0x01, 0x08, 0x0F, 0x11, 0x2A, 0x36, 0x55, 0x44, 0x3A, 0x0B, 0x06, 0x11, 0x20,
        0xE1, 14, 0xD0, 0x02, 0x07, 0x0A, 0x0B, 0x18, 0x34, 0x43, 0x4A, 0x2B, 0x1B, 0x1C, 0x22, 0x1F,
    ];

    #[inline(always)]
    fn strobe() {
        wr(GPIOC + BCR, WR_);
        unsafe { core::arch::asm!("nop", "nop", "nop", "nop") };
        wr(GPIOC + BSHR, WR_);
        unsafe { core::arch::asm!("nop", "nop") };
    }

    fn word(v: u16, data: bool) {
        wr(GPIOC + if data { BSHR } else { BCR }, DC);
        wr(GPIOC + BCR, CS);
        wr(GPIOB + OUTDR, v as u32);
        strobe();
        wr(GPIOC + BSHR, CS);
    }

    fn cmd(c: u8, data: &[u8]) {
        word(c as u16, false);
        for &d in data {
            word(d as u16, true);
        }
    }

    fn window(x0: u16, y0: u16, x1: u16, y1: u16) {
        cmd(0x2A, &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8]);
        cmd(0x2B, &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8]);
        word(0x2C, false);
    }

    fn stream(px: impl Iterator<Item = u16>) {
        wr(GPIOC + BSHR, DC);
        wr(GPIOC + BCR, CS);
        for p in px {
            wr(GPIOB + OUTDR, p as u32);
            strobe();
        }
        wr(GPIOC + BSHR, CS);
    }

    pub fn init() {
        wr(GPIOC + BSHR, RST | CS | DC | RD | WR_);
        for pin in [3, 4, 5, 7, 13] {
            pin_mode(GPIOC, pin, OUT_PP);
        }
        for pin in 0..16 {
            pin_mode(GPIOB, pin, OUT_PP);
        }
        delay_ms(10);
        wr(GPIOC + BCR, RST);
        delay_ms(20);
        wr(GPIOC + BSHR, RST);
        delay_ms(20);
        cmd(0x11, &[]); // sleep out
        delay_ms(120);
        let mut i = 0;
        while i < INIT.len() {
            let n = INIT[i + 1] as usize;
            cmd(INIT[i], &INIT[i + 2..i + 2 + n]);
            i += 2 + n;
        }
        cmd(0x29, &[]); // display on
        cmd(0x36, &[0x60]); // landscape 320x240, as stock
        fill(0);
    }

    pub fn fill(color: u16) {
        window(0, 0, 319, 239);
        stream(core::iter::repeat(color).take(320 * 240));
    }

    /// Send `rows` full-width rows starting at `y0`.
    pub fn push_rows(px: &[u16], y0: i32, rows: i32) {
        window(0, y0 as u16, 319, (y0 + rows - 1) as u16);
        stream(px.iter().copied());
    }
}
