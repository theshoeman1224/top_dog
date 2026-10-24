//! Applications select their own target. Desktops decide whether to foreground it.
use top_dog_protocol::FocusResult;

pub trait DesktopActivation {
    fn request_focus(
        &self,
        application_request: impl FnOnce() -> Result<bool, String>,
    ) -> Result<FocusResult, String>;
}

/// Use the application's native window.focus() implementation. This works across
/// desktops without depending on window titles or compositor scripting APIs.
pub struct NativeDesktop {
    pub wayland: bool,
}

impl NativeDesktop {
    pub fn detect() -> Self {
        Self {
            wayland: std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland")
                || std::env::var_os("WAYLAND_DISPLAY").is_some(),
        }
    }
}

impl DesktopActivation for NativeDesktop {
    fn request_focus(
        &self,
        request: impl FnOnce() -> Result<bool, String>,
    ) -> Result<FocusResult, String> {
        // A successful application request does not prove compositor focus.
        Ok(if request()? {
            FocusResult::Requested
        } else {
            FocusResult::Denied
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successful_request_does_not_claim_foreground_focus() {
        for wayland in [true, false] {
            let desktop = NativeDesktop { wayland };
            assert_eq!(
                desktop.request_focus(|| Ok(true)).unwrap(),
                FocusResult::Requested
            );
            assert_eq!(
                desktop.request_focus(|| Ok(false)).unwrap(),
                FocusResult::Denied
            );
            assert!(desktop.request_focus(|| Err("denied".into())).is_err());
        }
    }
}
