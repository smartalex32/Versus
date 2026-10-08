use eframe::egui::IconData;

// A square crop of the mark in assets/logo.png, sized for desktop launchers.
const ICON_PNG: &[u8] = include_bytes!("../assets/logo-icon.png");

pub fn icon_data() -> IconData {
    eframe::icon_data::from_png_bytes(ICON_PNG)
        .expect("the bundled Versus icon must be a valid PNG")
}

/// Preserves the blue half and transparency while making the dark half white
/// against dark UI backgrounds. The original asset is used for light mode.
pub fn themed_icon(dark_mode: bool) -> IconData {
    let mut icon = icon_data();
    if dark_mode {
        for (index, pixel) in icon.rgba.chunks_exact_mut(4).enumerate() {
            let x = index as u32 % icon.width;
            if x < icon.width / 2 && pixel[3] != 0 && pixel[2].saturating_sub(pixel[0]) < 24 {
                pixel[..3].fill(255);
            }
        }
    }
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_logo_yields_a_square_icon_with_visible_pixels() {
        let icon = icon_data();
        assert_eq!(icon.width, 512);
        assert_eq!(icon.height, 512);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] > 16));
    }
    #[test]
    fn themed_icon_preserves_blue_and_alpha_and_adapts_dark_half() {
        let original = icon_data();
        let light = themed_icon(false);
        let dark = themed_icon(true);
        assert_eq!(light.rgba, original.rgba);
        let mut changed = 0;
        for (index, (before, after)) in original
            .rgba
            .chunks_exact(4)
            .zip(dark.rgba.chunks_exact(4))
            .enumerate()
        {
            assert_eq!(before[3], after[3]);
            if index as u32 % original.width >= original.width / 2 {
                assert_eq!(before, after);
            }
            if before != after {
                changed += 1;
                assert_eq!(&after[..3], &[255, 255, 255]);
            }
        }
        assert!(changed > 10_000);
    }
}
