//! Place a static credential on the merchant endpoint: as a header
//! (`Authorization: Bearer …`) or as a query parameter (`?key=…`).

use connect_types::{CredentialPlacement, PlacementLocation};
use http::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("endpoint URL is invalid: {0}")]
    Url(#[from] url::ParseError),
    #[error("credential header name `{0}` is invalid")]
    HeaderName(String),
    #[error("credential contains characters that cannot be sent in a header")]
    HeaderValue,
}

/// Endpoint with the credential applied. `Debug` is deliberately not derived.
pub struct Endpoint {
    pub url: Url,
    pub headers: HeaderMap,
}

pub fn apply(
    url: &str,
    credential: Option<(&CredentialPlacement, &str)>,
) -> Result<Endpoint, CredentialError> {
    let mut url = Url::parse(url)?;
    let mut headers = HeaderMap::new();
    if let Some((placement, token)) = credential {
        let value = placement.render(token);
        match placement.location {
            PlacementLocation::Header => {
                let name = HeaderName::from_bytes(placement.name.as_bytes())
                    .map_err(|_| CredentialError::HeaderName(placement.name.clone()))?;
                let mut value =
                    HeaderValue::from_str(&value).map_err(|_| CredentialError::HeaderValue)?;
                value.set_sensitive(true);
                headers.insert(name, value);
            }
            PlacementLocation::Query => {
                let existing: Vec<(String, String)> = url
                    .query_pairs()
                    .filter(|(name, _)| name != placement.name.as_str())
                    .map(|(name, value)| (name.into_owned(), value.into_owned()))
                    .collect();
                url.query_pairs_mut()
                    .clear()
                    .extend_pairs(existing)
                    .append_pair(&placement.name, &value);
            }
        }
    }
    Ok(Endpoint { url, headers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_placement() {
        let placement = CredentialPlacement {
            location: PlacementLocation::Header,
            name: "Authorization".into(),
            format: Some("Bearer {token}".into()),
        };
        let endpoint = apply("https://mcp.mcd.cn", Some((&placement, "t0k"))).unwrap();
        assert_eq!(endpoint.headers["authorization"], "Bearer t0k");
        assert!(endpoint.headers["authorization"].is_sensitive());
        assert_eq!(endpoint.url.as_str(), "https://mcp.mcd.cn/");
    }

    #[test]
    fn query_placement_replaces_existing_param() {
        let placement = CredentialPlacement {
            location: PlacementLocation::Query,
            name: "key".into(),
            format: None,
        };
        let endpoint = apply(
            "https://mcp.didichuxing.com/mcp-servers?key=old&x=1",
            Some((&placement, "a b&c")),
        )
        .unwrap();
        assert_eq!(
            endpoint.url.as_str(),
            "https://mcp.didichuxing.com/mcp-servers?x=1&key=a+b%26c"
        );
        assert!(endpoint.headers.is_empty());
    }
}
