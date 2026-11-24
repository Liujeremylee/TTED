//! TTED 5x5x5 LED cube firmware for the Raspberry Pi Pico 2 (RP2350), in Rust.
//!
//! The LEDs are refreshed by hardware (PIO0 + a looping DMA chain): the CPU executes zero instructions per
//! layer. See cube.rs. Animations: the eight light shows in lightshow.rs, the seven in animations.rs (rain, ripple,
//! spinner, fireworks, sphere pulse, corkscrew, snake; all of these play one after another), the CubeLibDemo.py
//! sequences and the Quadrum player (TTED.txt).
//!
//! Buttons: 1 (GP13) next preset, 2 (GP10) previous, 3 (GP11) on/off, 4 (GP12) brightness high/low.
//! 'n' over USB serial: next animation.
//! USB serial commands: m = measure & report, t = output self-test, b = reboot to BOOTSEL (for flashing), h = help.

#![no_std]
#![no_main]

mod animations;
mod cube;
mod lightshow;
mod quadrum;
#[cfg(feature = "unlimited")]
mod overclock;

use core::cell::RefCell;
use core::fmt::Write as _;
use core::hint::black_box;
use core::sync::atomic::{AtomicU8, Ordering};

use cortex_m::peripheral::{DWT, NVIC};
use critical_section::Mutex;
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::InputPin;
use fugit::RateExtU32;
use panic_halt as _;
use rp235x_hal as hal;
use hal::pac::{self, interrupt};
use hal::Clock;
use usb_device::{class_prelude::UsbBusAllocator, prelude::*};
use usbd_serial::SerialPort;

use cube::{Cube, LEDS, N, WORDS};

#[link_section = ".start_block"]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

#[link_section = ".bi_entries"]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"TTED LED Cube"),
    hal::binary_info::rp_program_description!(c"5x5x5 LED cube, PIO + DMA refresh (Rust)"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];

const XTAL_HZ: u32 = 12_000_000;
/// 240 MHz: 12 MHz x 120 = 1440 MHz VCO, / 6 / 1. Divides by exactly 6 to give a 40 MHz PIO clock = 20 MHz 595 clock.
const PLL_SYS_240MHZ: hal::pll::PLLConfig =
    hal::pll::PLLConfig { vco_freq: fugit::HertzU32::MHz(1440), refdiv: 1, post_div1: 6, post_div2: 1 };
/// The 595 shift clock the original SPI code used. Raise it to go faster if your 595s and wiring allow.
#[cfg(not(feature = "unlimited"))]
const MAX_SCK_HZ: u32 = 20_000_000;
/// `--features unlimited`: ignore the 595's rating. 60 MHz = PIO divider 2 at 240 MHz; with BOOT_STEP 5
/// (328 MHz) that becomes 82 MHz, the default (84 MHz is the fastest this cube's 595s showed clean; tested Nov. 2026, with the
/// stacked 595s: 84 MHz clean, 85.5 MHz past the edge, 90 MHz faint flicker, 100 MHz and up dark or one LED).
/// '+' still goes to divider 1 (CPU/2) if you want to try.
// Jason, your retirement time has came closer.
#[cfg(feature = "unlimited")]
const MAX_SCK_HZ: u32 = 60_000_000;
/// PIO1 can sample the pins (self-test, timing) only up to this PIO divider; faster clocks skip that check.
const CAPTURE_MIN_DIV: u16 = 4;
/// `unlimited` only: the overclock step (index into overclock::STEPS) to boot at. 0 = 240 MHz, 5 = 328 MHz
/// (595 clock 82 MHz with the divider above). If it ever hangs, the watchdog restarts at 240 MHz (60 MHz).
#[cfg(feature = "unlimited")]
const BOOT_STEP: usize = 5;
/// Extra PIO cycles per slot. 0 = fastest. Raise it if you see ghosting between layers.
const HOLD: u32 = 0;
/// Limits for the '+'/'-' 595 clock commands: CPU/3/2 = 40 MHz ... CPU/24/2 = 5 MHz.
#[cfg(not(feature = "unlimited"))]
const MIN_DIV: u16 = 3;
#[cfg(feature = "unlimited")]
const MIN_DIV: u16 = 1;
const MAX_DIV: u16 = 24;

static USB_DEV: Mutex<RefCell<Option<UsbDevice<'static, hal::usb::UsbBus>>>> = Mutex::new(RefCell::new(None));
static SERIAL: Mutex<RefCell<Option<SerialPort<'static, hal::usb::UsbBus>>>> = Mutex::new(RefCell::new(None));
static CMD: AtomicU8 = AtomicU8::new(0);

/// A '@keyword' line being typed over USB serial (filled by the USB interrupt).
struct Line {
    typing: bool,
    ready: bool,
    buf: heapless::Vec<u8, 32>,
}
static LINE: Mutex<RefCell<Line>> =
    Mutex::new(RefCell::new(Line { typing: false, ready: false, buf: heapless::Vec::new() }));

/// The finished '@keyword' line, if one has arrived.
fn take_line() -> Option<heapless::Vec<u8, 32>> {
    critical_section::with(|cs| {
        let mut l = LINE.borrow_ref_mut(cs);
        if l.ready {
            l.ready = false;
            Some(core::mem::take(&mut l.buf))
        } else {
            None
        }
    })
}

/// Lower-case letters and digits only: "Plane Sweep" -> "planesweep".
fn norm(s: &[u8]) -> heapless::Vec<u8, 32> {
    s.iter().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).take(32).collect()
}

/// Find a mode by name: exact match first, then a name starting with it, then a name containing it.
/// A number picks the mode with that number in the '@list'.
fn find_mode(w: &[u8]) -> Option<u8> {
    if !w.is_empty() && w.iter().all(u8::is_ascii_digit) {
        let n = core::str::from_utf8(w).ok()?.parse::<u8>().ok()?;
        return (n < MODES).then_some(n);
    }
    let names = || (0..MODES).map(|m| (m, norm(mode_name(m).as_bytes())));
    names()
        .find(|(_, n)| n[..] == *w)
        .or_else(|| names().find(|(_, n)| n.starts_with(w)))
        .or_else(|| names().find(|(_, n)| n.windows(w.len()).any(|x| x == w)))
        .map(|(m, _)| m)
        .or_else(|| {
            // forgiving: "bounce" -> "bouncing balls". The name sharing the longest start with the word wins,
            // if that start is 4+ letters and at most 2 letters short of the word.
            let common = |n: &[u8]| n.iter().zip(w).take_while(|(a, b)| a == b).count();
            let (m, best) = names().map(|(m, n)| (m, common(&n))).max_by_key(|&(_, k)| k)?;
            (best >= 4 && best + 2 >= w.len()).then_some(m)
        })
}

