//! TTED 5x5x5 LED cube firmware for the Raspberry Pi Pico 2 (RP2350), in Rust.
//!
//! The LEDs are refreshed by hardware (PIO0 + a looping DMA chain): the CPU executes zero instructions per
//! layer. See cube.rs. Animations: the CubeLibDemo.py sequences and the Quadrum player (TTED.txt).
//!
//! Button 1 (GP13) or 'n' over USB serial: next animation.
//! USB serial commands: m = measure & report, t = output self-test, b = reboot to BOOTSEL (for flashing), h = help.

#![no_std]
#![no_main]

mod cube;
mod quadrum;

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

use cube::{Cube, LEDS, N, SLOTS};

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
const MAX_SCK_HZ: u32 = 20_000_000;
/// Extra PIO cycles per slot. 0 = fastest. Raise it if you see ghosting between layers.
const HOLD: u32 = 0;
/// Limits for the '+'/'-' 595 clock commands: CPU/3/2 = 40 MHz ... CPU/24/2 = 5 MHz.
const MIN_DIV: u16 = 3;
const MAX_DIV: u16 = 24;

static USB_DEV: Mutex<RefCell<Option<UsbDevice<'static, hal::usb::UsbBus>>>> = Mutex::new(RefCell::new(None));
static SERIAL: Mutex<RefCell<Option<SerialPort<'static, hal::usb::UsbBus>>>> = Mutex::new(RefCell::new(None));
static CMD: AtomicU8 = AtomicU8::new(0);

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
    button: B,
    btn_down: bool,
    mode: u8,
    rng: u32,
    boot_test: Option<bool>,
}

