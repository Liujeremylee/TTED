//! 5x5x5 LED cube driver for the TTED board (HC595 chain on GP1 latch / GP2 SCK / GP3 data).
//!
//! The refresh needs no CPU time:
//!   * PIO0 SM0 shifts one 32-bit word MSB first into the 595 chain and pulses the latch: 67 PIO cycles per word.
//!   * DMA channel 0 streams a 50-word table (10 brightness slots x 5 layers) into the PIO TX FIFO.
//!     When it finishes it chains to channel 1, which writes the table address back into channel 0's
//!     READ_ADDR trigger register, restarting it. That loop runs forever.
//!   * The CPU only builds a new table when the picture changes (`Cube::show`), into a back buffer.
//!     Channel 1 reads the table address from `next_addr` on every pass, so the switch to the new table
//!     always happens between passes: a half-built picture can never be displayed.
//!
//! Word layout is identical to the MicroPython CubeLib: bit `LAYER_MAP[z]` HIGH turns layer z on;
//! bit `XY_MAP[i]` HIGH turns column i OFF. An LED with brightness v is lit in slots 0..v-1 of 10.

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{compiler_fence, Ordering};

use pio::{Instruction, InstructionOperands, JmpCondition, MovDestination, MovOperation, MovSource};
use rp235x_hal as hal;
use hal::pac;
use hal::pio::{
    Buffers, PIOBuilder, PIOExt, PinDir, PinState, Running, ShiftDirection, StateMachine, Stopped, Rx, SM0,
};

pub const N: usize = 5;
pub const LEDS: usize = N * N * N;
/// Brightness steps: 0 = off ... LEVELS = full. Values above LEVELS are also full.
pub const LEVELS: usize = 10;
/// Words in one refresh table.
pub const SLOTS: usize = LEVELS * N;

/// Column bit for each (y*5 + x) - the TTED two-sided board, Oct. 2026 (same as CubeLibDemo.py).
pub const XY_MAP: [u8; N * N] = [27, 25, 24, 19, 16, 28, 30, 20, 22, 23, 29, 31, 21, 17, 4, 13, 26, 3, 1, 0, 15, 14, 5, 6, 7];
/// Layer bit for each z.
pub const LAYER_MAP: [u8; N] = [9, 10, 11, 8, 12];

pub const LATCH_PIN: u8 = 1; // side-set bit 0
pub const SCK_PIN: u8 = 2; // side-set bit 1
pub const DATA_PIN: u8 = 3; // OUT pin

pub const DATA_CH: usize = 0;
pub const CTRL_CH: usize = 1;
const TREQ_UNPACED: u32 = 0x3F;

/// PIO cycles per table word with no extra hold: 32 bits x 2 + latch(2, the next word is pulled during it).
pub const PIO_CYCLES_FAST: u32 = 66;
/// With a hold of h: the above + mov(1) + the hold loop (h+1).
pub const fn pio_cycles(hold: u32) -> u32 {
    if hold == 0 { PIO_CYCLES_FAST } else { PIO_CYCLES_FAST + 2 + hold }
}

const fn masks() -> [u32; N * N] {
    let mut m = [0u32; N * N];
    let mut i = 0;
    while i < N * N {
        m[i] = 1 << XY_MAP[i];
        i += 1;
    }
    m
}
const fn layer_bits() -> [u32; N] {
    let mut m = [0u32; N];
    let mut i = 0;
    while i < N {
        m[i] = 1 << LAYER_MAP[i];
        i += 1;
    }
    m
}
const MASKS: [u32; N * N] = masks();
const LBITS: [u32; N] = layer_bits();

// Every bit position must be used once at most and fit in 32 bits (checked at compile time).
const _: () = {
    let mut seen = 0u64;
    let mut i = 0;
    while i < N * N + N {
        let b = if i < N * N { XY_MAP[i] } else { LAYER_MAP[i - N * N] };
        assert!(b < 32, "bit map entry out of range");
        assert!(seen & (1u64 << b) == 0, "bit map entry used twice");
        seen |= 1u64 << b;
        i += 1;
    }
};

