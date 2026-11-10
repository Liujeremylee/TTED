//! Runtime overclocking for the `unlimited` build: the PIO runs at divider 1, so overclocking the system clock
//! overclocks the PIO.
//!
//! Every step stays at or below 1.30 V, the RP2350 regulator's built-in limit; the voltage limit is never
//! disabled. The flash (QMI) clock divider is raised first so the flash never runs faster than the 80 MHz the
//! boot ROM already uses at 240 MHz.
//!
//! Order going up: flash divider (slower) -> core voltage -> PLL. Going down: PLL -> flash divider -> voltage.
//! Jason, your retirement time has came closer.

use core::ptr::{read_volatile, write_volatile};
use rp235x_hal::pac;

pub struct Step {
    pub mhz: u32,
    vco_mhz: u32,
    post_div1: u32,
    post_div2: u32,
    /// Regulator VSEL: 11 = 1.10 V (default) ... 15 = 1.30 V (the top of the range allowed without unlocking).
    pub vsel: u32,
    /// Flash SCK = CPU / qmi_div. Kept at or below 80 MHz.
    pub qmi_div: u32,
}

/// 12 MHz crystal, REFDIV 1: VCO = 12 MHz x FBDIV (750..1600 MHz), CPU = VCO / post_div1 / post_div2.
pub const STEPS: [Step; 7] = [
    Step { mhz: 240, vco_mhz: 1440, post_div1: 6, post_div2: 1, vsel: 11, qmi_div: 3 },
    Step { mhz: 264, vco_mhz: 1584, post_div1: 6, post_div2: 1, vsel: 11, qmi_div: 4 },
    Step { mhz: 288, vco_mhz: 1440, post_div1: 5, post_div2: 1, vsel: 12, qmi_div: 4 },
    Step { mhz: 300, vco_mhz: 1500, post_div1: 5, post_div2: 1, vsel: 12, qmi_div: 4 },
    Step { mhz: 320, vco_mhz: 960, post_div1: 3, post_div2: 1, vsel: 13, qmi_div: 4 },
    Step { mhz: 360, vco_mhz: 1440, post_div1: 4, post_div2: 1, vsel: 14, qmi_div: 5 },
    Step { mhz: 400, vco_mhz: 1200, post_div1: 3, post_div2: 1, vsel: 15, qmi_div: 5 },
];

// Compile-time sanity checks on the table.
const _: () = {
    let mut i = 0;
    while i < STEPS.len() {
        let s = &STEPS[i];
        assert!(s.vco_mhz % 12 == 0 && s.vco_mhz >= 750 && s.vco_mhz <= 1600, "VCO out of range");
        assert!(s.vco_mhz / s.post_div1 / s.post_div2 == s.mhz, "PLL settings don't give the stated MHz");
        assert!(s.post_div1 >= 1 && s.post_div1 <= 7 && s.post_div2 >= 1 && s.post_div2 <= 7);
        assert!(s.vsel <= 15, "never above 1.30 V");
        assert!(s.mhz / s.qmi_div <= 80, "flash clock above 80 MHz");
        i += 1;
    }
};

pub const fn millivolts(vsel: u32) -> u32 {
    550 + 50 * vsel
}

const POWMAN_PASSWORD: u32 = 0x5AFE_0000;

/// Set the core regulator voltage (VSEL <= 15, i.e. <= 1.30 V) and wait until it is in regulation.
fn set_vreg(vsel: u32) {
    assert!(vsel <= 15);
    let p = unsafe { &*pac::POWMAN::ptr() };
    // Unlock the VREG interface (bit 13). Keep every other control bit as it is (bit 15 = RST_N must stay 1,
    // bit 8 = DISABLE_VOLTAGE_LIMIT stays 0).
    let ctrl = p.vreg_ctrl().read().bits() & 0xFFFF;
    p.vreg_ctrl().write(|w| unsafe { w.bits(POWMAN_PASSWORD | ctrl | (1 << 13)) });
    while p.vreg().read().bits() & (1 << 15) != 0 {} // UPDATE_IN_PROGRESS
    let v = p.vreg().read().bits() & 0xFFFF;
    p.vreg().write(|w| unsafe { w.bits(POWMAN_PASSWORD | (v & !(0x1F << 4)) | (vsel << 4)) });
    while p.vreg().read().bits() & (1 << 15) != 0 {}
    while p.vreg_sts().read().bits() & (1 << 4) == 0 {} // VOUT_OK: in regulation
}

pub fn vreg_vsel() -> u32 {
    let p = unsafe { &*pac::POWMAN::ptr() };
    (p.vreg().read().bits() >> 4) & 0x1F
}

