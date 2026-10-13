//! Application facade and Rust↔UI contract (ADR-0001 §4).
#![cfg_attr(test, allow(clippy::unwrap_used))]

/// Placeholder response used to exercise generated bindings before #34.
#[derive(serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldInfo {
    /// Display name of the application.
    pub app_name: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn contract_uses_camel_case() {
        let info = super::ScaffoldInfo {
            app_name: "crank".into(),
        };
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            serde_json::json!({"appName": "crank"})
        );
    }
}
