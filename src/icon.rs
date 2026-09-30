//! The app icon, drawn in code (a rounded blue square with a play triangle)
//! so no binary asset has to be shipped.

const SIZE: usize = 256;
const RADIUS: f32 = 52.0;

fn inside_rounded_square(x: f32, y: f32) -> bool {
    let s = SIZE as f32;
    let cx = x.clamp(RADIUS, s - RADIUS);
    let cy = y.clamp(RADIUS, s - RADIUS);
    (x - cx).powi(2) + (y - cy).powi(2) <= RADIUS * RADIUS
}

fn inside_triangle(x: f32, y: f32) -> bool {
    // Play triangle, optically centred.
    let (a, b, c) = ((96.0, 70.0), (96.0, 186.0), (196.0, 128.0));
    let sign = |p: (f32, f32), q: (f32, f32)| (x - q.0) * (p.1 - q.1) - (p.0 - q.0) * (y - q.1);
    let d1 = sign(a, b);
    let d2 = sign(b, c);
    let d3 = sign(c, a);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

pub fn app_icon() -> egui::IconData {
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for py in 0..SIZE {
        for px in 0..SIZE {
            // 2x2 supersampling for smooth edges.
            let (mut bg, mut fg) = (0u32, 0u32);
            for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let (x, y) = (px as f32 + ox, py as f32 + oy);
                if inside_rounded_square(x, y) {
                    bg += 1;
                    if inside_triangle(x, y) {
                        fg += 1;
                    }
                }
            }
            if bg == 0 {
                continue;
            }
            // Vertical gradient background, white triangle on top.
            let t = py as f32 / SIZE as f32;
            let base = [
                (34.0 + 20.0 * t) as u8,
                (92.0 + 30.0 * t) as u8,
                (200.0 - 30.0 * t) as u8,
            ];
            let mix = fg as f32 / bg as f32;
            let i = (py * SIZE + px) * 4;
            for k in 0..3 {
                rgba[i + k] = (base[k] as f32 * (1.0 - mix) + 255.0 * mix) as u8;
            }
            rgba[i + 3] = (bg * 255 / 4) as u8;
        }
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_expected_size_and_transparent_corners() {
        let icon = app_icon();
        assert_eq!(icon.rgba.len(), SIZE * SIZE * 4);
        assert_eq!(icon.rgba[3], 0); // top-left corner is transparent
        let centre = (SIZE / 2 * SIZE + SIZE / 2) * 4;
        assert_eq!(icon.rgba[centre + 3], 255); // centre is opaque
        assert_eq!(&icon.rgba[centre..centre + 3], &[255, 255, 255]); // inside the triangle
    }
}
