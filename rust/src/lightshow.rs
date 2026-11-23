//! Light shows: sixteen effects that use the 10 brightness levels and the fast refresh. They play one after
//! another (then the animations.rs ones); button 1 or 'n' skips to the next one.
//!
//! Same building blocks as animations.rs: `c.cube.set()` / `plot()` to light LEDs, `c.cube.dim(n)` for
//! fading trails, `c.frame(ms)?` to show the picture and wait.

use embedded_hal::digital::InputPin;
use libm::{cosf, expf, fabsf, sinf, sqrtf};

use super::{Ctx, Next};
use crate::animations::{deadline, layer, plot, rand_below, sq, uniform, CENTRE, FULL, TAU};
use crate::cube::N;

/// How many light shows this file has, and their names (shown over USB serial).
pub const COUNT: u8 = 16;
pub const NAMES: [&str; COUNT as usize] = [
    "plasma", "twinkle", "plane sweep", "shells", "double helix", "orbits", "diagonal wave", "bouncing balls",
    "fire", "lightning", "heartbeat", "fountain", "tornado", "lava lamp", "warp", "fill",
];

/// Play light show number `which` (0 = plasma ... COUNT-1 = fill) once.
pub fn play<B: InputPin>(c: &mut Ctx<B>, which: u8) -> Result<(), Next> {
    match which {
        0 => plasma(c, 12),
        1 => twinkle(c, 8),
        2 => sweep(c, 2),
        3 => shells(c, 8),
        4 => helix(c, 10),
        5 => orbits(c, 12),
        6 => diagonal(c, 8),
        7 => bounce(c, 12),
        8 => fire(c, 12),
        9 => lightning(c, 10),
        10 => heartbeat(c, 8),
        11 => fountain(c, 12),
        12 => tornado(c, 10),
        13 => lava(c, 14),
        14 => warp(c, 10),
        _ => fill(c, 2),
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

/// Flames: the bottom layer burns at random, and each layer above takes the heat of the one below (and its
/// neighbours) from the last frame, minus some cooling. The heat rises one layer per frame and flickers.
fn fire<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut heat = [[[0.0f32; N]; N]; N]; // [height][x][y], 0..1
    let end = deadline(c, secs);
    while c.now_us() < end {
        for h in (1..N).rev() {
            // top down, so every layer reads the layer below as it was last frame
            for x in 0..N {
                for y in 0..N {
                    let mut side = 0.0;
                    let mut n = 0.0;
                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if (0..N as i32).contains(&nx) && (0..N as i32).contains(&ny) {
                            side += heat[h - 1][nx as usize][ny as usize];
                            n += 1.0;
                        }
                    }
                    let cool = uniform(c, 0.12, 0.32);
                    heat[h][x][y] = (heat[h - 1][x][y] * 0.65 + side / n * 0.35 - cool).max(0.0);
                }
            }
        }
        for x in 0..N {
            for y in 0..N {
                // the embers: hotter in the middle of the floor
                let mid = 1.0 - (fabsf(x as f32 - CENTRE) + fabsf(y as f32 - CENTRE)) * 0.1;
                heat[0][x][y] = uniform(c, 0.4, 1.0) * mid;
            }
        }
        for h in 0..N {
            for x in 0..N {
                for y in 0..N {
                    c.cube.set(x, y, layer(h), level(heat[h][x][y]));
                }
            }
        }
        c.frame(55)?;
    }
    Ok(())
}

/// A dim, flickering cloud on the top layer; now and then a jagged bolt strikes down to the floor and flashes.
fn lightning<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let end = deadline(c, secs);
    while c.now_us() < end {
        // cloud flicker while waiting for the next strike
        for _ in 0..rand_below(c, 12) + 6 {
            c.cube.clear();
            for x in 0..N {
                for y in 0..N {
                    if rand_below(c, 3) == 0 {
                        let v = 1 + rand_below(c, 2) as u8;
                        c.cube.set(x, y, layer(N - 1), v);
                    }
                }
            }
            c.frame(60)?;
        }
        // the bolt: from a random point on top, wander down one layer at a time
        let mut path = [(0usize, 0usize); N];
        let (mut x, mut y) = (rand_below(c, 5) as i32, rand_below(c, 5) as i32);
        for h in (0..N).rev() {
            path[h] = (x as usize, y as usize);
            x = (x + rand_below(c, 3) as i32 - 1).clamp(0, N as i32 - 1);
            y = (y + rand_below(c, 3) as i32 - 1).clamp(0, N as i32 - 1);
        }
        // flash, off, flash again, then fade
        for (on, ms) in [(true, 50), (false, 40), (true, 90)] {
            c.cube.clear();
            if on {
                for (h, &(px, py)) in path.iter().enumerate() {
                    c.cube.set(px, py, layer(h), FULL as u8);
                }
            }
            c.frame(ms)?;
        }
        for _ in 0..5 {
            c.cube.dim(2);
            c.frame(40)?;
        }
    }
    Ok(())
}

