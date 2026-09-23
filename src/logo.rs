use eframe::egui::IconData;

// A square crop of the mark in assets/logo.png, sized for desktop launchers.
const ICON_PNG: &[u8] = include_bytes!("../assets/logo-icon.png");

pub fn icon_data() -> IconData {
    eframe::icon_data::from_png_bytes(ICON_PNG)
        .expect("the bundled Versus icon must be a valid PNG")
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
}
