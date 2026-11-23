//! Light shows: eight effects that use the 10 brightness levels and the fast refresh. They play one after
//! another (then the animations.rs ones); button 1 or 'n' skips to the next one.
//!
//! Same building blocks as animations.rs: `c.cube.set()` / `plot()` to light LEDs, `c.cube.dim(n)` for
//! fading trails, `c.frame(ms)?` to show the picture and wait.

use embedded_hal::digital::InputPin;
use libm::{cosf, fabsf, sinf, sqrtf};

use super::{Ctx, Next};
use crate::animations::{deadline, layer, plot, rand_below, sq, uniform, CENTRE, FULL, TAU};
use crate::cube::N;

/// How many light shows this file has, and their names (shown over USB serial).
pub const COUNT: u8 = 8;
pub const NAMES: [&str; COUNT as usize] =
    ["plasma", "twinkle", "plane sweep", "shells", "double helix", "orbits", "diagonal wave", "bouncing balls"];

/// Play light show number `which` (0 = plasma ... COUNT-1 = bouncing balls) once.
pub fn play<B: InputPin>(c: &mut Ctx<B>, which: u8) -> Result<(), Next> {
    match which {
        0 => plasma(c, 12),
        1 => twinkle(c, 8),
        2 => sweep(c, 2),
        3 => shells(c, 8),
        4 => helix(c, 10),
        5 => orbits(c, 12),
        6 => diagonal(c, 8),
        _ => bounce(c, 12),
    }
}

/// 0.0 ..= 1.0 to a brightness level 0 ..= 10.
fn level(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * FULL as f32 + 0.5) as u8
}

/// Call `f(x, y, h)` for every LED (h = height, 0 = bottom) and set it to the brightness it returns (0..1).
fn paint<B: InputPin>(c: &mut Ctx<B>, mut f: impl FnMut(f32, f32, f32) -> f32) {
    for x in 0..N {
        for y in 0..N {
            for h in 0..N {
                let v = f(x as f32, y as f32, h as f32);
                c.cube.set(x, y, layer(h), level(v));
            }
        }
    }
}

// ------------------------------------------------ shows ------------------------------------------------

/// Soft glowing blobs drifting through the cube: four sine waves (one per axis + one from the centre) added up.
fn plasma<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut t = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        paint(c, |x, y, h| {
            let d = sqrtf(sq(x - CENTRE) + sq(y - CENTRE) + sq(h - CENTRE));
            let v = sinf(x * 0.9 + t) + sinf(y * 0.8 - t * 1.3) + sinf(h * 1.1 + t * 0.7) + sinf(d * 1.5 - t * 1.6);
            (v + 0.5) / 3.5 // -4..4 -> mostly dark, with bright blobs where the waves meet
        });
        t += 0.12;
        c.frame(30)?;
    }
    Ok(())
}

/// Stars: random LEDs flash on and slowly fade away.
fn twinkle<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(1);
        for _ in 0..3 {
            let (x, y, z) = (rand_below(c, 5) as usize, rand_below(c, 5) as usize, rand_below(c, 5) as usize);
            c.cube.set(x, y, z, FULL as u8);
        }
        c.frame(35)?;
    }
    Ok(())
}

/// A wall of light sweeps back and forth along x, then y, then up and down, leaving a short trail.
fn sweep<B: InputPin>(c: &mut Ctx<B>, rounds: u32) -> Result<(), Next> {
    for _ in 0..rounds {
        for axis in 0..3 {
            for k in 0..2 * N - 2 {
                let p = if k < N { k } else { 2 * N - 2 - k }; // 0 1 2 3 4 3 2 1
                c.cube.dim(4);
                for a in 0..N {
                    for b in 0..N {
                        let (x, y, h) = match axis {
                            0 => (p, a, b),
                            1 => (a, p, b),
                            _ => (a, b, p),
                        };
                        c.cube.set(x, y, layer(h), FULL as u8);
                    }
                }
                c.frame(60)?;
            }
        }
    }
    Ok(())
}

