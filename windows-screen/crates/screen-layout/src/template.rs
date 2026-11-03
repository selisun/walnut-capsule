//! 布局模板：只按宽高比类别选择，不判断具体型号。

use screen_core::{ScreenProfile, SizeClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Portrait2Col,
    Landscape4Col,
    Wide6Col,
    Square2x2,
    Generic4Col,
}

impl TemplateKind {
    pub fn name(self) -> &'static str {
        match self {
            TemplateKind::Portrait2Col => "portrait-2col",
            TemplateKind::Landscape4Col => "landscape-4col",
            TemplateKind::Wide6Col => "wide-6col",
            TemplateKind::Square2x2 => "square-2x2",
            TemplateKind::Generic4Col => "generic-4col",
        }
    }

    pub fn cols(self) -> usize {
        match self {
            TemplateKind::Portrait2Col => 2,
            TemplateKind::Landscape4Col => 4,
            TemplateKind::Wide6Col => 6,
            TemplateKind::Square2x2 => 2,
            TemplateKind::Generic4Col => 4,
        }
    }

    /// 未知/未覆盖的 Profile 使用 generic 模板，绝不假设某一尺寸。
    pub fn for_profile(profile: &ScreenProfile) -> Self {
        match profile.size_class {
            SizeClass::Square => TemplateKind::Square2x2,
            SizeClass::Portrait => TemplateKind::Portrait2Col,
            SizeClass::Landscape => TemplateKind::Landscape4Col,
            SizeClass::Wide | SizeClass::UltraWide => TemplateKind::Wide6Col,
        }
    }

    pub fn from_override(name: &str) -> Option<Self> {
        match name {
            "portrait-2col" => Some(TemplateKind::Portrait2Col),
            "landscape-4col" => Some(TemplateKind::Landscape4Col),
            "wide-6col" => Some(TemplateKind::Wide6Col),
            "square-2x2" => Some(TemplateKind::Square2x2),
            "generic-4col" => Some(TemplateKind::Generic4Col),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::Orientation;

    #[test]
    fn template_by_size_class() {
        let p = ScreenProfile::new("3.5p", 320, 480, Orientation::Portrait, true).expect("p");
        assert_eq!(TemplateKind::for_profile(&p), TemplateKind::Portrait2Col);

        let p = ScreenProfile::new("3.5l", 480, 320, Orientation::Landscape, true).expect("p");
        assert_eq!(TemplateKind::for_profile(&p), TemplateKind::Landscape4Col);

        let p = ScreenProfile::new("9.2", 1920, 462, Orientation::Landscape, false).expect("p");
        assert_eq!(TemplateKind::for_profile(&p), TemplateKind::Wide6Col);

        let p = ScreenProfile::new("2.1", 480, 480, Orientation::Landscape, false).expect("p");
        assert_eq!(TemplateKind::for_profile(&p), TemplateKind::Square2x2);
    }

    #[test]
    fn name_roundtrip() {
        for t in [
            TemplateKind::Portrait2Col,
            TemplateKind::Landscape4Col,
            TemplateKind::Wide6Col,
            TemplateKind::Square2x2,
            TemplateKind::Generic4Col,
        ] {
            assert_eq!(TemplateKind::from_override(t.name()), Some(t));
        }
        assert_eq!(TemplateKind::from_override("unknown"), None);
    }
}
