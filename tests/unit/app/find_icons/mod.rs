use super::*;

#[test]
fn icon_codepoints_match_the_official_codicons_css() {
    assert_eq!(FindIcon::MatchCase.glyph(), '\u{EAB1}');
    assert_eq!(FindIcon::MatchWholeWord.glyph(), '\u{EB7E}');
    assert_eq!(FindIcon::UseRegularExpression.glyph(), '\u{EB38}');
    assert_eq!(FindIcon::ReplaceNext.glyph(), '\u{EB3D}');
    assert_eq!(FindIcon::ReplaceAll.glyph(), '\u{EB3C}');
}

#[test]
fn register_keeps_default_faces_and_adds_the_named_icon_family() {
    let mut definitions = egui::FontDefinitions::default();
    let proportional = definitions
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .expect("default proportional face");

    register(&mut definitions);
    register(&mut definitions);

    assert_eq!(
        definitions.families.get(&egui::FontFamily::Proportional),
        Some(&proportional)
    );
    assert_eq!(
        definitions
            .families
            .get(&egui::FontFamily::Name(FONT_FAMILY.into())),
        Some(&vec![FONT_NAME.to_owned()])
    );
    assert!(definitions.font_data.contains_key(FONT_NAME));
}