/// Write to USB serial; gives up after a few ms if nobody is reading, so animations never stall.
fn out(s: &str) {
    let b = s.as_bytes();
    let mut off = 0;
    let mut idle = 0u32;
    while off < b.len() {
        let n = critical_section::with(|cs| match SERIAL.borrow_ref_mut(cs).as_mut() {
            Some(ser) => ser.write(&b[off..]).unwrap_or(0),
            None => b.len() - off,
        });
        if n == 0 {
            idle += 1;
            if idle > 100_000 {
                return;
            }
        } else {
            idle = 0;
            off += n;
        }
    }
}

macro_rules! say {
    ($($arg:tt)*) => {{
        let mut s: heapless::String<200> = heapless::String::new();
        let _ = write!(s, $($arg)*);
        out(&s);
    }};
}

struct Next; // "stop this animation, go to the next one"

struct Ctx<'a, B: InputPin> {
    cube: &'a mut Cube,
    anim: &'a quadrum::Animation,
    anim_err: Option<&'static str>,
    timer: hal::Timer<hal::timer::CopyableTimer0>,
    /// Buttons 1-4 (GP13, GP10, GP11, GP12): next preset, previous preset, on/off, brightness high/low.
    buttons: [B; 4],
    btn_down: [bool; 4],
    /// When each button last changed state (us), for debouncing.
    btn_at: [u64; 4],
    /// The preset to come back to when the cube is switched on again after '@off' / button 3.
    resume: u8,
    mode: u8,
    rng: u32,
    boot_test: Option<bool>,
    #[cfg(feature = "unlimited")]
    wd: hal::Watchdog,
    #[cfg(feature = "unlimited")]
    step: usize,
    #[cfg(feature = "unlimited")]
    crashed: Option<u32>,
    /// Mode picked by a '@keyword': the main loop switches to it instead of the next mode.
    jump: Option<u8>,
    /// true after a '@keyword' pick: keep repeating that preset instead of moving on ('@auto' clears it).
    hold: bool,
    /// Animation tempo in percent ('@tempo'): frame waits are divided by it. 100 = as written.
    tempo: u32,
    /// The PIO divider set at power-up (for '@speed default' / '@reset').
    boot_div: u16,
    /// '@probe': the bit driven LOW in the test pattern (None = every bit off).
    probe: Option<u8>,
}

