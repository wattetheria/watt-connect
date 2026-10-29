//! Catalog snapshot compiled into the binary from `catalog/services/*.json`.
//!
//! It is the last-resort fallback when there is no verified cache and the
//! catalog service is unreachable (or no signing key is configured yet). It is
//! trusted because it ships inside the signed npm package, not because of a
//! catalog signature.

use crate::schema::{CATALOG_SCHEMA_VERSION, Catalog, ServerEntry, compare_versions};

const SOURCES: &[(&str, &str)] = &[
    (
        "cn.didichuxing.ride.json",
        include_str!("../../../catalog/services/cn.didichuxing.ride.json"),
    ),
    (
        "cn.lkcoffee.order.json",
        include_str!("../../../catalog/services/cn.lkcoffee.order.json"),
    ),
    (
        "cn.mcd.order.json",
        include_str!("../../../catalog/services/cn.mcd.order.json"),
    ),
    (
        "io.wattetheria.node.json",
        include_str!("../../../catalog/services/io.wattetheria.node.json"),
    ),
];

/// Build a catalog from service entries; its version is the newest entry version.
#[must_use]
pub fn assemble(services: Vec<ServerEntry>) -> Catalog {
    let version = services
        .iter()
        .map(|entry| entry.version.as_str())
        .max_by(|a, b| compare_versions(a, b))
        .unwrap_or("0")
        .to_owned();
    Catalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        version,
        generated_at: None,
        services,
    }
}

/// The bundled snapshot. Panics only if a bundled file is malformed, which
/// the unit tests and CI catalog validation rule out.
#[must_use]
pub fn bundled_catalog() -> Catalog {
    let services = SOURCES
        .iter()
        .map(|(file, source)| {
            serde_json::from_str::<ServerEntry>(source)
                .unwrap_or_else(|error| panic!("bundled catalog entry {file} is invalid: {error}"))
        })
        .collect();
    assemble(services)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{has_errors, validate_catalog};

    #[test]
    fn bundled_catalog_is_valid() {
        let catalog = bundled_catalog();
        let issues = validate_catalog(&catalog);
        assert!(!has_errors(&issues), "{issues:#?}");
        assert_eq!(catalog.services.len(), SOURCES.len());
    }
}