/// Build a refresh table: word[s*N + z] = layer bit | column bits of every LED in layer z with value <= s.
/// One pass buckets each LED's column bit by its value; a running OR over the slots then gives every slot.
/// That is N*N + LEVELS steps per layer instead of N*N*LEVELS.
///
/// Per LED this is 4 instructions: ldrb v / ldr tmp[v] / orr #mask / str tmp[v]:
///   * the 25 LEDs are fully unrolled, so every column mask (a single bit) is an instruction immediate,
///     never a load from flash;
///   * `tmp` has 256 entries, so any u8 brightness is a valid index and no clamp is needed.
///     LEDs at full brightness (v >= LEVELS) land in tmp[LEVELS..], which is never read.
///   * `tmp` is a long-lived scratch buffer: tmp[0..LEVELS] must be zero on entry, and each entry is
///     re-zeroed as it is consumed, so it is zero again on exit. No clearing loop, no memset call.
#[inline(always)]
fn build_body(data: &[u8; LEDS], out: &mut [u32; SLOTS], tmp: &mut Scratch) {
    macro_rules! bucket {
        ($tmp:ident, $layer:ident; $($i:literal)*) => { $( $tmp[$layer[$i] as usize] |= MASKS[$i]; )* };
    }
    let tmp = &mut tmp.0;
    for z in 0..N {
        let layer: &[u8; N * N] = data[z * N * N..(z + 1) * N * N].try_into().unwrap();
        bucket!(tmp, layer; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24);
        let lb = LBITS[z];
        let mut acc = 0u32;
        for s in 0..LEVELS {
            acc |= tmp[s];
            tmp[s] = 0;
            out[s * N + z] = acc | lb;
        }
    }
}
const _: () = assert!(N * N == 25, "the unrolled bucket! list above is written for 25 LEDs per layer");

/// Scratch for the table builder. Create it zeroed (`Scratch::new()`); the builder keeps tmp[0..LEVELS] zero.
pub struct Scratch([u32; 256]);
impl Scratch {
    pub const fn new() -> Self {
        Scratch([0; 256])
    }
}

/// The table builder used by `show()` (runs from flash, through the XIP cache).
#[inline(never)]
pub fn build(data: &[u8; LEDS], out: &mut [u32; SLOTS], tmp: &mut Scratch) {
    build_body(data, out, tmp)
}

/// The same code placed in SRAM (copied there at boot), to measure whether flash fetches cost anything.
#[inline(never)]
#[link_section = ".data.ram_func"]
pub fn build_ram(data: &[u8; LEDS], out: &mut [u32; SLOTS], tmp: &mut Scratch) {
    build_body(data, out, tmp)
}

/// First version (kept for the before/after measurement only).
#[inline(never)]
pub fn build_v1(data: &[u8; LEDS], out: &mut [u32; SLOTS]) {
    for z in 0..N {
        let mut tmp = [0u32; LEVELS + 1]; // tmp[LEVELS] collects "always on" LEDs and is never used
        let layer = &data[z * N * N..(z + 1) * N * N];
        for i in 0..N * N {
            let v = (layer[i] as usize).min(LEVELS);
            tmp[v] |= MASKS[i];
        }
        let mut acc = 0u32;
        for s in 0..LEVELS {
            acc |= tmp[s];
            out[s * N + z] = acc | LBITS[z];
        }
    }
}

/// The original CubeLib.display_layer() algorithm, used only to check `build`.
pub fn reference_word(data: &[u8; LEDS], slot: usize, z: usize) -> u32 {
    let mut bits = 1u32 << LAYER_MAP[z];
    for i in 0..N * N {
        if data[z * N * N + i] as usize <= slot {
            bits |= 1 << XY_MAP[i];
        }
    }
    bits
}