impl<B: InputPin> Ctx<'_, B> {
    /// '@speed': show the speed. '@speed 80': the fastest 595 clock at or below 80 MHz (unlimited build: picks
    /// the CPU clock step and PIO divider; normal build: the divider only). '@speed jason': the original design
    /// speed, 20 MHz at 240 MHz CPU. '@speed default': the power-up setting.
    fn speed(&mut self, arg: &[u8]) {
        match arg {
            b"" => {}
            b"jason" | b"original" | b"stock" => {
                self.set_speed_mhz(20);
                say!("Jason's setting: 595 clock 20 MHz (the original SPI speed), CPU 240 MHz\r\n");
            }
            b"default" | b"defaults" | b"reset" | b"boot" => self.speed_default(),
            _ => match core::str::from_utf8(arg).ok().and_then(|s| s.parse::<u32>().ok()) {
                Some(mhz) => self.set_speed_mhz(mhz),
                None => {
                    say!("speed: a number of MHz (e.g. @speed 80), jason, or default\r\n");
                    return;
                }
            },
        }
        let cpu = self.cube.sys_hz;
        let div = self.cube.pio_div as u32;
        let pio = cpu / div;
        say!("speed: 595 clock {:.1} MHz | CPU {} MHz, PIO CPU/{} | {} full brightness cycles/s | tempo {}%\r\n",
            pio as f32 / 2e6, cpu / 1_000_000, div, pio / (cube::pio_cycles(self.cube.hold) * WORDS as u32), self.tempo);
    }

    /// Set the 595 clock to the fastest setting at or below `mhz` (ties: the lowest CPU clock).
    fn set_speed_mhz(&mut self, mhz: u32) {
        let target = mhz.saturating_mul(1_000_000); // never above what was asked for
        // CPU clocks to choose from, lowest first
        let mut cpus: heapless::Vec<(usize, u32), 16> = heapless::Vec::new();
        #[cfg(feature = "unlimited")]
        for (i, s) in overclock::STEPS.iter().enumerate() {
            let _ = cpus.push((i, s.mhz * 1_000_000));
        }
        #[cfg(not(feature = "unlimited"))]
        let _ = cpus.push((0, self.cube.sys_hz));
        let mut best: Option<(usize, u16, u32)> = None; // (clock step, PIO divider, 595 clock)
        for &(step, cpu) in cpus.iter() {
            for div in MIN_DIV..=MAX_DIV {
                let hz = cpu / div as u32 / 2;
                if hz <= target && best.map_or(true, |(_, _, b)| hz > b) {
                    best = Some((step, div, hz)); // strictly faster only: ties keep the lower CPU clock
                }
            }
        }
        match best {
            Some((step, div, _)) => self.set_clock(step, div),
            None => say!("speed: {} MHz is below the slowest setting\r\n", mhz),
        }
    }

    /// The power-up speed (unlimited: BOOT_STEP and the boot divider, i.e. 82 MHz).
    fn speed_default(&mut self) {
        #[cfg(feature = "unlimited")]
        let step = BOOT_STEP;
        #[cfg(not(feature = "unlimited"))]
        let step = 0;
        self.set_clock(step, self.boot_div);
    }

    /// CPU clock step (unlimited build only; ignored otherwise) and PIO divider.
    fn set_clock(&mut self, _step: usize, div: u16) {
        #[cfg(feature = "unlimited")]
        if _step != self.step {
            self.set_clock_step(_step);
        }
        self.cube.set_pio_div(div);
    }

    /// '@tempo': show the animation tempo. '@tempo 150': 1.5x as fast, '@tempo 50': half speed (25..400 %).
    fn tempo(&mut self, arg: &[u8]) {
        if !arg.is_empty() {
            match core::str::from_utf8(arg).ok().and_then(|s| s.parse::<u32>().ok()) {
                Some(p) => self.tempo = p.clamp(25, 400),
                None => {
                    say!("tempo: a percentage, e.g. @tempo 150 (faster) or @tempo 50 (slower); 100 = normal\r\n");
                    return;
                }
            }
        }
        say!("tempo: {}% (animations play at {}x speed)\r\n", self.tempo, self.tempo as f32 / 100.0);
    }

    /// '@reset': power-up speed, normal tempo, all presets cycling.
    fn reset(&mut self) {
        self.speed_default();
        self.tempo = 100;
        self.hold = false;
        self.cube.bright = cube::BRIGHT_MAX;
        say!("reset: power-up speed, tempo 100%, full brightness, presets cycling (auto)\r\n");
        self.speed(b"");
    }

    /// Handle a '@keyword' line. Some(Err(Next)) = leave the current animation (the main loop then reads `jump`).
    fn keyword(&mut self, line: &[u8]) -> Option<Result<(), Next>> {
        let w = norm(line);
        match &w[..] {
            b"" => None,
            b"list" | b"help" | b"presets" => {
                list_modes(self.mode, self.hold);
                None
            }
            b"auto" | b"cycle" | b"loop" => {
                self.hold = false;
                say!("auto: the presets play one after another\r\n");
                if self.mode == OFF {
                    self.jump = Some(0);
                    return Some(Err(Next));
                }
                None
            }
            b"next" | b"skip" => Some(Err(Next)),
            b"prev" | b"previous" | b"back" => Some(self.previous()),
            b"off" | b"dark" | b"stop" | b"blackout" => Some(self.power(false)),
            b"on" | b"resume" => Some(self.power(true)),
            b"power" | b"toggle" => Some(self.power(self.mode == OFF)),
            b"dim" | b"darker" | b"down" => {
                self.brightness(self.cube.bright.saturating_sub(1).max(1));
                None
            }
            b"bright" | b"brighter" | b"up" => {
                self.brightness((self.cube.bright + 1).min(cube::BRIGHT_MAX));
                None
            }
            b"low" | b"lowest" | b"min" => {
                self.brightness(1);
                None
            }
            b"high" | b"full" | b"max" => {
                self.brightness(cube::BRIGHT_MAX);
                None
            }
            _ if w.starts_with(b"brightness") => {
                let arg = &w[10..];
                if arg.is_empty() {
                    self.brightness(self.cube.bright);
                } else {
                    match core::str::from_utf8(arg).ok().and_then(|s| s.parse::<u8>().ok()) {
                        Some(b) if (1..=cube::BRIGHT_MAX).contains(&b) => self.brightness(b),
                        _ => say!("brightness: 1 to {} (e.g. @brightness 3)\r\n", cube::BRIGHT_MAX),
                    }
                }
                None
            }
            _ if w.starts_with(b"speed") => {
                self.speed(&w[5..]);
                None
            }
            _ if w.starts_with(b"probe") => {
                let arg = &w[5..];
                let bit = match arg {
                    b"" | b"all" | b"none" | b"off" => None,
                    _ => match core::str::from_utf8(arg).ok().and_then(|s| s.parse::<u8>().ok()) {
                        Some(b) if b < 32 => Some(b),
                        _ => {
                            say!("probe: a bit number 0-31 (e.g. @probe 7), or @probe all\r\n");
                            return None;
                        }
                    },
                };
                self.probe = bit;
                describe_probe(bit);
                self.jump = Some(PROBE);
                Some(Err(Next))
            }
            _ if w.starts_with(b"tempo") => {
                self.tempo(&w[5..]);
                None
            }
            b"jason" | b"jasonssetting" | b"jasons" => {
                self.speed(b"jason");
                None
            }
            b"reset" | b"default" | b"defaults" => {
                self.reset();
                if self.mode == OFF {
                    self.jump = Some(0);
                    return Some(Err(Next));
                }
                None
            }
            _ => match find_mode(&w) {
                Some(m) => {
                    self.jump = Some(m);
                    self.hold = true;
                    Some(Err(Next))
                }
                None => {
                    say!("unknown preset '{}' (@list shows them all)\r\n", core::str::from_utf8(line).unwrap_or("?"));
                    None
                }
            },
        }
    }

    fn now_us(&self) -> u64 {
        self.timer.get_counter().ticks()
    }

    /// On (back to the preset that was playing) or off (all dark).
    fn power(&mut self, on: bool) -> Result<(), Next> {
        if on {
            self.jump = Some(if self.mode == OFF || self.mode == PROBE { self.resume } else { self.mode });
        } else {
            if self.mode < MODES {
                self.resume = self.mode;
            }
            self.jump = Some(OFF);
        }
        Err(Next)
    }

    /// The previous preset (from off: the one before the preset that was playing).
    fn previous(&mut self) -> Result<(), Next> {
        let from = if self.mode < MODES { self.mode } else { self.resume };
        self.jump = Some((from + MODES - 1) % MODES);
        Err(Next)
    }

    /// Brightness 1..=5 (20 % steps, 5 = full); shown at once.
    fn brightness(&mut self, b: u8) {
        self.cube.bright = b;
        say!("brightness: {} of {} ({}%)\r\n", b, cube::BRIGHT_MAX, b as u32 * 100 / cube::BRIGHT_MAX as u32);
        if self.mode == OFF {
            return;
        }
        if self.mode != PROBE {
            self.cube.show(); // the picture on screen right now, at the new brightness
        }
    }

    /// Check the four buttons (debounced, 30 ms). Buttons 1/2/3 switch presets or power, so they end the
    /// animation; button 4 just changes the brightness.
    fn check_buttons(&mut self) -> Option<Result<(), Next>> {
        let now = self.now_us();
        for i in 0..4 {
            let pressed = self.buttons[i].is_low().unwrap_or(false);
            if pressed == self.btn_down[i] || now - self.btn_at[i] < 30_000 {
                continue;
            }
            self.btn_down[i] = pressed;
            self.btn_at[i] = now;
            if !pressed {
                continue;
            }
            match i {
                0 => {
                    if self.mode == OFF {
                        return Some(self.power(true)); // from off, button 1 also switches back on
                    }
                    return Some(Err(Next));
                }
                1 => return Some(self.previous()),
                2 => return Some(self.power(self.mode == OFF)),
                // 5 -> 4 -> 3 -> 2 -> 1 -> back to 5
                _ => self.brightness(if self.cube.bright <= 1 { cube::BRIGHT_MAX } else { self.cube.bright - 1 }),
            }
        }
        None
    }

    /// Display the current picture, then wait (scaled by the '@tempo' setting).
    fn frame(&mut self, ms: u32) -> Result<(), Next> {
        self.cube.show();
        self.wait(ms * 100 / self.tempo.max(1))
    }

    /// Wait `ms`, handling serial commands and button 1 meanwhile.
    fn wait(&mut self, ms: u32) -> Result<(), Next> {
        let start = self.now_us();
        loop {
            #[cfg(feature = "unlimited")]
            self.wd.feed();
            match CMD.swap(0, Ordering::Relaxed) {
                0 => {}
                b'm' | b'M' => self.report(),
                b't' | b'T' => {
                    self.selftest(true);
                }
                b'n' | b'N' => return Err(Next),
                c @ (b'+' | b'-') => {
                    // 595 clock = CPU / div / 2. div 6 = 20 MHz (the original). Smaller div = faster.
                    let div = self.cube.pio_div;
                    let new = if c == b'+' { div.saturating_sub(1).max(MIN_DIV) } else { (div + 1).min(MAX_DIV) };
                    self.cube.set_pio_div(new);
                    let pio_hz = self.cube.sys_hz / new as u32;
                    let cyc = cube::pio_cycles(self.cube.hold);
                    say!("595 clock {:.2} MHz (CPU/{}) -> {} full PWM cycles/s{}\r\n",
                        pio_hz as f32 / 2e6, new, pio_hz / (cyc * WORDS as u32),
                        if new < 6 { "  [above the 20 MHz the board was designed for: check the cube carefully]" } else { "" });
                }
                #[cfg(feature = "unlimited")]
                c @ (b'>' | b'<') => {
                    let last = overclock::STEPS.len() - 1;
                    let to = if c == b'>' { (self.step + 1).min(last) } else { self.step.saturating_sub(1) };
                    self.set_clock_step(to);
                }
                b'h' | b'H' | b'?' => help(),
                c => say!("unknown command '{}' (h = help)\r\n", c as char),
            }
            if let Some(line) = take_line() {
                if let Some(r) = self.keyword(&line) {
                    return r;
                }
            }
            if let Some(r) = self.check_buttons() {
                return r;
            }
            if self.now_us() - start >= ms as u64 * 1000 {
                return Ok(());
            }
            self.timer.delay_us(250);
        }
    }

    fn random(&mut self) -> u32 {
        let mut x = self.rng; // xorshift32
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// 1) the fast table builder equals the original CubeLib algorithm for a random picture;
    /// 2) the words actually leaving GP1..3 (captured by PIO1) equal the table, latch after every 32 bits.
    fn selftest(&mut self, verbose: bool) -> bool {
        let saved = self.cube.data;
        let saved_bright = core::mem::replace(&mut self.cube.bright, cube::BRIGHT_MAX); // the check compares against full brightness
        for i in 0..LEDS {
            self.cube.data[i] = (self.random() % 13) as u8; // 0..12, including values above full brightness
        }
        self.cube.show();
        self.cube.wait_shown(); // the capture below must see this table
        let data = self.cube.data;
        let table: [u32; WORDS] = self.cube.latest_table().try_into().unwrap();

        let mut bad_words = 0;
        for s in 0..cube::LEVELS {
            for z in 0..N {
                if table[cube::idx(s, z)] != cube::reference_word(&data, s, z) {
                    bad_words += 1;
                }
            }
        }

        // the blank words must be all-off
        for z in 0..N {
            for k in 0..cube::BLANKS {
                if table[cube::idx(0, z) - cube::BLANKS + k] != cube::BLANK {
                    bad_words += 1;
                }
            }
        }

        let mut cap = [0u32; 3 * WORDS];
        let mut st = [0u32; 3 * WORDS];
        let can_capture = self.cube.pio_div >= CAPTURE_MIN_DIV;
        let got = if can_capture { self.cube.capture(&mut cap, &mut st) } else { 0 };
        let mut offset = None;
        if got == cap.len() {
            for k in 0..WORDS {
                if (0..got).all(|i| cap[i] == table[(k + i) % WORDS]) {
                    offset = Some(k);
                    break;
                }
            }
        }
        self.cube.data = saved;
        self.cube.bright = saved_bright;
        self.cube.show();

        let ok = bad_words == 0 && (offset.is_some() || !can_capture);
        if verbose {
            say!("self-test: table vs original algorithm: {} of {} words differ\r\n", bad_words, WORDS);
            match offset {
                Some(_) => say!("self-test: {} words captured on GP1-3 = table, bit for bit: PASS\r\n", got),
                None if !can_capture => say!(
                    "self-test: pin capture SKIPPED: the 595 clock (CPU/{}/2) is too fast for PIO1 to sample (needs CPU/{} or slower)\r\n",
                    self.cube.pio_div, CAPTURE_MIN_DIV),
                None => {
                    say!("self-test: pin capture got {} words, NOT equal to the table: FAIL\r\n", got);
                    for i in 0..got.min(6) {
                        say!("  captured {:08x}\r\n", cap[i]);
                    }
                }
            }
            say!("self-test overall: {}\r\n", if ok { "PASS" } else { "FAIL" });
        }
        ok
    }

    /// Overclock (or underclock) CPU + PIO to STEPS[to], then check that everything still works.
    #[cfg(feature = "unlimited")]
    fn set_clock_step(&mut self, to: usize) {
        let s = &overclock::STEPS[to];
        if to == self.step {
            say!("CPU/PIO already at {} MHz (end of the overclock table)\r\n", s.mhz);
            return;
        }
        let mv = overclock::millivolts(s.vsel);
        say!("switching CPU/PIO to {} MHz: core {}.{:02} V, flash CPU/{} ...\r\n", s.mhz, mv / 1000, (mv % 1000) / 10, s.qmi_div);
        overclock::mark_attempt(to); // if this clock hangs the chip, the watchdog reboots at 240 MHz and says so
        self.wd.feed();
        self.cube.sys_hz = overclock::change(self.step, to);
        self.step = to;
        self.stability_check();
    }

    /// PIO + DMA + CPU check at the current clock.
    #[cfg(feature = "unlimited")]
    fn stability_check(&mut self) {
        // PIO/DMA: pin capture needs the PIO at CPU/4 or slower, so drop to that for the test only.
        let div = self.cube.pio_div;
        self.cube.set_pio_div(CAPTURE_MIN_DIV);
        let pins_ok = self.selftest(false);
        self.cube.set_pio_div(div);
        self.wd.feed();
        // CPU: 2000 random tables, fast builder vs reference.
        let mut data = [0u8; LEDS];
        let mut a = [0u32; WORDS];
        let mut b = [0u32; WORDS];
        let mut tmp = cube::Scratch::new();
        let mut wrong = 0;
        for _ in 0..2000 {
            for d in data.iter_mut() {
                *d = self.random() as u8;
            }
            cube::build(&data, &mut a, &mut tmp);
            cube::build_v1(&data, &mut b);
            if a != b {
                wrong += 1;
            }
        }
        self.wd.feed();
        // Refresh rate at this clock (DMA pass counter over 250 ms).
        cube::PASSES.store(0, Ordering::Relaxed);
        let t0 = self.now_us();
        cube::passes_irq_enable(true);
        self.timer.delay_ms(250);
        cube::passes_irq_enable(false);
        let passes = cube::PASSES.load(Ordering::Relaxed) as u64 * 1_000_000 / (self.now_us() - t0);
        let pio_hz = self.cube.sys_hz / self.cube.pio_div as u32;
        say!("{} MHz: PIO/DMA pin self-test {} | CPU: {} of 2000 tables wrong | {} full PWM cycles/s, 595 clock {} MHz\r\n",
            self.cube.sys_hz / 1_000_000, if pins_ok { "PASS" } else { "FAIL" }, wrong, passes, pio_hz / 2_000_000);
    }

    fn report(&mut self) {
        let sys = self.cube.sys_hz;
        let div = self.cube.pio_div as u32;
        let pio_hz = sys / div;
        let cyc = cube::pio_cycles(self.cube.hold);
        say!("\r\n=== TTED LED cube - Rust, PIO + DMA refresh ===\r\n");
        if cfg!(feature = "unlimited") {
            say!("UNLIMITED build: 595 clock limit ignored, fast-slew 12 mA pads\r\n");
        }
        say!("CPU {} MHz | PIO {} MHz (CPU/{}) | 595 shift clock {} MHz\r\n", sys / 1_000_000, pio_hz / 1_000_000, div, pio_hz / 2_000_000);
        say!("PIO program: {} instructions, {} PIO cycles per slot (32 bits x 2 + 2 latch; next word pulled during the latch)\r\n",
            if self.cube.hold == 0 { 4 } else { 6 }, cyc);
        say!("CPU work per slot / layer / refresh: 0 instructions (DMA feeds the PIO)\r\n");
        say!("table: {} words per pass = per layer {} blank + {} brightness slots, layer by layer\r\n",
            WORDS, cube::BLANKS, cube::LEVELS);
        say!("expected: slot {} ns | layer switches {}/s | {} full PWM cycles/s\r\n",
            cyc as u64 * 1_000_000_000 / pio_hz as u64, pio_hz / (cyc * WORDS as u32) * N as u32, pio_hz / (cyc * WORDS as u32));

        // Measured: complete table passes per second, counted by DMA channel 1's interrupt.
        cube::PASSES.store(0, Ordering::Relaxed);
        let t0 = self.now_us();
        cube::passes_irq_enable(true);
        #[cfg(feature = "unlimited")]
        self.wd.feed();
        self.timer.delay_ms(1000);
        #[cfg(feature = "unlimited")]
        self.wd.feed();
        cube::passes_irq_enable(false);
        let us = self.now_us() - t0;
        let passes = cube::PASSES.load(Ordering::Relaxed) as u64 * 1_000_000 / us;
        // Each pass shows every layer once (its blank words, then its LEVELS slots back to back).
        say!("measured: {} full PWM cycles/s = {} words/s, layer switches {}/s\r\n",
            passes, passes * WORDS as u64, passes * N as u64);

        // Measured: time between words on the pins (PIO1 capture, CPU cycle counter).
        let mut cap = [0u32; 3 * WORDS];
        let mut st = [0u32; 3 * WORDS];
        let got = if self.cube.pio_div >= CAPTURE_MIN_DIV { self.cube.capture(&mut cap, &mut st) } else { usize::MAX };
        if got == usize::MAX {
            say!("measured on the pins: skipped (too fast for PIO1 to sample; the DMA pass count above is the measurement)\r\n");
        } else if got > 1 {
            let span = st[got - 1].wrapping_sub(st[0]) as u64;
            let per = span * 1000 / (got as u64 - 1); // milli-cycles
            say!("measured on the pins: one word every {}.{:03} CPU cycles = {} ns ({} PIO cycles)\r\n",
                per / 1000, per % 1000, per * 1_000_000 / sys as u64, (per + div as u64 * 500) / (div as u64 * 1000));
        } else {
            say!("measured on the pins: capture timed out\r\n");
        }

        // Table build cost (the only CPU work, once per animation frame).
        let mut data = [0u8; LEDS];
        let mut table_out = [0u32; WORDS];
        let mut tmp = cube::Scratch::new();
        let overhead = {
            let a = DWT::cycle_count();
            let b = DWT::cycle_count();
            b.wrapping_sub(a)
        };
        type Builder = fn(&[u8; LEDS], &mut [u32; WORDS], &mut cube::Scratch);
        fn v1(d: &[u8; LEDS], o: &mut [u32; WORDS], _t: &mut cube::Scratch) {
            cube::build_v1(d, o)
        }
        #[inline(never)]
        fn empty(_d: &[u8; LEDS], _o: &mut [u32; WORDS], _t: &mut cube::Scratch) {}
        let builders: [(&str, Builder); 4] = [
            ("empty fn (harness cost)", empty),
            ("v1 (first Rust)", v1),
            ("v2 from flash  ", cube::build),
            ("v2 from SRAM   ", cube::build_ram),
        ];
        let mut check = [0u32; WORDS];
        for (name, f) in builders {
            let (mut min, mut max, mut sum, mut wrong) = (u32::MAX, 0u32, 0u64, 0u32);
            const RUNS: u32 = 1000;
            for _ in 0..RUNS {
                for d in data.iter_mut() {
                    *d = (self.random() & 0xFF) as u8 % 13;
                }
                let c = cortex_m::interrupt::free(|_| {
                    let a = DWT::cycle_count();
                    f(black_box(&data), black_box(&mut table_out), black_box(&mut tmp));
                    DWT::cycle_count().wrapping_sub(a).saturating_sub(overhead)
                });
                min = min.min(c);
                max = max.max(c);
                sum += c as u64;
                if !name.starts_with("empty") {
                    cube::build_v1(&data, &mut check);
                    if table_out != check {
                        wrong += 1;
                    }
                }
            }
            let avg = (sum / RUNS as u64) as u32;
            say!("table build {}: min {} / avg {} / max {} cycles = {} ns{}\r\n",
                name, min, avg, max, avg as u64 * 1_000_000_000 / sys as u64,
                if wrong == 0 { "" } else { "  RESULTS DIFFER!" });
        }
        // Full-range brightness values (0..255) must also give identical tables.
        let mut wrong = 0;
        for _ in 0..200 {
            for d in data.iter_mut() {
                *d = self.random() as u8;
            }
            cube::build(&data, &mut table_out, &mut tmp);
            cube::build_v1(&data, &mut check);
            if table_out != check {
                wrong += 1;
            }
        }
        say!("v2 vs v1 with brightness 0..255: {} of 200 tables differ\r\n", wrong);

        // show() = pick a free table + build (from SRAM) + barrier + queue it. Never waits for the DMA.
        let (mut min, mut max) = (u32::MAX, 0u32);
        for _ in 0..1000 {
            let c = cortex_m::interrupt::free(|_| {
                let a = DWT::cycle_count();
                self.cube.show();
                DWT::cycle_count().wrapping_sub(a).saturating_sub(overhead)
            });
            min = min.min(c);
            max = max.max(c);
        }
        say!("show(): min {} / max {} CPU cycles = {} ns typical (no waiting for the DMA)\r\n",
            min, max, min as u64 * 1_000_000_000 / sys as u64);
        let a = DWT::cycle_count();
        self.cube.wait_shown();
        let c = DWT::cycle_count().wrapping_sub(a);
        say!("show() to picture on the LEDs: {} us (the pass in progress finishes first)\r\n",
            c as u64 * 1_000_000 / sys as u64);

        let qmi = unsafe { &*pac::QMI::ptr() };
        let t = qmi.m0_timing().read().bits();
        say!("flash: QMI clkdiv {} -> {} MHz flash clock\r\n", t & 0xFF, sys / 1_000_000 / (t & 0xFF).max(1));
        #[cfg(feature = "unlimited")]
        {
            let mv = overclock::millivolts(overclock::vreg_vsel());
            say!("overclock: step {} of {} | core {}.{:02} V | '>' faster, '<' slower\r\n",
                self.step, overclock::STEPS.len() - 1, mv / 1000, (mv % 1000) / 10);
            if let Some(m) = self.crashed {
                say!("last reset: watchdog, after the chip hung at {} MHz -> started at the safe clock\r\n", m);
            }
        }
        match self.boot_test {
            Some(true) => say!("boot self-test: PASS\r\n"),
            Some(false) => say!("boot self-test: FAIL (send 't' for details)\r\n"),
            None => {}
        }
        if let Some(e) = self.anim_err {
            say!("Quadrum animation: {}\r\n", e);
        } else {
            say!("Quadrum animation: {} frames\r\n", self.anim.frames);
        }
    }
}

