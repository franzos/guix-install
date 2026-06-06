use guix_install_core::keyboard::{Layout, parse_layouts};

const LST: &str = "\
! model
  pc105    Generic 105-key PC
! layout
  us       English (US)
  de       German
  pt       Portuguese
! variant
  nodeadkeys  German (no dead keys)
";

#[test]
fn parses_layout_section_only() {
    let layouts = parse_layouts(LST);
    assert_eq!(
        layouts,
        vec![
            Layout {
                code: "us".into(),
                description: "English (US)".into()
            },
            Layout {
                code: "de".into(),
                description: "German".into()
            },
            Layout {
                code: "pt".into(),
                description: "Portuguese".into()
            },
        ]
    );
}

#[test]
fn empty_input_yields_empty() {
    assert!(parse_layouts("").is_empty());
    assert!(parse_layouts("! model\n  pc105  PC\n").is_empty());
}