/// Three tables (triple buffering) with one spare word after each, so their DMA address ranges can never
/// touch (the DMA's READ_ADDR is one past the end when a pass finishes).
const NTABLES: usize = 3;
#[repr(C, align(4))]
struct Tables([[u32; SLOTS + 1]; NTABLES]);

type TxSm = StateMachine<(pac::PIO0, SM0), Running>;
type CapSm = (pac::PIO1, SM0);

pub struct Cube {
    /// Brightness of every LED, index = z*25 + y*5 + x. Call `show()` to display it.
    pub data: [u8; LEDS],
    tables: Tables,
    /// Table-builder scratch (see `build`).
    tmp: Scratch,
    /// Read by DMA channel 1 at the end of every pass: the table shown next.
    next_addr: u32,
    /// The most recently built table.
    latest: usize,
    _sm: Option<TxSm>,
    cap_sm: Option<StateMachine<CapSm, Stopped>>,
    cap_rx: Option<Rx<CapSm>>,
    cap_offset: u8,
    pub sys_hz: u32,
    pub pio_div: u16,
    pub hold: u32,
}

impl Cube {
    pub const fn new() -> Self {
        Cube {
            data: [0; LEDS],
            tables: Tables([[0; SLOTS + 1]; NTABLES]),
            tmp: Scratch::new(),
            next_addr: 0,
            latest: 0,
            _sm: None,
            cap_sm: None,
            cap_rx: None,
            cap_offset: 0,
            sys_hz: 0,
            pio_div: 0,
            hold: 0,
        }
    }

