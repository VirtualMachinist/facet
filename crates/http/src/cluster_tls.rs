//! Verified, origin-scoped client-certificate transport. No credential bytes are
//! exposed to request serialization, recording or Debug output.

use reqwest::{Certificate, Identity, Url};

use crate::HttpError;

/// Client-certificate credentials restricted to one HTTPS origin.
#[derive(Clone)]
pub struct ClusterTls {
    pub(crate) origin: Url,
    pub(crate) roots: Vec<Certificate>,
    pub(crate) identity: Identity,
}

impl std::fmt::Debug for ClusterTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterTls")
            .field("origin", &self.origin.as_str())
            .finish_non_exhaustive()
    }
}

impl ClusterTls {
    /// Parses an HTTPS server origin, trusted CA PEM bundle, and combined client
    /// certificate/private-key PEM. Rejects insecure or ambiguous server URLs.
    pub fn from_pem(server: &str, ca: &[u8], client_identity: &[u8]) -> Result<Self, HttpError> {
        let origin = Url::parse(server).map_err(|_| configuration("invalid cluster server URL"))?;
        if origin.scheme() != "https"
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.path() != "/"
        {
            return Err(configuration(
                "cluster server must be an HTTPS origin without userinfo, path, query or fragment",
            ));
        }
        let roots = Certificate::from_pem_bundle(ca)
            .map_err(|_| configuration("invalid cluster CA PEM bundle"))?;
        if roots.is_empty() {
            return Err(configuration("cluster CA PEM bundle is empty"));
        }
        let identity = Identity::from_pem(client_identity)
            .map_err(|_| configuration("invalid cluster client certificate/private-key PEM"))?;
        Ok(Self {
            origin,
            roots,
            identity,
        })
    }

    pub(crate) fn permits(&self, url: &Url) -> bool {
        url.origin() == self.origin.origin()
            && url.username().is_empty()
            && url.password().is_none()
    }
}

pub(crate) fn configuration(message: &str) -> HttpError {
    HttpError::ClientConfiguration(message.to_owned())
}
