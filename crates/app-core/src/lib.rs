#![doc = "Application orchestration boundary for Teratai Analytics Desktop."]

pub mod generated;

/// Identifies this crate as an initialized workspace component.
#[must_use]
pub const fn component_name() -> &'static str {
    "app-core"
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    #[test]
    fn exposes_component_name() {
        assert_eq!(super::component_name(), "app-core");
    }

    #[test]
    fn generated_contract_metadata_round_trips_shared_fixture() {
        let fixture =
            include_str!("../../../packages/contracts/fixtures/contract-metadata.valid.json");
        let metadata: super::generated::contract_metadata::ContractMetadata =
            serde_json::from_str(fixture).expect("shared contract fixture must deserialize");
        let serialized: Value =
            serde_json::to_value(metadata).expect("contract metadata must serialize");

        assert_eq!(
            serialized,
            json!({
                "generator_revision": 1,
                "protocol_version": "1.0",
                "schema_name": "contract-metadata",
                "schema_version": "1.0.0"
            })
        );
    }
}
