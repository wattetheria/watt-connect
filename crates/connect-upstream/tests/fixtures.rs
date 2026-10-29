//! Classify the recorded merchant responses in `fixtures/upstream` with the
//! rules from the real catalog entries.

use connect_catalog::bundled::bundled_catalog;
use connect_types::{ObservedResponse, ServiceId};
use connect_upstream::classify::{FailureRules, classify};
use serde_json::Value;
use std::path::Path;

fn rules(service: &str) -> FailureRules {
    let catalog = bundled_catalog();
    let entry = catalog.get(&ServiceId::parse(service).unwrap()).unwrap();
    FailureRules {
        auth_failure: entry.connect().auth_failure.clone(),
        rate_limited: entry.connect().rate_limited.clone(),
    }
}

#[test]
fn recorded_responses_classify_as_expected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/upstream");
    let mut checked = 0;
    for (dir, service) in [
        ("didi", "cn.didichuxing/ride"),
        ("luckin", "cn.lkcoffee/order"),
        ("mcd", "cn.mcd/order"),
    ] {
        let rules = rules(service);
        let mut files: Vec<_> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for path in files {
            let fixture: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let observed = ObservedResponse {
                http_status: fixture["http_status"]
                    .as_u64()
                    .map(|s| u16::try_from(s).unwrap()),
                content_type: fixture["content_type"].as_str().map(str::to_owned),
                body: fixture["body"].as_str().map(str::to_owned),
                jsonrpc_error_code: fixture["jsonrpc_error_code"].as_i64(),
                jsonrpc_error_message: fixture["jsonrpc_error_message"].as_str().map(str::to_owned),
                retry_after: None,
            };
            let code = classify(&observed, &rules);
            assert_eq!(
                code.as_str(),
                fixture["expected"].as_str().unwrap(),
                "{}",
                path.display()
            );
            checked += 1;
        }
    }
    assert!(checked >= 9);
}

#[test]
fn rules_do_not_leak_across_merchants() {
    // Didi's "200 + Missing KEY" must not count as auth failure for Luckin.
    let observed = ObservedResponse {
        http_status: Some(200),
        body: Some("Missing KEY parameter".into()),
        ..ObservedResponse::default()
    };
    assert_eq!(
        classify(&observed, &rules("cn.lkcoffee/order")).as_str(),
        "UPSTREAM_ERROR"
    );
}
