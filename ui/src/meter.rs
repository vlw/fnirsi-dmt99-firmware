//! Measurement chip protocol (USART1, 9600 8N1). The chip does the measuring, ranging and
//! calibration; it sends what its 4-digit LCD would show. Ported from smhgzll's reconstruction
//! of the stock firmware (MIT): display frames `5A A5` + 10 bytes, events `AA 55 FD id ...` (10 bytes).

/// Measurement modes; the value is the chip command / event id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Mode {
    Auto = 0x14,
    DcV = 0x0C,
    AcV = 0x0D,
    Ohm = 0x0B,
    Cap = 0x0A,
    Freq = 0x0F,
    Temp = 0x12,
    DcMa = 0x11,
    AcMa = 0x16,
    DcA = 0x10,
    AcA = 0x15,
    Ncv = 0x09,
    Live = 0x13,
    /// continuity / diode; stock UI shows it with mode/ol.jpg, between resistance and capacitance
    Diode = 0x0E,
}

pub const MODES: [Mode; 14] = [
    Mode::Auto,
    Mode::DcV,
    Mode::AcV,
    Mode::Ohm,
    Mode::Cap,
    Mode::Freq,
    Mode::Temp,
    Mode::DcMa,
    Mode::AcMa,
    Mode::DcA,
    Mode::AcA,
    Mode::Ncv,
    Mode::Live,
    Mode::Diode,
];

/// Stock key layout: right cycles voltage/resistance/capacitance/continuity, left the current
/// ranges, down the special modes; centre is AUTO.
pub const GROUP_RIGHT: [Mode; 5] = [Mode::DcV, Mode::AcV, Mode::Ohm, Mode::Cap, Mode::Diode];
pub const GROUP_LEFT: [Mode; 4] = [Mode::DcMa, Mode::AcMa, Mode::DcA, Mode::AcA];
pub const GROUP_DOWN: [Mode; 4] = [Mode::Temp, Mode::Freq, Mode::Ncv, Mode::Live];

pub fn group_of(m: Mode) -> &'static [Mode] {
    if GROUP_RIGHT.contains(&m) {
        &GROUP_RIGHT
    } else if GROUP_LEFT.contains(&m) {
        &GROUP_LEFT
    } else if GROUP_DOWN.contains(&m) {
        &GROUP_DOWN
    } else {
        &[Mode::Auto]
    }
}

impl Mode {
    pub fn from_id(id: u8) -> Option<Mode> {
        MODES.iter().copied().find(|m| *m as u8 == id)
    }
    pub fn name(self, en: bool) -> &'static str {
        match (self, en) {
            (Mode::Auto, false) => "Авто",
            (Mode::Auto, true) => "Auto",
            (Mode::DcV, false) => "Напряжение DC",
            (Mode::DcV, true) => "DC voltage",
            (Mode::AcV, false) => "Напряжение AC",
            (Mode::AcV, true) => "AC voltage",
            (Mode::Ohm, false) => "Сопротивление",
            (Mode::Ohm, true) => "Resistance",
            (Mode::Cap, false) => "Ёмкость",
            (Mode::Cap, true) => "Capacitance",
            (Mode::Freq, false) => "Частота",
            (Mode::Freq, true) => "Frequency",
            (Mode::Temp, false) => "Температура",
            (Mode::Temp, true) => "Temperature",
            (Mode::DcMa, false) => "Ток мА DC",
            (Mode::DcMa, true) => "DC current mA",
            (Mode::AcMa, false) => "Ток мА AC",
            (Mode::AcMa, true) => "AC current mA",
            (Mode::DcA, false) => "Ток А DC",
            (Mode::DcA, true) => "DC current A",
            (Mode::AcA, false) => "Ток А AC",
            (Mode::AcA, true) => "AC current A",
            (Mode::Ncv, false) => "Бесконтактный",
            (Mode::Ncv, true) => "Non-contact",
            (Mode::Live, false) => "Фаза",
            (Mode::Live, true) => "Live wire",
            (Mode::Diode, false) => "Прозвонка",
            (Mode::Diode, true) => "Continuity",
        }
    }
    pub fn short(self, en: bool) -> &'static str {
        match self {
            Mode::Auto if en => "Auto",
            Mode::Auto => "Авто",
            Mode::DcV => "DC V",
            Mode::AcV => "AC V",
            Mode::Ohm => "Ω",
            Mode::Cap => "C",
            Mode::Freq => "Hz",
            Mode::Temp => "°C",
            Mode::DcMa => "DC mA",
            Mode::AcMa => "AC mA",
            Mode::DcA => "DC A",
            Mode::AcA => "AC A",
            Mode::Ncv => "NCV",
            Mode::Live => "Live",
            Mode::Diode if en => "Diode",
            Mode::Diode => "Диод",
        }
    }
    /// The 10-byte command that selects this mode.
    pub fn command(self) -> [u8; 10] {
        let id = self as u8;
        [0xAA, 0x55, 0x05, id, 0, 0, 0, 0, 0, id + 5]
    }
}

