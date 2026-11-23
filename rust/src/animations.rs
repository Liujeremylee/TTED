//! Seven extra animations, the same as CoolAnimations.py: rain, ripple, spinner, fireworks, sphere pulse,
//! corkscrew and snake. In the main loop they play one after another; button 1 or 'n' skips to the next one.
//!
//! Every animation uses the same three ideas:
//!   1. `cube.dim(n)` at the start of each frame fades the old picture, so moving LEDs leave a glowing trail.
//!   2. `plot(x, y, h, brightness)` lights an LED. It takes decimals (it rounds them) and ignores points
//!      outside the cube, so the sin()/cos() maths can run off the edges safely.
//!   3. `c.frame(ms)?` shows the picture and waits (handling the button and serial commands meanwhile).

use embedded_hal::digital::InputPin;
use libm::{cosf, fabsf, roundf, sinf, sqrtf};

use super::{Ctx, Next};
use crate::cube::{LEVELS, N};

/// How many animations this file has, and their names (shown over USB serial).
pub const COUNT: u8 = 7;
pub const NAMES: [&str; COUNT as usize] = ["rain", "ripple", "spinner", "fireworks", "sphere pulse", "corkscrew", "snake"];

/// Set this to false if the rain falls UP on your cube (your layers are wired the other way round).
const TOP_IS_Z4: bool = true;

pub(crate) const FULL: i32 = LEVELS as i32; // full brightness (10)
pub(crate) const CENTRE: f32 = (N - 1) as f32 / 2.0;
pub(crate) const TAU: f32 = 2.0 * core::f32::consts::PI;

/// Play animation number `which` (0 = rain ... COUNT-1 = snake) once.
pub fn play<B: InputPin>(c: &mut Ctx<B>, which: u8) -> Result<(), Next> {
    match which {
        0 => rain(c, 8),
        1 => ripple(c, 8),
        2 => spinner(c, 8),
        3 => fireworks(c, 6),
        4 => sphere_pulse(c, 5),
        5 => corkscrew(c, 6),
        _ => snake(c, 10),
    }
}

// ------------------------------------------------ helpers ------------------------------------------------

/// Light the LED at (x, y) and height h (0 = bottom layer, 4 = top layer). Decimals are rounded, points
/// outside the cube are ignored, and an LED that is already brighter is left alone.
pub(crate) fn plot<B: InputPin>(c: &mut Ctx<B>, x: f32, y: f32, h: f32, brightness: i32) {
    let (x, y, h) = (roundf(x) as i32, roundf(y) as i32, roundf(h) as i32);
    let n = N as i32;
    if x < 0 || x >= n || y < 0 || y >= n || h < 0 || h >= n {
        return;
    }
    let (x, y, z) = (x as usize, y as usize, layer(h as usize));
    let v = brightness.clamp(0, FULL) as u8;
    if v > c.cube.get(x, y, z) {
        c.cube.set(x, y, z, v);
    }
}

/// Height (0 = bottom) to the cube's layer number z.
pub(crate) fn layer(h: usize) -> usize {
    if TOP_IS_Z4 { h } else { N - 1 - h }
}

pub(crate) fn sq(v: f32) -> f32 {
    v * v
}

/// Random whole number from 0 to n-1.
pub(crate) fn rand_below<B: InputPin>(c: &mut Ctx<B>, n: u32) -> u32 {
    c.random() % n
}

/// Random decimal from lo up to (not including) hi.
pub(crate) fn uniform<B: InputPin>(c: &mut Ctx<B>, lo: f32, hi: f32) -> f32 {
    let r = (c.random() >> 8) as f32 / 16_777_216.0; // 24 random bits -> 0.0 .. 1.0
    lo + r * (hi - lo)
}

/// The time (in microseconds) `secs` seconds from now.
pub(crate) fn deadline<B: InputPin>(c: &Ctx<B>, secs: u64) -> u64 {
    c.now_us() + secs * 1_000_000
}

// ------------------------------------------------ animations ------------------------------------------------

/// Drops fall from the top layer to the bottom, leaving fading trails.
fn rain<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut drops: heapless::Vec<[i8; 3], 16> = heapless::Vec::new(); // each drop is [x, y, height]
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(3); // old drop positions fade out: that's the trail
        if uniform(c, 0.0, 1.0) < 0.6 {
            // 60% chance of a new drop each frame
            let d = [rand_below(c, N as u32) as i8, rand_below(c, N as u32) as i8, (N - 1) as i8];
            let _ = drops.push(d);
        }
        for d in drops.iter_mut() {
            plot(c, d[0] as f32, d[1] as f32, d[2] as f32, FULL);
            d[2] -= 1; // fall one layer
        }
        drops.retain(|d| d[2] >= 0); // forget drops that hit the bottom
        c.frame(70)?;
    }
    Ok(())
}

