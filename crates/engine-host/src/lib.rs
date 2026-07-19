#![doc = "Python analytics engine lifecycle and IPC boundary."]

/// Identifies this crate as an initialized workspace component.
#[must_use]
pub const fn component_name() -> &'static str {
    "engine-host"
}

#[cfg(test)]
mod tests {
    #[test]
    fn exposes_component_name() {
        assert_eq!(super::component_name(), "engine-host");
    }
}
