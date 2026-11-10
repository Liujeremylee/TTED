//! Quadrum LED Cube Code Generator animation player (port of QuadrumPlayer2.py).
//! The animation text file (../TTED.txt) is built into the firmware.

use crate::cube::{LEDS, N};

pub const SOURCE: &str = include_str!("../../TTED.txt");
const MAX_BYTES: usize = 16 * 1024;

pub struct Animation {
    raw: [u8; MAX_BYTES],
    len: usize,
    pub size: u8,
    pub bits_per_voxel: u8,
    /// Frame count from the header (16-bit little endian).
    pub header_frames: u16,
    bitmap_bytes: usize,
    /// Frames actually present in the file.
    pub frames: usize,
}

impl Animation {
    pub const fn empty() -> Self {
        Animation { raw: [0; MAX_BYTES], len: 0, size: 0, bits_per_voxel: 0, header_frames: 0, bitmap_bytes: 0, frames: 0 }
    }

    /// Pull every "0xNN" out of the text, like QuadrumAnimation.extract_bytes().
    pub fn parse(&mut self, text: &str) -> Result<(), &'static str> {
        let b = text.as_bytes();
        let mut i = 0;
        self.len = 0;
        while i + 1 < b.len() {
            if b[i] == b'0' && (b[i + 1] == b'x' || b[i + 1] == b'X') {
                // hex() of a missing character (past the end) is None, so this never reads out of bounds
                if let (Some(h), Some(l)) = (hex(b.get(i + 2).copied()), hex(b.get(i + 3).copied())) {
                    if self.len == MAX_BYTES {
                        return Err("animation too large");
                    }
                    self.raw[self.len] = (h << 4) | l;
                    self.len += 1;
                    i += 4;
                    continue;
                }
            }
            i += 1;
        }
        if self.len < 5 {
            return Err("invalid Quadrum file");
        }
        self.size = self.raw[0];
        self.bits_per_voxel = self.raw[2];
        self.header_frames = self.raw[3] as u16 | ((self.raw[4] as u16) << 8);
        if self.size as usize != N {
            return Err("animation is not for a 5x5x5 cube");
        }
        self.bitmap_bytes = match self.bits_per_voxel {
            1 => LEDS.div_ceil(8),
            4 => LEDS.div_ceil(2),
            _ => return Err("unsupported bits-per-voxel"),
        };
        let per_frame = 4 + self.bitmap_bytes;
        let available = (self.len - 5) / per_frame;
        self.frames = (self.header_frames as usize).min(available);
        Ok(())
    }

    /// Decode frame `f` into `out`; returns its delay in ms.
    pub fn frame(&self, f: usize, out: &mut [u8; LEDS]) -> u32 {
        let p = 5 + f * (4 + self.bitmap_bytes);
        let delay = f32::from_le_bytes([self.raw[p], self.raw[p + 1], self.raw[p + 2], self.raw[p + 3]]);
        let bm = &self.raw[p + 4..p + 4 + self.bitmap_bytes];
        for v in 0..LEDS {
            let value = if self.bits_per_voxel == 1 {
                if bm[v >> 3] & (1 << (v & 7)) != 0 { 15 } else { 0 }
            } else if v & 1 != 0 {
                (bm[v >> 1] >> 4) & 0x0F // low nibble first
            } else {
                bm[v >> 1] & 0x0F
            };
            out[remap(v)] = value;
        }
        if delay.is_finite() && delay > 0.0 { delay as u32 } else { 0 }
    }
}

/// Quadrum's axes -> the TTED cube's axes (same as QuadrumAnimation.remap_index).
fn remap(voxel: usize) -> usize {
    let x = voxel % N;
    let y = (voxel / N) % N;
    let z = voxel / (N * N);
    let (nx, ny, nz) = (x, N - 1 - z, y);
    nx + ny * N + nz * N * N
}

fn hex(c: Option<u8>) -> Option<u8> {
    match c? {
        c @ b'0'..=b'9' => Some(c - b'0'),
        c @ b'a'..=b'f' => Some(c - b'a' + 10),
        c @ b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}
