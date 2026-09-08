//! Ephemeral test-only keys, shared by HTTP and kubeconfig adapter tests.
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};

pub struct Pki {
    pub ca: String,
    pub server_cert: String,
    pub server_key: String,
    pub client_cert: String,
    pub client_key: String,
}
impl Pki {
    pub fn new(names: &[&str]) -> Self {
        let mut ca_params = CertificateParams::default();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "test-cluster-ca");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::from_params(&ca_params, &ca_key);
        let issue = |name: &str, names: Vec<String>, usage| {
            let mut params = CertificateParams::new(names).unwrap();
            params.distinguished_name.push(DnType::CommonName, name);
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![usage];
            let key = KeyPair::generate().unwrap();
            let cert = params.signed_by(&key, &issuer).unwrap();
            (cert.pem(), key.serialize_pem())
        };
        let (server_cert, server_key) = issue(
            "test-server",
            names.iter().map(|v| v.to_string()).collect(),
            ExtendedKeyUsagePurpose::ServerAuth,
        );
        let (client_cert, client_key) = issue(
            "facet-test",
            Vec::new(),
            ExtendedKeyUsagePurpose::ClientAuth,
        );
        Self {
            ca: ca_cert.pem(),
            server_cert,
            server_key,
            client_cert,
            client_key,
        }
    }
    pub fn client_pem(&self) -> String {
        format!("{}\n{}", self.client_cert, self.client_key)
    }
}