fn help() {
    out("\r\nTTED LED cube (Rust). Commands: m = measure, t = self-test, n = next animation,\r\n");
    out("+ / - = faster / slower 595 clock (default 20 MHz), b = BOOTSEL (flash mode), h = help\r\n");
    out("Presets by name: @plasma, @fire, @lava ... + Enter (@list = all of them, @auto = cycle, @off = dark)\r\n");
    out("@prev, @on, @bright, @dim, @brightness 1-5. Buttons: 1 next, 2 previous, 3 on/off, 4 brightness 5..1\r\n");
}

#[hal::entry]
fn main() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let mut cp = cortex_m::Peripherals::take().unwrap();

    // ---- clocks: 240 MHz system, 48 MHz USB ----
    // Read the crash memory before anything else touches the watchdog.
    #[cfg(feature = "unlimited")]
    let crashed_at = overclock::crashed_at();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let xosc = hal::xosc::setup_xosc_blocking(pac.XOSC, XTAL_HZ.Hz()).unwrap();
    watchdog.enable_tick_generation((XTAL_HZ / 1_000_000) as u16);
    let mut clocks = hal::clocks::ClocksManager::new(pac.CLOCKS);
    let pll_sys = hal::pll::setup_pll_blocking(pac.PLL_SYS, xosc.operating_frequency(), PLL_SYS_240MHZ, &mut clocks, &mut pac.RESETS).unwrap();
    let pll_usb = hal::pll::setup_pll_blocking(
        pac.PLL_USB,
        xosc.operating_frequency(),
        hal::pll::common_configs::PLL_USB_48MHZ,
        &mut clocks,
        &mut pac.RESETS,
    )
    .unwrap();
    clocks.init_default(&xosc, &pll_sys, &pll_usb).unwrap();
    let sys_hz = clocks.system_clock.freq().to_Hz();

    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();

    let timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);

    // ---- pins ----
    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(pac.IO_BANK0, pac.PADS_BANK0, sio.gpio_bank0, &mut pac.RESETS);
    #[allow(unused_mut)]
    let mut _latch = pins.gpio1.into_function::<hal::gpio::FunctionPio0>();
    #[allow(unused_mut)]
    let mut _sck = pins.gpio2.into_function::<hal::gpio::FunctionPio0>();
    #[allow(unused_mut)]
    let mut _data = pins.gpio3.into_function::<hal::gpio::FunctionPio0>();
    #[cfg(feature = "unlimited")]
    {
        // Sharpest edges the pads can make: needed for a 120 MHz clock on GP2.
        use hal::gpio::{OutputDriveStrength::TwelveMilliAmps, OutputSlewRate::Fast};
        _latch.set_slew_rate(Fast);
        _latch.set_drive_strength(TwelveMilliAmps);
        _sck.set_slew_rate(Fast);
        _sck.set_drive_strength(TwelveMilliAmps);
        _data.set_slew_rate(Fast);
        _data.set_drive_strength(TwelveMilliAmps);
    }
    // 1 next preset, 2 previous preset, 3 on/off, 4 brightness high/low
    let buttons = [
        pins.gpio13.into_pull_up_input().into_dyn_pin(),
        pins.gpio10.into_pull_up_input().into_dyn_pin(),
        pins.gpio11.into_pull_up_input().into_dyn_pin(),
        pins.gpio12.into_pull_up_input().into_dyn_pin(),
    ];

    // ---- DMA out of reset (channels 0 and 1 are driven directly by cube.rs) ----
    use hal::dma::DMAExt;
    let _dma = pac.DMA.split(&mut pac.RESETS);

    // ---- cube: start the hardware refresh ----
    let cube: &'static mut Cube = cortex_m::singleton!(: Cube = Cube::new()).unwrap();
    cube.start(pac.PIO0, pac.PIO1, &mut pac.RESETS, sys_hz, MAX_SCK_HZ, HOLD);

    // ---- Quadrum animation ----
    let anim: &'static mut quadrum::Animation = cortex_m::singleton!(: quadrum::Animation = quadrum::Animation::empty()).unwrap();
    let anim_err = anim.parse(quadrum::SOURCE).err();

    // ---- USB serial ----
    let bus: &'static UsbBusAllocator<hal::usb::UsbBus> = cortex_m::singleton!(
        : UsbBusAllocator<hal::usb::UsbBus> = UsbBusAllocator::new(hal::usb::UsbBus::new(
            pac.USB, pac.USB_DPRAM, clocks.usb_clock, true, &mut pac.RESETS))
    )
    .unwrap();
    let serial = SerialPort::new(bus);
    let dev = UsbDeviceBuilder::new(bus, UsbVidPid(0x2E8A, 0x000A))
        .strings(&[StringDescriptors::default().manufacturer("BCIT TTED").product("LED Cube (Rust)").serial_number("TTED-CUBE")])
        .unwrap()
        .device_class(usbd_serial::USB_CLASS_CDC)
        .build();
    critical_section::with(|cs| {
        USB_DEV.borrow(cs).replace(Some(dev));
        SERIAL.borrow(cs).replace(Some(serial));
    });
    unsafe {
        NVIC::unmask(pac::Interrupt::USBCTRL_IRQ);
        NVIC::unmask(pac::Interrupt::DMA_IRQ_0);
    }

    let boot_div = cube.pio_div;
    let mut ctx = Ctx {
        cube,
        anim,
        anim_err,
        timer,
        buttons,
        btn_down: [false; 4],
        btn_at: [0; 4],
        resume: 0,
        mode: 0,
        rng: 0x1234_5678 ^ (timer.get_counter().ticks() as u32),
        boot_test: None,
        #[cfg(feature = "unlimited")]
        wd: watchdog,
        #[cfg(feature = "unlimited")]
        step: 0,
        #[cfg(feature = "unlimited")]
        crashed: crashed_at,
        jump: None,
        hold: false,
        tempo: 100,
        boot_div,
        probe: None,
    };
    if ctx.rng == 0 {
        ctx.rng = 1;
    }
    ctx.boot_test = Some(ctx.selftest(false));
    #[cfg(feature = "unlimited")]
    {
        ctx.wd.start(fugit::MicrosDurationU32::millis(3000));
        // Boot overclocked, unless the previous run hung at an overclock (then stay at 240 MHz).
        if ctx.crashed.is_none() && BOOT_STEP > 0 {
            ctx.set_clock_step(BOOT_STEP);
        }
    }

    // Modes: the light shows (lightshow.rs), then the animations.rs ones, then the CubeLib demo, then the
    // Quadrum player (TTED.txt).
    loop {
        let r = match ctx.mode {
            m if m < lightshow::COUNT => lightshow::play(&mut ctx, m),
            m if m < SHOWS => animations::play(&mut ctx, m - lightshow::COUNT),
            DEMO => demo(&mut ctx),
            PROBE => {
                let bit = ctx.probe;
                ctx.cube.show_words(|z, blank| probe_word(z, blank, bit));
                ctx.wait(1000)
            }
            OFF => {
                ctx.cube.clear();
                ctx.cube.show();
                ctx.wait(1000)
            }
            _ => quadrum_play(&mut ctx),
        };
        let next = match r {
            // a '@keyword' pick, or button 1 / 'n' / '@next': the next mode
            Err(Next) => Some(ctx.jump.take().unwrap_or(if ctx.mode >= MODES { 0 } else { (ctx.mode + 1) % MODES })),
            // the light shows and animations play one after another, round and round, unless a '@keyword'
            // picked one (then it repeats until '@auto')
            Ok(()) if ctx.mode < SHOWS && !ctx.hold => Some((ctx.mode + 1) % SHOWS),
            // the demo and the Quadrum player repeat
            Ok(()) => None,
        };
        if let Some(m) = next {
            ctx.mode = m;
            ctx.cube.clear();
            ctx.cube.show();
            say!("animation: {}{}\r\n", mode_name(m), if ctx.hold && m < SHOWS { "  (repeating; @auto to cycle)" } else { "" });
        }
    }
}

