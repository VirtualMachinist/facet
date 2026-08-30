use std::{error::Error, fmt, path::PathBuf};

/// A request-construction, cancellation, timeout, or transport failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpError {
    /// Request method is absent.
    MissingMethod,
    /// Request URL is absent.
    MissingUrl,
    /// Method is outside the Phase 5 HTTP subset.
    UnsupportedMethod(String),
    /// A request header name is invalid.
    InvalidHeaderName(String),
    /// A request header value is invalid.
    InvalidHeaderValue(String),
    /// A body variant could not be selected unambiguously.
    InvalidBodySelection(String),
    /// A body definition is inconsistent with its declared kind.
    InvalidBody(String),
    /// A required authentication property is absent or not a string.
    MissingAuthenticationProperty {
        /// Authentication scheme.
        scheme: &'static str,
        /// Required property.
        property: &'static str,
    },
    /// The parsed authentication kind is not supported in Phase 5.
    UnsupportedAuthentication(String),
    /// A request-body file could not be opened or configured.
    File {
        /// Resolved file path.
        path: PathBuf,
        /// Underlying diagnostic.
        message: String,
    },
    /// The HTTP client could not be configured.
    ClientConfiguration(String),
    /// Reqwest rejected the method, URL, headers, or another request component.
    InvalidRequest(String),
    /// The configured total timeout elapsed.
    Timeout,
    /// Execution was cancelled by the caller.
    Cancelled,
    /// Connection, protocol, or response-body failure.
    Transport(String),
    /// A response body could not be written to its requested destination.
    ResponseOutput {
        /// Requested final output path.
        path: PathBuf,
        /// Underlying diagnostic.
        message: String,
    },
}

impl HttpError {
    /// Returns whether this is a deterministic request-configuration failure.
    #[must_use]
    pub const fn is_configuration(&self) -> bool {
        matches!(
            self,
            Self::MissingMethod
                | Self::MissingUrl
                | Self::UnsupportedMethod(_)
                | Self::InvalidHeaderName(_)
                | Self::InvalidHeaderValue(_)
                | Self::InvalidBodySelection(_)
                | Self::InvalidBody(_)
                | Self::MissingAuthenticationProperty { .. }
                | Self::UnsupportedAuthentication(_)
                | Self::File { .. }
                | Self::ClientConfiguration(_)
                | Self::InvalidRequest(_)
        )
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMethod => formatter.write_str("HTTP request method is not set"),
            Self::MissingUrl => formatter.write_str("HTTP request URL is not set"),
            Self::UnsupportedMethod(method) => {
                write!(formatter, "unsupported HTTP method: {method}")
            }
            Self::InvalidHeaderName(name) => write!(formatter, "invalid header name: {name}"),
            Self::InvalidHeaderValue(name) => write!(formatter, "invalid value for header: {name}"),
            Self::InvalidBodySelection(message) | Self::InvalidBody(message) => {
                formatter.write_str(message)
            }
            Self::MissingAuthenticationProperty { scheme, property } => {
                write!(formatter, "{scheme} authentication requires '{property}'")
            }
            Self::UnsupportedAuthentication(scheme) => {
                write!(formatter, "unsupported authentication scheme: {scheme}")
            }
            Self::File { path, message } => {
                write!(formatter, "cannot read '{}': {message}", path.display())
            }
            Self::ClientConfiguration(message) => {
                write!(formatter, "cannot configure HTTP client: {message}")
            }
            Self::InvalidRequest(message) => {
                write!(formatter, "cannot construct HTTP request: {message}")
            }
            Self::Timeout => formatter.write_str("HTTP request timed out"),
            Self::Cancelled => formatter.write_str("HTTP request was cancelled"),
            Self::Transport(message) => write!(formatter, "HTTP execution failed: {message}"),
            Self::ResponseOutput { path, message } => write!(
                formatter,
                "cannot write response body to '{}': {message}",
                path.display()
            ),
        }
    }
}

impl Error for HttpError {}