/// Change the flash SCK divider. Runs from SRAM with interrupts off, touching only registers, so nothing is
/// fetched from flash while its timing changes.
#[inline(never)]
#[link_section = ".data.ram_func"]
unsafe fn write_qmi_div_ram(reg: *mut u32, div: u32) {
    let t = read_volatile(reg);
    write_volatile(reg, (t & !0xFF) | (div & 0xFF));
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
}

fn set_qmi_div(div: u32) {
    let qmi = unsafe { &*pac::QMI::ptr() };
    let reg = qmi.m0_timing().as_ptr();
    cortex_m::interrupt::free(|_| unsafe { write_qmi_div_ram(reg, div) });
}

/// Re-lock PLL_SYS at the step's frequency. clk_sys runs from the 12 MHz reference while the PLL relocks.
fn set_pll_sys(s: &Step) {
    let clk = unsafe { &*pac::CLOCKS::ptr() };
    let pll = unsafe { &*pac::PLL_SYS::ptr() };
    cortex_m::interrupt::free(|_| {
        // clk_sys -> clk_ref (glitchless mux; SELECTED is one-hot: bit 0 = clk_ref)
        clk.clk_sys_ctrl().modify(|_, w| w.src().clk_ref());
        while clk.clk_sys_selected().read().bits() != 1 {}

        // Same sequence as rp235x-hal's PLL setup: power down, set dividers, power up, wait for lock,
        // set post dividers, power up the post divider.
        pll.pwr().reset();
        pll.fbdiv_int().reset();
        pll.cs().write(|w| unsafe { w.refdiv().bits(1) });
        pll.fbdiv_int().write(|w| unsafe { w.fbdiv_int().bits((s.vco_mhz / 12) as u16) });
        pll.pwr().modify(|_, w| {
            w.pd().clear_bit();
            w.vcopd().clear_bit()
        });
        while pll.cs().read().lock().bit_is_clear() {}
        pll.prim().write(|w| unsafe {
            w.postdiv1().bits(s.post_div1 as u8);
            w.postdiv2().bits(s.post_div2 as u8)
        });
        pll.pwr().modify(|_, w| w.postdivpd().clear_bit());

        // clk_sys -> its aux source, which is PLL_SYS (set at boot); bit 1 = aux
        clk.clk_sys_ctrl().modify(|_, w| w.src().clksrc_clk_sys_aux());
        while clk.clk_sys_selected().read().bits() != 2 {}
    });
}

/// Move from step `from` to step `to`. Returns the new CPU frequency in Hz.
pub fn change(from: usize, to: usize) -> u32 {
    let (a, b) = (&STEPS[from], &STEPS[to]);
    if b.mhz > a.mhz {
        set_qmi_div(b.qmi_div.max(a.qmi_div)); // flash slower first
        set_vreg(b.vsel.max(a.vsel)); // then more voltage
        set_pll_sys(b); // then the faster clock
        set_qmi_div(b.qmi_div);
    } else if b.mhz < a.mhz {
        set_pll_sys(b); // slower clock first
        set_qmi_div(b.qmi_div);
        set_vreg(b.vsel);
    }
    // HSTX is unused; stop its clock so it never runs out of spec from PLL_SYS.
    let clk = unsafe { &*pac::CLOCKS::ptr() };
    clk.clk_hstx_ctrl().modify(|_, w| w.enable().clear_bit());
    b.mhz * 1_000_000
}

// ---- crash memory: survives a watchdog reset (scratch 0; the boot ROM uses scratch 4..7) ----
const MAGIC: u32 = 0xC10C_0000;

/// Record the step we are about to run at.
pub fn mark_attempt(step: usize) {
    let wd = unsafe { &*pac::WATCHDOG::ptr() };
    wd.scratch0().write(|w| unsafe { w.bits(MAGIC | STEPS[step].mhz) });
}

/// If the last reset was a watchdog timeout while overclocked, return that clock in MHz (and forget it).
pub fn crashed_at() -> Option<u32> {
    let wd = unsafe { &*pac::WATCHDOG::ptr() };
    let s = wd.scratch0().read().bits();
    let timer_reset = wd.reason().read().bits() & 1 != 0; // REASON.TIMER
    wd.scratch0().write(|w| unsafe { w.bits(0) });
    if timer_reset && s & 0xFFFF_0000 == MAGIC && (s & 0xFFFF) > STEPS[0].mhz {
        Some(s & 0xFFFF)
    } else {
        None
    }
}
