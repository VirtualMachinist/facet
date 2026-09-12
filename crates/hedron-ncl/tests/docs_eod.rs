use hedron_ncl::r#gen::docs_eod::{check_shape, KIND};
use serde_json::json;

#[test]
fn valid_docs_eod_passes() {
    let spec = json!({
        "kind": KIND,
        "date": "2026-08-25",
        "required_briefs": ["alpha", "beta"],
    });
    let parsed = check_shape(&spec).expect("valid docs_eod should pass");
    assert_eq!(parsed.date, "2026-08-25");
    assert_eq!(parsed.required_briefs, ["alpha", "beta"]);
}

#[test]
fn missing_date_fails() {
    let spec = json!({
        "kind": KIND,
        "required_briefs": ["alpha"],
    });
    let err = check_shape(&spec).expect_err("missing date should fail");
    assert!(
        err.message.contains("date"),
        "expected date-related error, got: {}",
        err.message
    );
}

#[test]
fn unknown_kind_fails() {
    let spec = json!({
        "kind": "cluster_ready",
        "replicas": 3,
    });
    let err = check_shape(&spec).expect_err("unknown kind should fail");
    assert!(
        err.message.contains("unknown desired state kind"),
        "expected unknown-kind error, got: {}",
        err.message
    );
}