/// Light shows + animations (they play in a loop), then the demo and the Quadrum player.
const SHOWS: u8 = lightshow::COUNT + animations::COUNT;
const DEMO: u8 = SHOWS;
/// Number of modes the button cycles through.
const MODES: u8 = SHOWS + 2;
/// '@off': all LEDs dark (not in the button cycle).
const OFF: u8 = 255;
/// '@probe': a fixed test pattern for finding wiring faults (not in the button cycle).
const PROBE: u8 = 254;

/// The probe test pattern. Every column bit and the unused bits (2, 18) HIGH = off; the layers multiplex as
/// usual. With a bit given, that one bit is flipped: a column bit then lights its whole column.
fn probe_word(z: usize, blank: bool, bit: Option<u8>) -> u32 {
    let base = !cube::ALL_LAYERS; // all columns + unused bits HIGH, all layers LOW
    if blank {
        return base;
    }
    let w = base | cube::LBITS[z];
    match bit {
        Some(b) => w ^ (1 << b),
        None => w,
    }
}

/// Say what '@probe' is showing and where that bit is on the 595 chain.
fn describe_probe(bit: Option<u8>) {
    let Some(b) = bit else {
        out("probe all: every column OFF, unused bits 2 and 18 HIGH too, layers multiplexing.\r\n");
        out("  Anything still lit is held on by the hardware (or wired to a bit the map doesn't know).\r\n");
        out("  Compare with @off (same, but bits 2 and 18 LOW). @probe <bit> lights one bit. @reset to leave.\r\n");
        return;
    };
    // MSB first: bit b ends at chain position b = chip b/8 (A = first, fed by GP3), output QA..QH.
    const PINS: [u8; 8] = [15, 1, 2, 3, 4, 5, 6, 7];
    let chip = (b'A' + b / 8) as char;
    let q = (b'A' + b % 8) as char;
    let pin = PINS[(b % 8) as usize];
    if let Some(i) = cube::XY_MAP.iter().position(|&m| m == b) {
        say!("probe bit {}: column x={} y={} (index {}) LOW = lit | 595 chip {} output Q{} (pin {})\r\n",
            b, i % N, i / N, i, chip, q, pin);
    } else if let Some(z) = cube::LAYER_MAP.iter().position(|&m| m == b) {
        say!("probe bit {}: layer {} (flipped: that layer goes dark) | 595 chip {} output Q{} (pin {})\r\n",
            b, z, chip, q, pin);
    } else {
        say!("probe bit {}: not in the bit map (unused) driven LOW | 595 chip {} output Q{} (pin {})\r\n",
            b, chip, q, pin);
    }
    out("  @reset to leave the probe.\r\n");
}

