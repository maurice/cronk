//! Exercise the real CLI in subprocesses so environment changes cannot race other tests.
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose};
use rustls::{ServerConfig, ServerConnection, StreamOwned, pki_types::PrivatePkcs8KeyDer};
use std::{
    ffi::OsStr,
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tempfile::TempDir;

fn ca() -> CertifiedIssuer<'static, KeyPair> {
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap()
}

struct HttpsMock {
    url: String,
    stop: Arc<AtomicBool>,
    authenticated: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl HttpsMock {
    fn new(ca: &CertifiedIssuer<'_, KeyPair>, hostname: &str) -> Self {
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec![hostname.to_owned()])
            .unwrap()
            .signed_by(&key, ca)
            .unwrap();
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![cert.der().clone()],
                    PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
                )
                .unwrap();
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        );
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let authenticated = Arc::new(AtomicUsize::new(0));
        let count = authenticated.clone();
        let worker = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let (socket, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("TLS mock accept: {error}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut stream =
                    StreamOwned::new(ServerConnection::new(config.clone()).unwrap(), socket);
                let mut request = Vec::new();
                loop {
                    let mut byte = [0];
                    // Unknown CAs and hostname mismatches intentionally abort TLS.
                    if stream.read_exact(&mut byte).is_err() {
                        break;
                    }
                    request.push(byte[0]);
                    assert!(request.len() < 32 * 1024);
                    if request.ends_with(b"\r\n\r\n") {
                        let request = String::from_utf8(request).unwrap();
                        assert!(request.starts_with("GET /api/v4/user HTTP/1.1\r\n"));
                        assert!(
                            request
                                .to_lowercase()
                                .contains("private-token: test-private-secret\r\n")
                        );
                        count.fetch_add(1, Ordering::SeqCst);
                        let body = r#"{"id":1,"username":"corporate-user"}"#;
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
                        stream.flush().unwrap();
                        break;
                    }
                }
            }
        });
        Self {
            url,
            stop,
            authenticated,
            worker: Some(worker),
        }
    }
}

impl Drop for HttpsMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn check(url: &str, extra_ca: Option<&OsStr>) -> Output {
    check_with_native_roots(url, extra_ca, None)
}

fn check_with_native_roots(
    url: &str,
    extra_ca: Option<&OsStr>,
    native_ca: Option<&OsStr>,
) -> Output {
    let dir = TempDir::new().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cronk"));
    command
        .args(["--check", "--host", url, "--config"])
        .arg(dir.path().join("config.toml"))
        .env("GITLAB_TOKEN", "test-private-secret")
        .env("NO_PROXY", "*")
        .env("no_proxy", "*")
        .env_remove("NODE_EXTRA_CA_CERTS");
    if let Some(path) = extra_ca {
        command.env("NODE_EXTRA_CA_CERTS", path);
    }
    if let Some(path) = native_ca {
        // rustls-native-certs uses SSL_CERT_FILE as a native trust-store override.
        command
            .env("SSL_CERT_FILE", path)
            .env("SSL_CERT_DIR", dir.path());
    }
    command.output().unwrap()
}

fn assert_failure(output: Output, message: &str) {
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(message), "{stderr}");
    assert!(!stderr.contains("test-private-secret"), "{stderr}");
}

#[test]
fn custom_ca_is_required_and_a_multi_certificate_bundle_trusts_each_ca() {
    let first_ca = ca();
    let second_ca = ca();
    let first = HttpsMock::new(&first_ca, "localhost");
    let second = HttpsMock::new(&second_ca, "localhost");
    let dir = TempDir::new().unwrap();
    let bundle = dir.path().join("company-bundle.pem");
    std::fs::write(&bundle, format!("{}{}", first_ca.pem(), second_ca.pem())).unwrap();

    for extra in [None, Some(OsStr::new(""))] {
        assert_failure(check(&first.url, extra), "connection or TLS failure");
    }
    assert_eq!(first.authenticated.load(Ordering::SeqCst), 0);
    for server in [&first, &second] {
        let output = check(&server.url, Some(bundle.as_os_str()));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "Authenticated as @corporate-user"
        );
        assert_eq!(server.authenticated.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn extra_roots_do_not_disable_certificate_or_hostname_verification() {
    let issuer = ca();
    let unrelated = ca();
    let valid_host = HttpsMock::new(&issuer, "localhost");
    let wrong_host = HttpsMock::new(&issuer, "different.example");
    let dir = TempDir::new().unwrap();
    let bundle = dir.path().join("company.pem");
    std::fs::write(&bundle, unrelated.pem()).unwrap();
    assert_failure(
        check(&valid_host.url, Some(bundle.as_os_str())),
        "connection or TLS failure",
    );
    std::fs::write(&bundle, issuer.pem()).unwrap();
    assert_failure(
        check(&wrong_host.url, Some(bundle.as_os_str())),
        "connection or TLS failure",
    );
    assert_eq!(valid_host.authenticated.load(Ordering::SeqCst), 0);
    assert_eq!(wrong_host.authenticated.load(Ordering::SeqCst), 0);
}

#[test]
fn extra_ca_bundle_supplements_native_roots() {
    let native_ca = ca();
    let extra_ca = ca();
    let server = HttpsMock::new(&native_ca, "localhost");
    let dir = TempDir::new().unwrap();
    let native_bundle = dir.path().join("native.pem");
    let extra_bundle = dir.path().join("extra.pem");
    std::fs::write(&native_bundle, native_ca.pem()).unwrap();
    std::fs::write(&extra_bundle, extra_ca.pem()).unwrap();
    for extra in [None, Some(extra_bundle.as_os_str())] {
        let output = check_with_native_roots(&server.url, extra, Some(native_bundle.as_os_str()));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(server.authenticated.load(Ordering::SeqCst), 2);
}

#[test]
fn unreadable_empty_and_malformed_bundles_fail_at_initialization() {
    let dir = TempDir::new().unwrap();
    let bundle = dir.path().join("company.pem");
    let url = "https://localhost:1";
    assert_failure(
        check(url, Some(bundle.as_os_str())),
        "Could not read the PEM CA bundle from NODE_EXTRA_CA_CERTS",
    );
    for contents in [
        "",
        "not a PEM certificate",
        "-----BEGIN PRIVATE KEY-----\nAQID\n-----END PRIVATE KEY-----\n",
    ] {
        std::fs::write(&bundle, contents).unwrap();
        assert_failure(
            check(url, Some(bundle.as_os_str())),
            "NODE_EXTRA_CA_CERTS must contain at least one PEM certificate",
        );
    }
    std::fs::write(
        &bundle,
        "-----BEGIN CERTIFICATE-----\n!invalid!\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    assert_failure(
        check(url, Some(bundle.as_os_str())),
        "Invalid PEM CA bundle in NODE_EXTRA_CA_CERTS",
    );
    // Valid base64/PEM containing invalid DER must also be rejected when roots are loaded.
    std::fs::write(
        &bundle,
        "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    assert_failure(
        check(url, Some(bundle.as_os_str())),
        "check native roots and NODE_EXTRA_CA_CERTS",
    );
}