/// Cube-shaped shells (centre LED, the 3x3x3 around it, the outside) pulse outwards one after another.
fn shells<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut t = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        paint(c, |x, y, h| {
            let d = fabsf(x - CENTRE).max(fabsf(y - CENTRE)).max(fabsf(h - CENTRE)); // 0, 1 or 2
            let b = 0.5 + 0.5 * cosf(t - d * 1.8);
            b * b // squared: sharper pulses, darker gaps
        });
        t += 0.25;
        c.frame(30)?;
    }
    Ok(())
}

/// Two strands twisting around each other as they climb, with dim rungs between them (DNA).
fn helix<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut t = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.clear();
        for h in 0..N {
            let a = t + h as f32 * 0.8; // each layer is turned a bit further: that makes the twist
            let (co, si) = (cosf(a), sinf(a));
            let hf = h as f32;
            plot(c, CENTRE + 2.0 * co, CENTRE + 2.0 * si, hf, FULL);
            plot(c, CENTRE - 2.0 * co, CENTRE - 2.0 * si, hf, FULL);
            for r in [-1.0f32, 0.0, 1.0] {
                plot(c, CENTRE + r * co, CENTRE + r * si, hf, 2); // rung
            }
        }
        t += 0.15;
        c.frame(40)?;
    }
    Ok(())
}

/// Three planets on tilted orbits around a dim sun in the middle, each with a fading tail.
fn orbits<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    // (u, v, radius, speed): the planet is at centre + radius * (cos a * u + sin a * v)
    const ORBITS: [([f32; 3], [f32; 3], f32, f32); 3] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 2.0, 1.0),          // flat, around the middle layer
        ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0, -1.4),         // standing up, the other way round
        ([0.707, -0.707, 0.0], [0.408, 0.408, 0.816], 1.8, 1.9), // tilted across the diagonal
    ];
    let mut t = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(2);
        let mid = N / 2;
        c.cube.set(mid, mid, layer(mid), 3); // the sun
        for (u, v, r, w) in ORBITS {
            let a = t * w;
            let (co, si) = (cosf(a), sinf(a));
            let p = |k: usize| CENTRE + r * (co * u[k] + si * v[k]);
            plot(c, p(0), p(1), p(2), FULL);
        }
        t += 0.12;
        c.frame(35)?;
    }
    Ok(())
}

/// Bright bands roll diagonally through the cube, corner to corner.
fn diagonal<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut t = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        paint(c, |x, y, h| {
            let b = 0.5 + 0.5 * sinf((x + y + h) * 0.9 - t);
            b * b * b // cubed: thin bright bands
        });
        t += 0.3;
        c.frame(35)?;
    }
    Ok(())
}

/// Three balls bouncing off the walls and floor, with gravity and trails.
fn bounce<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let top = (N - 1) as f32;
    let mut balls = [[0.0f32; 6]; 3]; // [x, y, h, speed x, speed y, speed up]
    for b in balls.iter_mut() {
        let a = uniform(c, 0.0, TAU);
        *b = [uniform(c, 0.0, top), uniform(c, 0.0, top), top, 0.3 * cosf(a), 0.3 * sinf(a), 0.0];
    }
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(3);
        for b in balls.iter_mut() {
            b[5] -= 0.08; // gravity
            for k in 0..3 {
                b[k] += b[k + 3];
                // bounce: mirror the position back inside and reverse the speed
                if b[k] < 0.0 {
                    b[k] = -b[k];
                    b[k + 3] = -b[k + 3];
                } else if b[k] > top {
                    b[k] = 2.0 * top - b[k];
                    b[k + 3] = -b[k + 3];
                }
            }
        }
        for b in balls {
            plot(c, b[0], b[1], b[2], FULL);
        }
        c.frame(40)?;
    }
    Ok(())
}