fn mode_name(mode: u8) -> &'static str {
    match mode {
        m if m < lightshow::COUNT => lightshow::NAMES[m as usize],
        m if m < SHOWS => animations::NAMES[(m - lightshow::COUNT) as usize],
        DEMO => "CubeLib demo",
        OFF => "off",
        PROBE => "probe (test pattern)",
        _ => "Quadrum TTED.txt",
    }
}

/// '@list': every preset with its number, the one playing marked with '>'.
fn list_modes(current: u8, hold: bool) {
    out("\r\npresets (type @name or @number, Enter):\r\n");
    for m in 0..MODES {
        say!("{} {:2}  {}\r\n", if m == current { '>' } else { ' ' }, m, mode_name(m));
    }
    out("also: @auto, @next, @prev, @off, @on, @bright, @dim, @list, @speed [MHz|jason|default], @tempo [percent], @jason, @reset\r\n");
    say!("now: {} ({})\r\n", mode_name(current), if hold { "repeating" } else { "auto" });
}

fn quadrum_play<B: InputPin>(c: &mut Ctx<B>) -> Result<(), Next> {
    if c.anim.frames == 0 {
        return c.wait(500);
    }
    for f in 0..c.anim.frames {
        let ms = c.anim.frame(f, &mut c.cube.data);
        c.frame(ms)?;
    }
    Ok(())
}

