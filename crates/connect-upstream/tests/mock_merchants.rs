//! End-to-end against the testkit mocks: credential placement, auth failure
//! classification, version fallback, SSE, and rate-limit backoff.

use connect_catalog::bundled::bundled_catalog;
use connect_testkit::{MockMerchant, profiles};
use connect_types::{ConnectErrorCode, ServiceId};
use connect_upstream::{BackoffPolicy, UpstreamPool, UpstreamTarget, probe};
use std::time::Duration;
use zeroize::Zeroizing;

fn target(service: &str, url: &str, token: Option<&str>) -> UpstreamTarget {
    let catalog = bundled_catalog();
    let entry = catalog
        .get(&ServiceId::parse(service).unwrap())
        .unwrap()
        .clone();
    let meta = entry.connect();
    UpstreamTarget {
        service: entry.name.clone(),
        url: url.to_owned(),
        credential: token.map(|token| {
            (
                meta.credential.as_ref().unwrap().placement.clone(),
                Zeroizing::new(token.to_owned()),
            )
        }),
        protocol_versions: meta.protocol_versions.clone(),
        auth_failure: meta.auth_failure.clone(),
        rate_limited: meta.rate_limited.clone(),
    }
}

fn fast_pool() -> UpstreamPool {
    UpstreamPool::new(BackoffPolicy {
        max_retries: 2,
        base: Duration::from_millis(5),
        max_delay: Duration::from_millis(20),
    })
}

#[tokio::test]
async fn luckin_valid_token_lists_and_calls() {
    let mock = MockMerchant::start(profiles::luckin::config("good")).await;
    let target = target("cn.lkcoffee/order", &mock.url(), Some("good"));
    let tools = probe(&target).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "createOrder"));

    let pool = fast_pool();
    let result = pool.call_tool(&target, "previewOrder", None).await.unwrap();
    assert_ne!(result.is_error, Some(true));
    assert_eq!(mock.calls(), vec!["previewOrder"]);
}

#[tokio::test]
async fn luckin_missing_and_invalid_tokens_are_credential_invalid() {
    let mock = MockMerchant::start(profiles::luckin::config("good")).await;
    for token in [None, Some("bad")] {
        let error = probe(&target("cn.lkcoffee/order", &mock.url(), token))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            ConnectErrorCode::CredentialInvalid,
            "{token:?}: {error}"
        );
    }
}

#[tokio::test]
async fn didi_query_key_and_http_200_failures() {
    let mock = MockMerchant::start(profiles::didi::config("k-good")).await;
    let ok = target("cn.didichuxing/ride", &mock.url(), Some("k-good"));
    let tools = probe(&ok).await.unwrap();
    assert_eq!(tools.len(), 2);

    let missing = probe(&target("cn.didichuxing/ride", &mock.url(), None))
        .await
        .unwrap_err();
    assert_eq!(
        missing.code,
        ConnectErrorCode::CredentialInvalid,
        "{missing}"
    );
    let invalid = probe(&target("cn.didichuxing/ride", &mock.url(), Some("k-bad")))
        .await
        .unwrap_err();
    assert_eq!(
        invalid.code,
        ConnectErrorCode::CredentialInvalid,
        "{invalid}"
    );
}

#[tokio::test]
async fn mcd_falls_back_to_supported_protocol_version() {
    let mock = MockMerchant::start(profiles::mcd::config("m-good")).await;
    let target = target("cn.mcd/order", &mock.url(), Some("m-good"));
    let pool = fast_pool();
    let session = pool.session(&target).await.unwrap();
    assert_eq!(session.protocol_version(), "2025-06-18");
    let versions: Vec<String> = mock
        .requests()
        .iter()
        .filter(|request| request["method"] == "initialize")
        .map(|request| {
            request["params"]["protocolVersion"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(versions, vec!["2025-11-25", "2025-06-18"]);

    let missing = probe(&target_with_token(&mock, None)).await.unwrap_err();
    assert_eq!(missing.code, ConnectErrorCode::CredentialInvalid);
    let invalid = probe(&target_with_token(&mock, Some("m-bad")))
        .await
        .unwrap_err();
    assert_eq!(invalid.code, ConnectErrorCode::CredentialInvalid);
}

fn target_with_token(mock: &MockMerchant, token: Option<&str>) -> UpstreamTarget {
    target("cn.mcd/order", &mock.url(), token)
}

#[tokio::test]
async fn rate_limits_are_retried_then_reported() {
    let mock = MockMerchant::start(profiles::mcd::config("m-good")).await;
    let target = target("cn.mcd/order", &mock.url(), Some("m-good"));
    let pool = fast_pool();

    mock.rate_limit_next(2);
    pool.call_tool(&target, "calculate-price", None)
        .await
        .unwrap();
    assert_eq!(mock.calls(), vec!["calculate-price"]);

    mock.rate_limit_next(3);
    let error = pool
        .call_tool(&target, "calculate-price", None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ConnectErrorCode::RateLimited);

    let didi = MockMerchant::start(profiles::didi::config("k")).await;
    let didi_target = self::target("cn.didichuxing/ride", &didi.url(), Some("k"));
    didi.rate_limit_next(1);
    pool.call_tool(&didi_target, "taxi_estimate", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn expired_token_mid_session_is_detected() {
    let mock = MockMerchant::start(profiles::luckin::config("good")).await;
    let target = target("cn.lkcoffee/order", &mock.url(), Some("good"));
    let pool = fast_pool();
    pool.call_tool(&target, "queryShopList", None)
        .await
        .unwrap();
    mock.set_valid_token(Some("rotated"));
    let error = pool
        .call_tool(&target, "queryShopList", None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ConnectErrorCode::CredentialInvalid, "{error}");
}

#[tokio::test]
async fn unreachable_merchant_is_unavailable() {
    let error = probe(&target(
        "cn.lkcoffee/order",
        "http://127.0.0.1:9/mcp",
        Some("x"),
    ))
    .await
    .unwrap_err();
    assert_eq!(error.code, ConnectErrorCode::UpstreamUnavailable, "{error}");
}