/// A heartbeat: lub-dub, then rest. Brightest in the middle, fading towards the corners.
fn heartbeat<B: InputPin>(c: &mut Ctx<B>, beats: u32) -> Result<(), Next> {
    for _ in 0..beats {
        for k in 0..40 {
            let t = k as f32 / 40.0; // one beat = 40 frames x 25 ms = 1 s
            // two quick bumps: a strong one at 0.1 s and a weaker one at 0.3 s
            let e = expf(-sq((t - 0.10) / 0.05)) + 0.6 * expf(-sq((t - 0.30) / 0.05));
            paint(c, |x, y, h| {
                let d = sqrtf(sq(x - CENTRE) + sq(y - CENTRE) + sq(h - CENTRE)); // 0 .. 3.46
                e * (1.25 - d / 3.0)
            });
            c.frame(25)?;
        }
    }
    Ok(())
}

/// Sparks shoot up from the middle of the floor, spread out, and fall back down.
fn fountain<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut drops: heapless::Vec<[f32; 6], 48> = heapless::Vec::new(); // [x, y, h, speed x, speed y, speed up]
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(3);
        for _ in 0..2 {
            let a = uniform(c, 0.0, TAU);
            let spread = uniform(c, 0.08, 0.22);
            let up = uniform(c, 0.85, 1.05);
            let _ = drops.push([CENTRE, CENTRE, 0.0, spread * cosf(a), spread * sinf(a), up]);
        }
        for d in drops.iter_mut() {
            d[0] += d[3];
            d[1] += d[4];
            d[2] += d[5];
            d[5] -= 0.13; // gravity
        }
        drops.retain(|d| d[2] >= -0.5);
        for d in drops.iter() {
            plot(c, d[0], d[1], d[2], FULL);
        }
        c.frame(45)?;
    }
    Ok(())
}

/// A spinning funnel, narrow at the bottom and wide at the top.
fn tornado<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut t = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        c.cube.dim(3);
        for h in 0..N {
            let r = 0.3 + h as f32 * 0.5; // radius grows with height
            let a = t * 2.2 + h as f32 * 0.7;
            for k in 0..3 {
                let ak = a + k as f32 * TAU / 3.0;
                plot(c, CENTRE + r * cosf(ak), CENTRE + r * sinf(ak), h as f32, FULL);
            }
        }
        t += 0.1;
        c.frame(35)?;
    }
    Ok(())
}

/// Lava lamp: three soft blobs drift slowly through the cube and melt together where they meet.
fn lava<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut t = 0.0f32;
    let end = deadline(c, secs);
    while c.now_us() < end {
        // each blob follows its own slow wandering path
        let blobs = [
            (CENTRE + 1.5 * sinf(t * 0.7), CENTRE + 1.5 * cosf(t * 0.5), CENTRE + 1.8 * sinf(t * 0.4)),
            (CENTRE + 1.5 * cosf(t * 0.6 + 1.0), CENTRE + 1.5 * sinf(t * 0.8), CENTRE + 1.8 * cosf(t * 0.5 + 2.0)),
            (CENTRE + 1.5 * sinf(t * 0.4 + 2.0), CENTRE + 1.5 * cosf(t * 0.7 + 1.0), CENTRE + 1.8 * sinf(t * 0.6 + 4.0)),
        ];
        paint(c, |x, y, h| {
            let mut f = 0.0;
            for (bx, by, bh) in blobs {
                f += 1.0 / (sq(x - bx) + sq(y - by) + sq(h - bh) + 0.6);
            }
            (f - 0.45) * 1.4 // the edge of the blobs is where the field is about 0.45
        });
        t += 0.06;
        c.frame(30)?;
    }
    Ok(())
}

/// Warp speed: stars rush from the back of the cube to the front, faster ones leaving longer streaks.
fn warp<B: InputPin>(c: &mut Ctx<B>, secs: u64) -> Result<(), Next> {
    let mut stars: heapless::Vec<[f32; 4], 24> = heapless::Vec::new(); // [x, depth, h, speed]
    let end = deadline(c, secs);
    let back = (N - 1) as f32;
    while c.now_us() < end {
        c.cube.dim(4);
        if uniform(c, 0.0, 1.0) < 0.7 {
            let s = [rand_below(c, 5) as f32, back, rand_below(c, 5) as f32, uniform(c, 0.25, 0.6)];
            let _ = stars.push(s);
        }
        for s in stars.iter_mut() {
            s[1] -= s[3];
        }
        stars.retain(|s| s[1] > -0.5);
        for s in stars.iter() {
            plot(c, s[0], s[1], s[2], FULL);
        }
        c.frame(40)?;
    }
    Ok(())
}

/// The cube fills up one random LED at a time, then empties the same way.
fn fill<B: InputPin>(c: &mut Ctx<B>, rounds: u32) -> Result<(), Next> {
    let mut order = [0u8; N * N * N];
    for _ in 0..rounds {
        for on in [true, false] {
            for (i, o) in order.iter_mut().enumerate() {
                *o = i as u8;
            }
            for i in (1..order.len()).rev() {
                // shuffle
                let j = rand_below(c, i as u32 + 1) as usize;
                order.swap(i, j);
            }
            for &i in order.iter() {
                let i = i as usize;
                c.cube.set(i % N, (i / N) % N, i / (N * N), if on { FULL as u8 } else { 0 });
                c.frame(18)?;
            }
            c.frame(400)?;
        }
    }
    Ok(())
}