/// The CubeLibDemo.py animations, step for step. Every sleep_ms() becomes frame(): show the picture, then wait.
fn demo<B: InputPin>(c: &mut Ctx<B>) -> Result<(), Next> {
    for x in 0..5 {
        for y in 0..5 {
            for z in 0..5 {
                c.cube.dim(1);
                c.cube.set(x, y, z, 6);
                c.frame(60)?;
            }
        }
    }
    c.cube.clear();
    for y in 0..5 {
        for z in 0..5 {
            for x in 0..5 {
                c.cube.dim(1);
                c.cube.set(x, y, z, 6);
                c.frame(30)?;
            }
        }
    }
    c.cube.clear();
    for z in 0..5 {
        for x in 0..5 {
            for y in 0..5 {
                c.cube.dim(1);
                c.cube.set(x, y, z, 6);
                c.frame(20)?;
            }
        }
    }
    c.cube.clear();

    for _ in 0..8 {
        for z in 0..5 {
            plane_z(c, z);
            c.frame(40)?;
            c.cube.dim(2);
        }
        c.cube.clear();
        for z in (0..5).rev() {
            plane_z(c, z);
            c.frame(40)?;
            c.cube.dim(2);
        }
        c.cube.clear();
    }
    for _ in 0..8 {
        for x in 0..5 {
            plane_x(c, x);
            c.frame(50)?;
            c.cube.dim(3);
        }
        c.cube.clear();
        for x in (0..5).rev() {
            plane_x(c, x);
            c.frame(50)?;
            c.cube.dim(3);
        }
        c.cube.clear();
    }
    for _ in 0..8 {
        for y in 0..5 {
            plane_y(c, y);
            c.frame(40)?;
            c.cube.dim(3);
        }
        c.cube.clear();
        for y in (0..5).rev() {
            plane_y(c, y);
            c.frame(40)?;
            c.cube.dim(3);
        }
        c.cube.clear();
    }

    for i in 0..12u8 {
        c.cube.clear();
        let bright = i / 2;
        for x in 0..5 {
            for y in 0..5 {
                c.cube.set(x, y, 0, bright);
                c.cube.set(x, y, 4, bright);
            }
        }
        for x in 0..5 {
            for z in 0..5 {
                c.cube.set(x, 0, z, bright);
                c.cube.set(x, 4, z, bright);
            }
        }
        for z in 0..5 {
            for y in 0..5 {
                c.cube.set(0, y, z, bright);
                c.cube.set(4, y, z, bright);
            }
        }
        c.frame(100)?;
        c.cube.clear();
        inner_shell(c, bright);
        c.frame(100)?;
        c.cube.clear();
        c.cube.set(2, 2, 2, 6);
        c.frame(300)?;
        c.cube.clear();
        inner_shell(c, bright);
        c.frame(300)?;
        c.cube.clear();

        for b in 1..10 {
            c.cube.fill(b);
            c.frame(20)?;
        }
        for b in (2..=10).rev() {
            c.cube.fill(b);
            c.frame(20)?;
        }
    }
    Ok(())
}

