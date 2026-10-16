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
#[path = "../tests/unit/logo/mod.rs"]
mod tests;
