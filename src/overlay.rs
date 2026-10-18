//! Bounded display-list geometry for standalone D3D8 verification.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rhw: f32,
    pub color: u32,
}
pub struct Frame {
    width: u32,
    height: u32,
    vertices: Vec<Vertex>,
}
impl Frame {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err("Invalid overlay viewport".into());
        }
        Ok(Self {
            width,
            height,
            vertices: Vec::new(),
        })
    }
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }
    pub fn rect(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        color: u32,
    ) -> Result<(), String> {
        if width > 16384 || height > 16384 {
            return Err("Overlay rectangle limit".into());
        }
        if self.vertices.len() + 6 > 200000 {
            return Err("Overlay geometry capacity".into());
        }
        let left = i64::from(x).clamp(0, i64::from(self.width));
        let top = i64::from(y).clamp(0, i64::from(self.height));
        let right = (i64::from(x) + i64::from(width)).clamp(0, i64::from(self.width));
        let bottom = (i64::from(y) + i64::from(height)).clamp(0, i64::from(self.height));
        if left >= right || top >= bottom {
            return Ok(());
        }
        for (x, y) in [
            (left, top),
            (right, top),
            (left, bottom),
            (left, bottom),
            (right, top),
            (right, bottom),
        ] {
            self.vertices.push(Vertex {
                x: x as f32 - 0.5,
                y: y as f32 - 0.5,
                z: 0.0,
                rhw: 1.0,
                color,
            });
        }
        Ok(())
    }
    pub fn image(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        scale: u32,
        pixels: &[u32],
    ) -> Result<(), String> {
        if width == 0
            || height == 0
            || width > 128
            || height > 128
            || !(1..=8).contains(&scale)
            || pixels.len() != width as usize * height as usize
        {
            return Err("Invalid bounded RGBA overlay image".into());
        }
        let before = self.vertices.len();
        let run = (|| {
            for py in 0..height {
                for px in 0..width {
                    let dx = i64::from(x) + i64::from(px * scale);
                    let dy = i64::from(y) + i64::from(py * scale);
                    self.rect(
                        i32::try_from(dx).map_err(|_| "Overlay coordinate overflow")?,
                        i32::try_from(dy).map_err(|_| "Overlay coordinate overflow")?,
                        scale,
                        scale,
                        pixels[(py * width + px) as usize],
                    )?;
                }
            }
            Ok::<(), String>(())
        })();
        if run.is_err() {
            self.vertices.truncate(before);
        }
        run
    }
    /// Compact diagnostic font. Unrepresented printable glyphs use a visible box.
    pub fn text(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        scale: u32,
        color: u32,
    ) -> Result<(), String> {
        if text.len() > 256
            || !text.is_ascii()
            || text.bytes().any(|b| b.is_ascii_control())
            || !(1..=8).contains(&scale)
        {
            return Err("Unsupported diagnostic overlay text/scale".into());
        }
        let before = self.vertices.len();
        let run = (|| {
            for (index, c) in text.chars().enumerate() {
                let glyph = glyph(c.to_ascii_uppercase());
                for (row, bits) in glyph.iter().enumerate() {
                    for col in 0..5 {
                        if bits & (1 << (4 - col)) != 0 {
                            let dx = i64::from(x)
                                + (index as i64 * 6 + i64::from(col)) * i64::from(scale);
                            let dy = i64::from(y) + row as i64 * i64::from(scale);
                            self.rect(
                                i32::try_from(dx).map_err(|_| "Text coordinate overflow")?,
                                i32::try_from(dy).map_err(|_| "Text coordinate overflow")?,
                                scale,
                                scale,
                                color,
                            )?;
                        }
                    }
                }
            }
            Ok::<(), String>(())
        })();
        if run.is_err() {
            self.vertices.truncate(before);
        }
        run
    }
}
fn glyph(c: char) -> [u8; 7] {
    match c {
        ' ' => [0; 7],
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'N' => [17, 25, 25, 21, 19, 19, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        _ => [31, 17, 21, 21, 21, 17, 31],
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_clips_images_scales_text_and_refuses_overflow() {
        let mut frame = Frame::new(100, 80).unwrap();
        frame.rect(-5, -5, 10, 10, 0xffffffff).unwrap();
        assert_eq!(frame.vertices.len(), 6);
        assert!(
            frame
                .vertices
                .iter()
                .all(|v| v.x >= -0.5 && v.y >= -0.5 && v.x <= 4.5 && v.y <= 4.5)
        );
        frame.image(10, 10, 2, 2, 2, &[1, 2, 3, 4]).unwrap();
        assert_eq!(frame.vertices.len(), 30);
        frame.text(1, 20, "RUST", 2, 0xffffffff).unwrap();
        let count = frame.vertices.len();
        assert!(frame.image(0, 0, 2, 2, 1, &[1]).is_err());
        assert_eq!(frame.vertices.len(), count);
        assert!(frame.text(i32::MAX, 0, "RR", 8, 0).is_err());
        assert_eq!(frame.vertices.len(), count);
        assert!(frame.text(0, 0, "\n", 1, 0).is_err());
    }
}