impl<B: InputPin> Ctx<'_, B> {
    fn now_us(&self) -> u64 {
        self.timer.get_counter().ticks()
    }

    /// Display the current picture, then wait.
    fn frame(&mut self, ms: u32) -> Result<(), Next> {
        self.cube.show();
        self.wait(ms)
    }

    /// Wait `ms`, handling serial commands and button 1 meanwhile.
    fn wait(&mut self, ms: u32) -> Result<(), Next> {
        let start = self.now_us();
        loop {
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
                        pio_hz as f32 / 2e6, new, pio_hz / (cyc * SLOTS as u32),
                        if new < 6 { "  [above the 20 MHz the board was designed for: check the cube carefully]" } else { "" });
                }
                b'h' | b'H' | b'?' => help(),
                c => say!("unknown command '{}' (h = help)\r\n", c as char),
            }
            let pressed = self.button.is_low().unwrap_or(false);
            if pressed && !self.btn_down {
                self.btn_down = true;
                return Err(Next);
            }
            if !pressed {
                self.btn_down = false;
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
        for i in 0..LEDS {
            self.cube.data[i] = (self.random() % 13) as u8; // 0..12, including values above full brightness
        }
        self.cube.show();
        self.cube.wait_shown(); // the capture below must see this table
        let data = self.cube.data;
        let table: [u32; SLOTS] = self.cube.latest_table().try_into().unwrap();

        let mut bad_words = 0;
        for s in 0..cube::LEVELS {
            for z in 0..N {
                if table[s * N + z] != cube::reference_word(&data, s, z) {
                    bad_words += 1;
                }
            }
        }

        let mut cap = [0u32; 3 * SLOTS];
        let mut st = [0u32; 3 * SLOTS];
        let got = self.cube.capture(&mut cap, &mut st);
        let mut offset = None;
        if got == cap.len() {
            for k in 0..SLOTS {
                if (0..got).all(|i| cap[i] == table[(k + i) % SLOTS]) {
                    offset = Some(k);
                    break;
                }
            }
        }
        self.cube.data = saved;
        self.cube.show();

        let ok = bad_words == 0 && offset.is_some();
        if verbose {
            say!("self-test: table vs original algorithm: {} of {} words differ\r\n", bad_words, SLOTS);
            match offset {
                Some(_) => say!("self-test: {} words captured on GP1-3 = table, bit for bit: PASS\r\n", got),
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

    fn report(&mut self) {
        let sys = self.cube.sys_hz;
        let div = self.cube.pio_div as u32;
        let pio_hz = sys / div;
        let cyc = cube::pio_cycles(self.cube.hold);
        say!("\r\n=== TTED LED cube - Rust, PIO + DMA refresh ===\r\n");
        say!("CPU {} MHz | PIO {} MHz (CPU/{}) | 595 shift clock {} MHz\r\n", sys / 1_000_000, pio_hz / 1_000_000, div, pio_hz / 2_000_000);
        say!("PIO program: {} instructions, {} PIO cycles per slot (32 bits x 2 + 2 latch; next word pulled during the latch)\r\n",
            if self.cube.hold == 0 { 4 } else { 6 }, cyc);
        say!("CPU work per slot / layer / refresh: 0 instructions (DMA feeds the PIO)\r\n");
        say!("expected: slot {} ns | each layer refreshed {}/s | {} full PWM cycles/s\r\n",
            cyc as u64 * 1_000_000_000 / pio_hz as u64, pio_hz / (cyc * N as u32), pio_hz / (cyc * SLOTS as u32));

        // Measured: complete table passes per second, counted by DMA channel 1's interrupt.
        cube::PASSES.store(0, Ordering::Relaxed);
        let t0 = self.now_us();
        cube::passes_irq_enable(true);
        self.timer.delay_ms(1000);
        cube::passes_irq_enable(false);
        let us = self.now_us() - t0;
        let passes = cube::PASSES.load(Ordering::Relaxed) as u64 * 1_000_000 / us;
        // Each pass shows every layer once per brightness slot (LEVELS times).
        say!("measured: {} full PWM cycles/s = {} slots/s, each layer refreshed {}/s\r\n",
            passes, passes * SLOTS as u64, passes * cube::LEVELS as u64);

        // Measured: time between words on the pins (PIO1 capture, CPU cycle counter).
        let mut cap = [0u32; 3 * SLOTS];
        let mut st = [0u32; 3 * SLOTS];
        let got = self.cube.capture(&mut cap, &mut st);
        if got > 1 {
            let span = st[got - 1].wrapping_sub(st[0]) as u64;
            let per = span * 1000 / (got as u64 - 1); // milli-cycles
            say!("measured on the pins: one word every {}.{:03} CPU cycles = {} ns ({} PIO cycles)\r\n",
                per / 1000, per % 1000, per * 1_000_000 / sys as u64, (per + div as u64 * 500) / (div as u64 * 1000));
        } else {
            say!("measured on the pins: capture timed out\r\n");
        }

        // Table build cost (the only CPU work, once per animation frame).
        let mut data = [0u8; LEDS];
        let mut table_out = [0u32; SLOTS];
        let mut tmp = cube::Scratch::new();
        let overhead = {
            let a = DWT::cycle_count();
            let b = DWT::cycle_count();
            b.wrapping_sub(a)
        };
        type Builder = fn(&[u8; LEDS], &mut [u32; SLOTS], &mut cube::Scratch);
        fn v1(d: &[u8; LEDS], o: &mut [u32; SLOTS], _t: &mut cube::Scratch) {
            cube::build_v1(d, o)
        }
        #[inline(never)]
        fn empty(_d: &[u8; LEDS], _o: &mut [u32; SLOTS], _t: &mut cube::Scratch) {}
        let builders: [(&str, Builder); 4] = [
            ("empty fn (harness cost)", empty),
            ("v1 (first Rust)", v1),
            ("v2 from flash  ", cube::build),
            ("v2 from SRAM   ", cube::build_ram),
        ];
        let mut check = [0u32; SLOTS];
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
}

#[hal::entry]
fn main() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let mut cp = cortex_m::Peripherals::take().unwrap();

    // ---- clocks: 240 MHz system, 48 MHz USB ----
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
    let _latch = pins.gpio1.into_function::<hal::gpio::FunctionPio0>();
    let _sck = pins.gpio2.into_function::<hal::gpio::FunctionPio0>();
    let _data = pins.gpio3.into_function::<hal::gpio::FunctionPio0>();
    let button1 = pins.gpio13.into_pull_up_input();
    let _button2 = pins.gpio10.into_pull_up_input();
    let _button3 = pins.gpio11.into_pull_up_input();
    let _button4 = pins.gpio12.into_pull_up_input();

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

    let mut ctx = Ctx {
        cube,
        anim,
        anim_err,
        timer,
        button: button1,
        btn_down: false,
        mode: 0,
        rng: 0x1234_5678 ^ (timer.get_counter().ticks() as u32),
        boot_test: None,
    };
    if ctx.rng == 0 {
        ctx.rng = 1;
    }
    ctx.boot_test = Some(ctx.selftest(false));

    loop {
        let r = match ctx.mode {
            0 => demo(&mut ctx),
            _ => quadrum_play(&mut ctx),
        };
        if r.is_err() {
            ctx.mode = (ctx.mode + 1) % 2;
            ctx.cube.clear();
            ctx.cube.show();
            say!("animation: {}\r\n", if ctx.mode == 0 { "CubeLib demo" } else { "Quadrum TTED.txt" });
        }
    }
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
                    for &b in &buf[..n] {
                        match b {
                            b'b' | b'B' => hal::reboot::reboot(
                                hal::reboot::RebootKind::BootSel { picoboot_disabled: false, msd_disabled: false },
                                hal::reboot::RebootArch::Normal,
                            ),
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