fn plane_z<B: InputPin>(c: &mut Ctx<B>, z: usize) {
    for x in 0..5 {
        for y in 0..5 {
            c.cube.set(x, y, z, 6);
        }
    }
}
fn plane_x<B: InputPin>(c: &mut Ctx<B>, x: usize) {
    for y in 0..5 {
        for z in 0..5 {
            c.cube.set(x, y, z, 6);
        }
    }
}
fn plane_y<B: InputPin>(c: &mut Ctx<B>, y: usize) {
    for z in 0..5 {
        for x in 0..5 {
            c.cube.set(x, y, z, 6);
        }
    }
}
fn inner_shell<B: InputPin>(c: &mut Ctx<B>, bright: u8) {
    for x in 1..4 {
        for y in 1..4 {
            c.cube.set(x, y, 1, bright);
            c.cube.set(x, y, 3, bright);
        }
    }
    for x in 0..5 {
        for z in 1..4 {
            c.cube.set(x, 1, z, bright);
            c.cube.set(x, 3, z, bright);
        }
    }
    for z in 1..4 {
        for y in 1..4 {
            c.cube.set(1, y, z, bright);
            c.cube.set(3, y, z, bright);
        }
    }
}

#[interrupt]
fn USBCTRL_IRQ() {
    critical_section::with(|cs| {
        let mut dev = USB_DEV.borrow_ref_mut(cs);
        let mut ser = SERIAL.borrow_ref_mut(cs);
        if let (Some(dev), Some(ser)) = (dev.as_mut(), ser.as_mut()) {
            if dev.poll(&mut [ser]) {
                let mut buf = [0u8; 64];
                if let Ok(n) = ser.read(&mut buf) {
                    let mut line = LINE.borrow_ref_mut(cs);
                    for &b in &buf[..n] {
                        // '@keyword' + Enter: collect the keyword (no single-letter commands meanwhile, so the
                        // 'b' in "@bounce" doesn't reboot). The main loop picks it up in Ctx::wait.
                        if line.typing {
                            match b {
                                b'\r' | b'\n' => {
                                    line.typing = false;
                                    line.ready = true;
                                }
                                0x08 | 0x7F => {
                                    line.buf.pop(); // backspace
                                }
                                _ => {
                                    let _ = line.buf.push(b);
                                }
                            }
                            continue;
                        }
                        if b == b'@' {
                            line.typing = true;
                            line.ready = false;
                            line.buf.clear();
                            continue;
                        }
                        match b {
                            b'b' | b'B' => {
                                // A running watchdog would reset the chip in the middle of flashing.
                                unsafe { (*pac::WATCHDOG::ptr()).ctrl().modify(|_, w| w.enable().clear_bit()) };
                                // Deliberate reboot: forget the overclock marker, or the next boot (after picotool's
                                // watchdog reset) would think the chip hung and refuse to overclock.
                                #[cfg(feature = "unlimited")]
                                overclock::forget();
                                hal::reboot::reboot(
                                hal::reboot::RebootKind::BootSel { picoboot_disabled: false, msd_disabled: false },
                                hal::reboot::RebootArch::Normal,
                                )
                            }
                            b'\r' | b'\n' | b' ' => {}
                            c => CMD.store(c, Ordering::Relaxed),
                        }
                    }
                }
            }
        }
    });
}

#[interrupt]
fn DMA_IRQ_0() {
    cube::passes_irq_handler();
}
