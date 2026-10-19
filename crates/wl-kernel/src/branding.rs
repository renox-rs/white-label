//! Branding (`PLT.ORG.01`): the client's colours as the UI kit's tokens, on
//! Renox's [`Theme`], with its contrast guard (DESIGN-SYSTEM.md §4).

use renox::theme::{Brand, ContrastErrorKind, Theme};

use crate::config::{Branding, ConfigError};

/// The theme for `[branding]`. A colour that cannot reach readable contrast
/// stops the boot and names the nearest colour that passes (wl-kernel §5.1).
pub fn theme(branding: &Branding) -> Result<Theme, ConfigError> {
    let brand = Brand::new(&branding.accent)
        .second(&branding.second)
        .display_font(&branding.display_font);
    Theme::from_brand(&brand).map_err(|err| {
        let field = if err.color.eq_ignore_ascii_case(&branding.accent) {
            "accent"
        } else {
            "second"
        };
        ConfigError(match err.kind {
            ContrastErrorKind::LowContrast => format!(
                "[branding] {field} `{}` is not readable on the kit's backgrounds; the nearest colour that is: {}",
                err.color, err.suggestion
            ),
            _ => format!("[branding] {field} `{}` is not a colour like #0b6e66", err.color),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_platform_default_passes() {
        assert!(
            theme(&Branding::default())
                .unwrap()
                .css()
                .contains("--rx-accent")
        );
    }

    #[test]
    fn a_pale_accent_is_refused_with_a_suggestion() {
        let pale = Branding {
            accent: "#FFF59D".into(),
            ..Branding::default()
        };
        let err = theme(&pale).unwrap_err();
        assert!(
            err.0
                .starts_with("[branding] accent `#FFF59D` is not readable"),
            "{err}"
        );
        let suggestion = err.0.rsplit(' ').next().unwrap();
        let fixed = Branding {
            accent: suggestion.into(),
            ..Branding::default()
        };
        assert!(theme(&fixed).is_ok());
    }

    #[test]
    fn a_malformed_colour_is_refused() {
        let bad = Branding {
            second: "teal".into(),
            ..Branding::default()
        };
        assert!(
            theme(&bad)
                .unwrap_err()
                .0
                .contains("second `teal` is not a colour")
        );
    }
}
