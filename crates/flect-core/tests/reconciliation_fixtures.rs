use flect_core::{Alignment, EchoedSpec, IntendedSpec, reconcile};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    intended: IntendedSpec,
    echoed: EchoedSpec,
    expected: Alignment,
}

#[test]
fn deterministic_reconciliation_cases_match_expectations() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../../../fixtures/reconciliation/cases.json")).unwrap();

    for case in cases {
        assert_eq!(
            reconcile(&case.intended, &case.echoed).alignment,
            case.expected,
            "fixture: {}",
            case.name
        );
    }
}
