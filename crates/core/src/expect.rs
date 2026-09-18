//! Status expectations for a completed HTTP exchange.

use std::{error::Error, fmt};

/// A parsed `status=<code>` or `status=<code|code>` expectation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusExpectation {
    expr: String,
    accepted: Vec<u16>,
}

/// Result of evaluating one status expectation against a completed response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectationOutcome {
    /// The original `--expect` expression.
    pub expr: String,
    /// Whether the response status satisfied the expression.
    pub ok: bool,
    /// The HTTP status returned by a completed exchange.
    pub actual: u16,
}

/// An `--expect` expression that is not a supported status assertion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectationParseError {
    expr: String,
}

impl StatusExpectation {
    /// Parses `status=<code>` or `status=<code|code>`.
    pub fn parse(expr: &str) -> Result<Self, ExpectationParseError> {
        let trimmed = expr.trim();
        let Some(codes) = trimmed.strip_prefix("status=") else {
            return Err(ExpectationParseError {
                expr: expr.to_owned(),
            });
        };
        let accepted = parse_status_codes(codes).ok_or_else(|| ExpectationParseError {
            expr: expr.to_owned(),
        })?;
        Ok(Self {
            expr: expr.to_owned(),
            accepted,
        })
    }

    /// Returns the original expression text.
    #[must_use]
    pub fn expr(&self) -> &str {
        &self.expr
    }

    /// Evaluates this expectation against a completed response status.
    #[must_use]
    pub fn evaluate(&self, status: u16) -> ExpectationOutcome {
        ExpectationOutcome {
            expr: self.expr.clone(),
            ok: self.accepted.contains(&status),
            actual: status,
        }
    }
}

impl ExpectationParseError {
    /// Returns the rejected expression.
    #[must_use]
    pub fn expr(&self) -> &str {
        &self.expr
    }
}

impl fmt::Display for ExpectationParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid --expect expression: {}; expected status=<code> or status=<code|code>",
            self.expr
        )
    }
}

impl Error for ExpectationParseError {}

fn parse_status_codes(source: &str) -> Option<Vec<u16>> {
    let mut accepted = Vec::new();
    for part in source.split('|') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        let code = part.parse().ok()?;
        if !(100..=599).contains(&code) {
            return None;
        }
        accepted.push(code);
    }
    (!accepted.is_empty()).then_some(accepted)
}

/// Evaluates every expectation against a completed response status.
#[must_use]
pub fn evaluate_expectations(
    expectations: &[StatusExpectation],
    status: u16,
) -> Vec<ExpectationOutcome> {
    expectations
        .iter()
        .map(|expectation| expectation.evaluate(status))
        .collect()
}