/// One decoded display frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Reading {
    /// LCD symbols: 0..=9 digits, 10..=21 "AUTOLE- CPSr", 255 unknown
    pub symbols: [u8; 4],
    pub negative: bool,
    /// digits before the decimal point (1..=3), 0 = none
    pub decimal_after: u8,
    pub unit: u8,
    /// 1 = AC, 2 = DC, 255 = neither
    pub ac_dc: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Frame {
    Display(Reading),
    Event(u8),
}

fn segment(v: u8) -> u8 {
    const P: [u8; 20] = [
        0xEB, 0x0A, 0xAD, 0x8F, 0x4E, 0xC7, 0xE7, 0x8A, 0xEF, 0xCF, 0xEE, 0x23, 0x65, 0x27, 0x61, 0xE5, 0x04, 0x00, 0xE1,
        0xEC,
    ];
    let v = v & 0xEF;
    match P.iter().position(|&p| p == v) {
        Some(i) => i as u8,
        None if v == 0x24 => 21,
        None => 255,
    }
}

fn unit_code(b: &[u8]) -> u8 {
    if b[8] & 2 != 0 {
        return if b[7] & 1 != 0 { 7 } else { 6 };
    }
    if b[7] & 0x20 != 0 {
        return if b[6] & 0x40 != 0 { 12 } else if b[7] & 4 != 0 { 13 } else { 11 };
    }
    if b[8] & 1 != 0 {
        return if b[6] & 0x10 != 0 { 8 } else if b[7] & 1 != 0 { 9 } else { 10 };
    }
    if b[7] & 0x10 != 0 {
        return if b[6] & 0x20 != 0 { 15 } else if b[6] & 0x10 != 0 { 16 } else { 17 };
    }
    if b[8] & 0x20 != 0 {
        return 21;
    }
    if b[8] & 4 != 0 {
        return if b[6] & 0x40 != 0 { 19 } else if b[7] & 4 != 0 { 20 } else { 18 };
    }
    1
}

/// Decode a display frame (12 bytes from `5A A5`).
pub fn decode(b: &[u8]) -> Reading {
    let mut symbols = [0; 4];
    for (i, s) in symbols.iter_mut().enumerate() {
        *s = segment((b[2 + i] & 0xF0) | (b[3 + i] & 0x0F));
    }
    Reading {
        symbols,
        negative: b[2] & 0x10 != 0,
        decimal_after: if b[3] & 0x10 != 0 {
            1
        } else if b[4] & 0x10 != 0 {
            2
        } else if b[5] & 0x10 != 0 {
            3
        } else {
            0
        },
        unit: unit_code(b),
        ac_dc: if b[2] & 2 != 0 {
            2
        } else if b[2] & 1 != 0 {
            1
        } else {
            255
        },
    }
}

impl Reading {
    /// The LCD text, e.g. "-1.234", " 99.8", "0L", "AUTO".
    pub fn text(&self, out: &mut [u8; 8]) -> usize {
        const A: &[u8] = b"0123456789AUTOLE- CPSr";
        let mut n = 0;
        if self.negative {
            out[n] = b'-';
            n += 1;
        }
        for (i, &s) in self.symbols.iter().enumerate() {
            out[n] = *A.get(s as usize).unwrap_or(&b'?');
            n += 1;
            if self.decimal_after as usize == i + 1 {
                out[n] = b'.';
                n += 1;
            }
        }
        n
    }

