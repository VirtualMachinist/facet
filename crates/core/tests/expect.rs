use probe_core::{StatusExpectation, evaluate_expectations};

#[test]
fn parses_a_single_status_code() {
    let expectation = StatusExpectation::parse("status=200").expect("expression should parse");
    assert_eq!(expectation.expr(), "status=200");
    let passed = expectation.evaluate(200);
    assert!(passed.ok);
    assert_eq!(passed.actual, 200);
    let failed = expectation.evaluate(404);
    assert!(!failed.ok);
    assert_eq!(failed.actual, 404);
    assert_eq!(failed.expr, "status=200");
}

#[test]
fn parses_alternate_status_codes() {
    let expectation =
        StatusExpectation::parse("status=200|201|204").expect("alternates should parse");
    assert!(expectation.evaluate(200).ok);
    assert!(expectation.evaluate(201).ok);
    assert!(expectation.evaluate(204).ok);
    assert!(!expectation.evaluate(404).ok);
}

#[test]
fn trims_whitespace_around_codes() {
    let expectation =
        StatusExpectation::parse(" status=200 | 201 ").expect("whitespace should be ignored");
    assert!(expectation.evaluate(201).ok);
    assert_eq!(expectation.expr(), " status=200 | 201 ");
}

#[test]
fn rejects_unsupported_expressions() {
    for expr in [
        "",
        "200",
        "status",
        "status=",
        "status=200|",
        "status=|200",
        "status=2xx",
        "status=99",
        "status=600",
        "Status=200",
        "header:content-type~json",
    ] {
        assert!(
            StatusExpectation::parse(expr).is_err(),
            "unexpectedly accepted {expr:?}"
        );
    }
}

#[test]
fn evaluates_all_expectations() {
    let expectations = [
        StatusExpectation::parse("status=200").unwrap(),
        StatusExpectation::parse("status=200|201").unwrap(),
    ];
    let outcomes = evaluate_expectations(&expectations, 201);
    assert!(!outcomes[0].ok);
    assert!(outcomes[1].ok);
    assert_eq!(outcomes[0].actual, 201);
    assert_eq!(outcomes[1].actual, 201);
}
