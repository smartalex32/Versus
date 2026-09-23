use eframe::egui::IconData;

const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");

/// Extract the mark above the wordmark for compact window and toolbar icons.
pub fn icon_data() -> IconData {
    let source = eframe::icon_data::from_png_bytes(LOGO_PNG)
        .expect("the bundled Versus logo must be a valid PNG");
    let width = source.width as usize;
    let height = source.height as usize;
    let occupied = |x: usize, y: usize| source.rgba[(y * width + x) * 4 + 3] > 16;

    let mut top = None;
    let mut bottom = 0;
    for y in 0..height {
        let has_ink = (0..width).any(|x| occupied(x, y));
        if has_ink {
            top.get_or_insert(y);
            bottom = y;
        } else if top.is_some() {
            break;
        }
    }
    let top = top.expect("the bundled Versus logo must contain visible pixels");
    let mut left = width;
    let mut right = 0;
    for y in top..=bottom {
        for x in 0..width {
            if occupied(x, y) {
                left = left.min(x);
                right = right.max(x);
            }
        }
    }
    let mark_width = right - left + 1;
    let mark_height = bottom - top + 1;
    let side = mark_width.max(mark_height) + 40;
    let x_offset = (side - mark_width) / 2;
    let y_offset = (side - mark_height) / 2;
    let mut rgba = vec![0; side * side * 4];
    for y in 0..mark_height {
        let src = ((top + y) * width + left) * 4;
        let dst = ((y + y_offset) * side + x_offset) * 4;
        rgba[dst..dst + mark_width * 4].copy_from_slice(&source.rgba[src..src + mark_width * 4]);
    }
    IconData {
        rgba,
        width: side as u32,
        height: side as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_logo_yields_a_square_icon_with_visible_pixels() {
        let icon = icon_data();
        assert_eq!(icon.width, icon.height);
        assert!(icon.width > 300);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] > 16));
    }
}