    /// Numeric value in the shown unit as (mantissa, divisor), None for OL / text.
    pub fn number(&self) -> Option<(i32, i32)> {
        let mut m: i32 = 0;
        let mut seen = false;
        let mut first = 4;
        for (i, &s) in self.symbols.iter().enumerate() {
            if s < 10 {
                if !seen {
                    first = i;
                }
                seen = true;
                m = m * 10 + s as i32;
            } else if !(s == 17 && !seen) {
                return None;
            }
        }
        let dp = self.decimal_after as usize;
        if !seen || (dp != 0 && first >= dp) {
            return None;
        }
        let div = if dp != 0 { 10i32.pow((4 - dp) as u32) } else { 1 };
        Some((if self.negative { -m } else { m }, div))
    }

    pub fn is_overload(&self) -> bool {
        self.symbols.contains(&14) && self.number().is_none()
    }

    pub fn unit_str(&self) -> &'static str {
        match self.unit {
            6 => "V",
            7 => "mV",
            8 => "µA",
            9 => "mA",
            10 => "A",
            11 => "Ω",
            12 => "kΩ",
            13 => "MΩ",
            15 => "nF",
            16 => "µF",
            17 => "mF",
            18 => "Hz",
            19 => "kHz",
            20 => "MHz",
            21 => "°C",
            _ => "",
        }
    }
}

/// Reassembles frames from the UART byte stream by their headers.
pub struct Stream {
    buf: [u8; 12],
    n: usize,
}

impl Default for Stream {
    fn default() -> Self {
        Self { buf: [0; 12], n: 0 }
    }
}

impl Stream {
    pub fn feed(&mut self, byte: u8) -> Option<Frame> {
        self.buf[self.n] = byte;
        self.n += 1;
        loop {
            let b = &self.buf[..self.n];
            let need = match b {
                [] => return None,
                [0x5A] | [0xAA] | [0xAA, 0x55] => return None,
                [0x5A, 0xA5, ..] => 12,
                [0xAA, 0x55, 0xFD, ..] => 10,
                _ => {
                    self.buf.copy_within(1..self.n, 0); // not a frame start: drop one byte, resync
                    self.n -= 1;
                    continue;
                }
            };
            if self.n < need {
                return None;
            }
            self.n = 0;
            return Some(if need == 12 {
                if self.buf[2] == 0xFD {
                    Frame::Event(self.buf[3])
                } else {
                    Frame::Display(decode(&self.buf))
                }
            } else {
                Frame::Event(self.buf[3])
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: &Reading) -> std::string::String {
        let mut b = [0; 8];
        let n = r.text(&mut b);
        std::string::String::from_utf8(b[..n].to_vec()).unwrap()
    }

    fn feed(bytes: &[u8]) -> std::vec::Vec<Frame> {
        let mut s = Stream::default();
        bytes.iter().filter_map(|&b| s.feed(b)).collect()
    }

    #[test]
    fn captured_frames() {
        // captures from smhgzll's projects/asic/docs: AUTO screen, then frames from other modes
        let auto = feed(&[0x5A, 0xA5, 0xE4, 0x2E, 0x63, 0x25, 0x07, 0x00, 0x00, 0x00, 0x01, 0x3F]);
        let Frame::Display(r) = auto[0] else { panic!() };
        assert_eq!(text(&r), "AUTO");
        assert_eq!(r.number(), None);
        // an event between noise bytes, then a display frame
        let f = feed(&[0x00, 0x13, 0xAA, 0x55, 0xFD, 0x14, 0, 0, 0, 0xAA, 0x01, 0xBB, 0x5A, 0xA5, 0x04, 0xC0, 0xDF, 0x8F, 0x4A, 0x20, 0, 0, 1, 0x3E]);
        assert_eq!(f[0], Frame::Event(0x14));
        let Frame::Display(r) = f[1] else { panic!() };
        assert_eq!(r.unit_str(), "kΩ");
        assert_eq!(text(&r), " 9.97"); // RESISTANCE_VALIDATION.md: 10 kΩ resistor reads " 9.97" kΩ
        assert_eq!(r.number(), Some((997, 100)));
    }

    #[test]
    fn commands() {
        assert_eq!(Mode::Auto.command(), [0xAA, 0x55, 5, 0x14, 0, 0, 0, 0, 0, 0x19]); // observed at startup
        assert_eq!(Mode::Ohm.command()[9], 0x10);
    }
}
