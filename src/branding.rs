//! Procedurally drawn application mark, used for the tray and window icons.
//!
//! Drawing it at startup avoids shipping binary assets and keeps the icon
//! crisp at any size. Supersampling provides anti-aliasing.

/// RGBA pixel data for a square icon.
pub struct IconRgba {
    pub rgba: Vec<u8>,
    pub size: u32,
}

const BODY: [f32; 3] = [0.90, 0.91, 0.94];
const OUTLINE: [f32; 3] = [0.11, 0.12, 0.15];
const SAMPLES: u32 = 4;

/// Whether the normalized point lies inside the ghost silhouette grown by
/// `grow` (used to produce a thin dark outline that reads on any taskbar).
fn in_ghost(u: f32, v: f32, grow: f32) -> bool {
    let (cx, cy, r) = (0.5, 0.45, 0.33 + grow);
    let head = (u - cx).powi(2) + (v - cy).powi(2) <= r * r;
    let in_columns = u >= 0.17 - grow && u <= 0.83 + grow;
    // Scalloped hem: rounded bumps point down, notches point up.
    let phase = ((u - 0.17) / 0.66).clamp(0.0, 1.0);
    let hem = 0.78 + 0.08 * (phase * 3.0 * std::f32::consts::PI).sin().abs() + grow;
    let body = in_columns && v >= cy && v <= hem;
    head || body
}

fn in_eye(u: f32, v: f32) -> bool {
    [(0.40, 0.45), (0.60, 0.45)]
        .iter()
        .any(|(ex, ey)| (u - ex).powi(2) + (v - ey).powi(2) <= 0.055 * 0.055)
}

/// Render the mark at `size` x `size` pixels.
pub fn render(size: u32) -> IconRgba {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let total = (SAMPLES * SAMPLES) as f32;
    for y in 0..size {
        for x in 0..size {
            let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let u = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let colour = if in_eye(u, v) {
                        Some(OUTLINE)
                    } else if in_ghost(u, v, 0.0) {
                        Some(BODY)
                    } else if in_ghost(u, v, 0.045) {
                        Some(OUTLINE)
                    } else {
                        None
                    };
                    if let Some(c) = colour {
                        r += c[0];
                        g += c[1];
                        b += c[2];
                        a += 1.0;
                    }
                }
            }
            if a > 0.0 {
                // Colour is averaged over covered samples only (straight alpha).
                rgba.extend_from_slice(&[
                    (r / a * 255.0).round() as u8,
                    (g / a * 255.0).round() as u8,
                    (b / a * 255.0).round() as u8,
                    (a / total * 255.0).round() as u8,
                ]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    IconRgba { rgba, size }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_expected_buffer_length() {
        let icon = render(32);
        assert_eq!(icon.rgba.len(), 32 * 32 * 4);
    }

    #[test]
    fn centre_is_opaque_and_corner_is_transparent() {
        let icon = render(32);
        let px = |x: usize, y: usize| &icon.rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(px(16, 12)[3], 255, "ghost body should be opaque");
        assert_eq!(px(0, 0)[3], 0, "corner should be transparent");
    }
}

