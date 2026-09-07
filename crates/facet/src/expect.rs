//! `--expect <codes>`: the assertion an agent asks for after a real run.
//!
//! Exit **1** (`expect_failed`) when the HTTP exchange completed and the
//! status is not in the set. Transport, timeout, and cancel stay 6 so a
//! harness that maps "6 → retry" never retries a 404, and one that maps
//! "1 → stop, read the run" never parses a category first. The full success
//! document is still emitted (run id in hand), plus `error`.

use serde_json::{Value, json};

use crate::{ASSERTION_EXIT_CODE, FacetError};

/// Tag stamped on a recorded run that missed its expectation.
pub(crate) const EXPECT_FAIL_TAG: &str = "expect:fail";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Code(u16),
    Class(u16),
}

/// A parsed `--expect` specification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Expectation {
    tokens: Vec<Token>,
}

impl Expectation {
    /// Parses `200,201` or class shorthands `2xx`, `3xx` (mixable).
    pub(crate) fn parse(spec: &str) -> Result<Self, FacetError> {
        let mut tokens = Vec::new();
        for raw in spec.split(',') {
            let item = raw.trim();
            let token = match item.strip_suffix("xx") {
                Some(class) => class
                    .parse::<u16>()
                    .ok()
                    .filter(|class| (1..=5).contains(class))
                    .map(Token::Class),
                None => item
                    .parse::<u16>()
                    .ok()
                    .filter(|code| (100..=599).contains(code))
                    .map(Token::Code),
            };
            let Some(token) = token else {
                return Err(FacetError::invalid_arguments(format!(
                    "--expect expects HTTP status codes or classes like 200,201 or 2xx, got {item:?}"
                )));
            };
            tokens.push(token);
        }
        Ok(Self { tokens })
    }

    /// Whether `status` satisfies the expectation.
    pub(crate) fn matches(&self, status: u16) -> bool {
        self.tokens.iter().any(|token| match token {
            Token::Code(code) => *code == status,
            Token::Class(class) => status / 100 == *class,
        })
    }

    /// The tokens as given: numbers for codes, strings for classes.
    pub(crate) fn json(&self) -> Value {
        Value::Array(
            self.tokens
                .iter()
                .map(|token| match token {
                    Token::Code(code) => json!(code),
                    Token::Class(class) => json!(format!("{class}xx")),
                })
                .collect(),
        )
    }

    /// The `expect_failed` error for a completed exchange with `status`.
    pub(crate) fn failure(&self, status: u16) -> FacetError {
        FacetError::expect_failed(
            format!("expected status in {}, got {status}", self.json()),
            json!({ "expected": self.json(), "actual": status }),
        )
    }

    /// `None` when the exchange did not complete (transport family keeps its
    /// own exit code) or the status is in the set.
    pub(crate) fn miss(&self, status: Option<u16>) -> Option<FacetError> {
        match status {
            Some(status) if !self.matches(status) => Some(self.failure(status)),
            _ => None,
        }
    }
}

impl FacetError {
    pub(crate) fn expect_failed(message: String, details: Value) -> Self {
        Self {
            category: "expect_failed",
            message,
            exit_code: ASSERTION_EXIT_CODE,
            details: Some(details),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Expectation;

    #[test]
    fn parses_codes_and_classes() {
        let expect = Expectation::parse("200, 201,3xx").unwrap();
        assert!(expect.matches(200));
        assert!(expect.matches(201));
        assert!(expect.matches(302));
        assert!(!expect.matches(204));
        assert_eq!(expect.json().to_string(), r#"[200,201,"3xx"]"#);
        assert!(expect.miss(None).is_none(), "transport stays 6");
        assert!(expect.miss(Some(200)).is_none());
        let error = expect.miss(Some(500)).unwrap();
        assert_eq!(error.category, "expect_failed");
        assert_eq!(error.exit_code, 1);
        assert_eq!(error.details.unwrap()["actual"], 500);
    }

    #[test]
    fn rejects_bad_tokens() {
        for bad in ["20x", "6xx", "0xx", "999", "", "200,", "ok"] {
            assert!(Expectation::parse(bad).is_err(), "{bad:?}");
        }
    }
}