/// A water surface with rings spreading out from the middle, like a stone dropped in a pond.
fn ripple<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut phase = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.clear();
        for x in 0..N {
            for y in 0..N {
                let (fx, fy) = (x as f32, y as f32);
                let d = sqrtf(sq(fx - CENTRE) + sq(fy - CENTRE)); // distance from the middle column
                let h = 2.0 + 2.0 * sinf(d * 1.3 - phase); // height of the water here, 0..4
                plot(c, fx, fy, h, FULL); // the surface
                plot(c, fx, fy, h - 1.0, 2); // a dim glow just under it
            }
        }
        phase += 0.35; // moving the phase makes the rings travel outwards
        c.frame(40)?;
    }
    Ok(())
}

/// A flat vertical sheet of light spinning around the middle, like a revolving door.
fn spinner<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut angle = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(4); // leaves a short ghost behind the sheet
        let (co, si) = (cosf(angle), sinf(angle));
        for i in -6..=6 {
            // walk along the sheet in half-LED steps (long enough to reach the corners)
            let r = i as f32 / 2.0;
            for h in 0..N {
                plot(c, CENTRE + r * co, CENTRE + r * si, h as f32, FULL);
            }
        }
        angle += 0.22; // radians per frame: bigger = faster
        c.frame(30)?;
    }
    Ok(())
}

/// A rocket climbs, bursts into sparks, and the sparks fall and fade under gravity.
fn fireworks<B: InputPin>(c: &mut Ctx<B>, count: u32) -> Result<(), Next> {
    for _ in 0..count {
        let x = 1.0 + rand_below(c, 3) as f32;
        let y = 1.0 + rand_below(c, 3) as f32;
        let top = 3 + rand_below(c, 2) as usize;
        for h in 0..=top {
            // the rocket climbs
            c.cube.dim(4);
            plot(c, x, y, h as f32, FULL);
            c.frame(70)?;
        }
        let mut sparks = [[0.0f32; 6]; 16]; // each spark is [x, y, h, speed x, speed y, speed up]
        for s in sparks.iter_mut() {
            let a = uniform(c, 0.0, TAU); // random direction around
            let speed = uniform(c, 0.5, 1.0);
            let up = uniform(c, -0.5, 1.0);
            *s = [x, y, top as f32, speed * cosf(a), speed * sinf(a), up];
        }
        for step in 0..10 {
            c.cube.dim(2);
            for s in sparks.iter_mut() {
                s[0] += s[3]; // move
                s[1] += s[4];
                s[2] += s[5];
                s[5] -= 0.25; // gravity pulls the spark down a bit more each frame
                plot(c, s[0], s[1], s[2], FULL - step); // sparks get dimmer as they burn out
            }
            c.frame(60)?;
        }
        for _ in 0..6 {
            // let the last sparks fade away
            c.cube.dim(2);
            c.frame(40)?;
        }
    }
    Ok(())
}

/// A glowing ball grows from the centre to the corners and shrinks back, like a heartbeat.
fn sphere_pulse<B: InputPin>(c: &mut Ctx<B>, beats: u32) -> Result<(), Next> {
    for _ in 0..beats {
        for k in 0..30 {
            let i = if k < 15 { k } else { 29 - k }; // grow (0..14), then shrink (14..0)
            let r = i as f32 * 0.25; // radius 0 to 3.5 (the corners are 3.46 away)
            for x in 0..N {
                for y in 0..N {
                    for h in 0..N {
                        let d = sqrtf(sq(x as f32 - CENTRE) + sq(y as f32 - CENTRE) + sq(h as f32 - CENTRE));
                        let b = FULL - (fabsf(d - r) * 8.0) as i32; // bright on the surface, fading either side
                        c.cube.set(x, y, layer(h), b.max(0) as u8);
                    }
                }
            }
            c.frame(25)?;
        }
    }
    Ok(())
}

/// The 16 LEDs around the outside edge of one layer, in order (used by corkscrew).
const RING: [(u8, u8); 16] = [
    (0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (4, 1), (4, 2), (4, 3),
    (4, 4), (3, 4), (2, 4), (1, 4), (0, 4), (0, 3), (0, 2), (0, 1),
];

/// Two comets chase each other around the outside of the cube, climbing as they go.
fn corkscrew<B: InputPin>(c: &mut Ctx<B>, laps: usize) -> Result<(), Next> {
    for step in 0..laps * RING.len() {
        c.cube.dim(1); // dim(1) gives a long tail
        for offset in [0, RING.len() / 2] {
            // the second comet is half a lap behind
            let i = step + offset;
            let (x, y) = RING[i % RING.len()];
            let h = (i / 4) % N; // climb one layer every 4 steps, then wrap to the bottom
            plot(c, x as f32, y as f32, h as f32, FULL);
        }
        c.frame(45)?;
    }
    Ok(())
}

/// A dot wanders randomly through the cube, one LED at a time, with a fading tail.
fn snake<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut p = [2i32; 3]; // start in the middle
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(1);
        let axis = rand_below(c, 3) as usize; // move along x, y or height
        let step = if rand_below(c, 2) == 0 { -1 } else { 1 };
        if (0..N as i32).contains(&(p[axis] + step)) {
            p[axis] += step;
        } else {
            p[axis] -= step; // hit a wall: bounce back the other way
        }
        plot(c, p[0] as f32, p[1] as f32, p[2] as f32, FULL);
        c.frame(60)?;
    }
    Ok(())
}
