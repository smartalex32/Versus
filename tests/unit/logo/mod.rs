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