    #[inline]
    fn index(x: usize, y: usize, z: usize) -> usize {
        z * N * N + y * N + x
    }
    pub fn set(&mut self, x: usize, y: usize, z: usize, v: u8) {
        if x < N && y < N && z < N {
            self.data[Self::index(x, y, z)] = v;
        }
    }
    #[allow(dead_code)]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u8 {
        self.data[Self::index(x, y, z)]
    }
    pub fn clear(&mut self) {
        self.data = [0; LEDS];
    }
    pub fn fill(&mut self, v: u8) {
        self.data = [v; LEDS];
    }
    pub fn dim(&mut self, step: u8) {
        for d in self.data.iter_mut() {
            *d = d.saturating_sub(step);
        }
    }

    fn table_addr(&self, i: usize) -> u32 {
        addr_of!(self.tables.0[i]) as u32
    }
    /// The most recently built table (for the self-test).
    pub fn latest_table(&self) -> &[u32] {
        &self.tables.0[self.latest][..SLOTS]
    }

    fn table_of(&self, addr: u32) -> Option<usize> {
        (0..NTABLES).find(|&i| addr == self.table_addr(i))
    }

    /// Which table the DMA is streaming right now. READ_ADDR in [start, end) of a table means that table.
    /// READ_ADDR == end means a pass just finished and channel 1 is (within a few bus cycles) loading the
    /// next address: wait that out, so the answer is never ambiguous.
    fn current_table(&self) -> usize {
        if self._sm.is_none() {
            return self.latest;
        }
        let dma = unsafe { &*pac::DMA::ptr() };
        loop {
            let ra = dma.ch(DATA_CH).ch_read_addr().read().bits();
            for i in 0..NTABLES {
                let lo = self.table_addr(i);
                if ra >= lo && ra < lo + (SLOTS as u32) * 4 {
                    return i;
                }
            }
        }
    }

    /// Build the picture in `data` and queue it; the DMA switches to it at the end of the current pass.
    /// Never waits for the DMA: with three tables there is always one that is neither being displayed nor
    /// queued, and only that one is written. If show() is called again before the queued table was reached,
    /// the newer picture simply takes its place in the queue.
    pub fn show(&mut self) {
        let cur = self.current_table();
        let queued = self.table_of(unsafe { read_volatile(addr_of!(self.next_addr)) }).unwrap_or(cur);
        let target = (0..NTABLES).find(|&i| i != cur && i != queued).unwrap();
        {
            let t: &mut [u32; SLOTS] = (&mut self.tables.0[target][..SLOTS]).try_into().unwrap();
            build_ram(&self.data, t, &mut self.tmp);
        }
        // The table must be in memory before the DMA (another bus master) can see its address.
        compiler_fence(Ordering::SeqCst);
        cortex_m::asm::dmb();
        unsafe { write_volatile(addr_of_mut!(self.next_addr), self.table_addr(target)) };
        self.latest = target;
    }

    /// Wait until the DMA is streaming the most recently built table (at most ~2 passes, ~170 us).
    pub fn wait_shown(&self) {
        while self.current_table() != self.latest {}
    }

    /// Start the PIO + DMA refresh. `hold` = extra PIO cycles per slot (0 = as fast as the 595 clock allows).
    pub fn start(&mut self, pio0: pac::PIO0, pio1: pac::PIO1, resets: &mut pac::RESETS, sys_hz: u32, max_sck_hz: u32, hold: u32) {
        self.sys_hz = sys_hz;
        self.hold = hold;
        // PIO clock = sys / integer divider (no jitter), at most 2 x the 595 clock.
        let div = (sys_hz.div_ceil(2 * max_sck_hz) as u16).max(1);
        self.pio_div = div;

        // Show the current picture from the very first pass.
        let t: &mut [u32; SLOTS] = (&mut self.tables.0[0][..SLOTS]).try_into().unwrap();
        build(&self.data, t, &mut self.tmp);
        self.latest = 0;
        self.next_addr = self.table_addr(0);

        // ---------- PIO0: the 595 shifter ----------
        let (mut pio, sm0, _, _, _) = pio0.split(resets);
        // side-set bit0 = latch (GP1), bit1 = SCK (GP2).  Data changes with SCK low, is clocked on SCK rising,
        // and the latch rises one PIO cycle after the last SCK edge and stays high for 2 cycles -
        // exactly the edge timing of the MicroPython PIO version. The next word is pulled *during* the
        // latch pulse, so the pull costs no extra cycle: 64 + 2 = 66 cycles per word.
        // The first instruction runs once (outside the wrap) so the very first latch only ever happens
        // after a complete 32-bit word.
        let program = if hold == 0 {
            pio::pio_asm!(
                ".side_set 2",
                "    pull block        side 0b00",
                ".wrap_target",
                "bitloop:",
                "    out pins, 1       side 0b00",
                "    jmp !osre bitloop side 0b10",
                "    pull block        side 0b01 [1]",
                ".wrap",
            )
            .program
        } else {
            pio::pio_asm!(
                ".side_set 2",
                "    pull block        side 0b00",
                ".wrap_target",
                "bitloop:",
                "    out pins, 1       side 0b00",
                "    jmp !osre bitloop side 0b10",
                "    pull block        side 0b01 [1]",
                "    mov y, isr        side 0b00",
                "hold:",
                "    jmp y-- hold      side 0b00",
                ".wrap",
            )
            .program
        };
        let installed = pio.install(&program).unwrap();
        let (mut sm, _rx, mut tx) = PIOBuilder::from_installed_program(installed)
            .out_pins(DATA_PIN, 1)
            .side_set_pin_base(LATCH_PIN)
            .out_shift_direction(ShiftDirection::Left) // MSB first, like the old SPI code
            .autopull(false)
            .pull_threshold(0) // 0 encodes 32
            .buffers(Buffers::OnlyTx) // 8-word FIFO
            .clock_divisor_fixed_point(div, 0)
            .build(sm0);
        sm.set_pins([(LATCH_PIN, PinState::Low), (SCK_PIN, PinState::Low), (DATA_PIN, PinState::Low)]);
        sm.set_pindirs([(LATCH_PIN, PinDir::Output), (SCK_PIN, PinDir::Output), (DATA_PIN, PinDir::Output)]);
        if hold != 0 {
            // Park the hold count in the ISR (unused otherwise): pull block; mov isr, osr.
            // side 0b00 is mandatory with this program's side-set and keeps latch + SCK low.
            // The pull can't stall: the word is already in the FIFO.
            tx.write(hold);
            sm.exec_instruction(Instruction {
                operands: InstructionOperands::PULL { if_empty: false, block: true },
                delay: 0,
                side_set: Some(0),
            });
            sm.exec_instruction(Instruction {
                operands: InstructionOperands::MOV {
                    destination: MovDestination::ISR,
                    op: MovOperation::None,
                    source: MovSource::OSR,
                },
                delay: 0,
                side_set: Some(0),
            });
        }
        let txf = tx.fifo_address() as u32;
        let dreq = tx.dreq_value() as u32;

        // ---------- DMA loop ----------
        let dma = unsafe { &*pac::DMA::ptr() };
        let ctrl_word = |chain_to: usize, treq: u32, incr_read: bool, quiet: bool| -> u32 {
            1                                   // EN
                | (2 << 2)                      // DATA_SIZE = word
                | ((incr_read as u32) << 4)     // INCR_READ
                | ((chain_to as u32) << 13)     // CHAIN_TO
                | (treq << 17)                  // TREQ_SEL
                | ((quiet as u32) << 23)        // IRQ_QUIET
        };
        let c = dma.ch(CTRL_CH);
        let d = dma.ch(DATA_CH);
        // Channel 1: copy next_addr into channel 0's READ_ADDR trigger alias. Chains to itself = no chain.
        // Not quiet: it raises its INTR bit once per pass, which `measure_passes` counts (when enabled).
        c.ch_read_addr().write(|w| unsafe { w.bits(addr_of!(self.next_addr) as u32) });
        c.ch_write_addr().write(|w| unsafe { w.bits(d.ch_al3_read_addr_trig().as_ptr() as u32) });
        c.ch_trans_count().write(|w| unsafe { w.bits(1) });
        c.ch_al1_ctrl().write(|w| unsafe { w.bits(ctrl_word(CTRL_CH, TREQ_UNPACED, false, false)) });
        // Channel 0: table -> PIO TX FIFO, paced by the PIO, chains to channel 1.
        d.ch_read_addr().write(|w| unsafe { w.bits(self.next_addr) });
        d.ch_write_addr().write(|w| unsafe { w.bits(txf) });
        d.ch_trans_count().write(|w| unsafe { w.bits(SLOTS as u32) });
        d.ch_al1_ctrl().write(|w| unsafe { w.bits(ctrl_word(CTRL_CH, dreq, true, true)) });

        // Read the configuration back through the named fields: catches any bit-position mistake.
        let r = d.ch_ctrl_trig().read();
        assert!(r.data_size().bits() == 2 && r.incr_read().bit() && !r.incr_write().bit());
        assert!(r.chain_to().bits() as usize == CTRL_CH && r.treq_sel().bits() as u32 == dreq);
        let r = c.ch_ctrl_trig().read();
        assert!(r.chain_to().bits() as usize == CTRL_CH && r.treq_sel().bits() as u32 == TREQ_UNPACED);
        assert!(!r.incr_read().bit() && !r.incr_write().bit() && !r.irq_quiet().bit());

        let sm = sm.start();
        // Trigger channel 0 (writing READ_ADDR through the trigger alias starts it).
        d.ch_al3_read_addr_trig().write(|w| unsafe { w.bits(self.next_addr) });
        self._sm = Some(sm);

        // ---------- PIO1: logic-analyser self-test (reads GP1..3, never drives them) ----------
        let (mut pio1, p1sm0, _, _, _) = pio1.split(resets);
        let cap = pio::pio_asm!(
            "    wait 1 gpio 1", // align to the end of a latch pulse
            "    wait 0 gpio 1",
            ".wrap_target",
            "    set x, 31",
            "bit:",
            "    wait 0 gpio 2",
            "    wait 1 gpio 2", // SCK rising edge: the moment the 595 samples data
            "    in pins, 1",
            "    jmp x-- bit",
            "    wait 1 gpio 1", // a latch pulse must follow every 32 bits, or the capture desyncs
            "    wait 0 gpio 1",
            ".wrap",
        );
        let installed = pio1.install(&cap.program).unwrap();
        self.cap_offset = installed.offset();
        let (csm, crx, _) = PIOBuilder::from_installed_program(installed)
            .in_pin_base(DATA_PIN)
            .in_shift_direction(ShiftDirection::Left)
            .autopush(true)
            .push_threshold(0)
            .buffers(Buffers::OnlyRx)
            .clock_divisor_fixed_point(1, 0)
            .build(p1sm0);
        self.cap_sm = Some(csm);
        self.cap_rx = Some(crx);
        core::mem::forget(pio);
        core::mem::forget(pio1);
    }

    /// Change the PIO clock divider while running (595 clock = sys / div / 2). Takes effect immediately.
    pub fn set_pio_div(&mut self, div: u16) {
        if let Some(sm) = self._sm.as_mut() {
            sm.clock_divisor_fixed_point(div, 0);
            self.pio_div = div;
        }
    }

    /// Capture `out.len()` words straight off the GP1..3 pins with PIO1, timestamping each with the CPU
    /// cycle counter. Returns the number captured (fewer on timeout).
    pub fn capture(&mut self, out: &mut [u32], stamps: &mut [u32]) -> usize {
        let mut sm = self.cap_sm.take().unwrap();
        let rx = self.cap_rx.as_mut().unwrap();
        sm.clear_fifos();
        // mov isr, null: empty the ISR and reset its shift count, so the first word is aligned
        sm.exec_instruction(Instruction {
            operands: InstructionOperands::MOV { destination: MovDestination::ISR, op: MovOperation::None, source: MovSource::NULL },
            delay: 0,
            side_set: None,
        });
        // jmp to the program start (the latch sync)
        sm.exec_instruction(Instruction {
            operands: InstructionOperands::JMP { condition: JmpCondition::Always, address: self.cap_offset },
            delay: 0,
            side_set: None,
        });
        let running = sm.start();
        let timeout = self.sys_hz / 100; // 10 ms
        let n = cortex_m::interrupt::free(|_| {
            let t0 = cortex_m::peripheral::DWT::cycle_count();
            let mut got = 0;
            while got < out.len() {
                if let Some(w) = rx.read() {
                    stamps[got] = cortex_m::peripheral::DWT::cycle_count();
                    out[got] = w;
                    got += 1;
                } else if cortex_m::peripheral::DWT::cycle_count().wrapping_sub(t0) > timeout {
                    break;
                }
            }
            got
        });
        self.cap_sm = Some(running.stop());
        n
    }
}

/// Count completed table passes over `ms` milliseconds via channel 1's interrupt.
pub static PASSES: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

pub fn passes_irq_enable(on: bool) {
    let dma = unsafe { &*pac::DMA::ptr() };
    dma.ints0().write(|w| unsafe { w.bits(1 << CTRL_CH) }); // clear any stale flag
    dma.inte0().modify(|r, w| unsafe {
        w.bits(if on { r.bits() | (1 << CTRL_CH) } else { r.bits() & !(1 << CTRL_CH) })
    });
}

pub fn passes_irq_handler() {
    let dma = unsafe { &*pac::DMA::ptr() };
    if dma.ints0().read().bits() & (1 << CTRL_CH) != 0 {
        dma.ints0().write(|w| unsafe { w.bits(1 << CTRL_CH) });
        PASSES.fetch_add(1, Ordering::Relaxed);
    }
}
