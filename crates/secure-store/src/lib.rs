#![doc = "Secure secret and project key storage boundary."]

/// Identifies this crate as an initialized workspace component.
#[must_use]
pub const fn component_name() -> &'static str {
    "secure-store"
}

#[cfg(test)]
mod tests {
    #[test]
    fn exposes_component_name() {
        assert_eq!(super::component_name(), "secure-store");
    }
}
