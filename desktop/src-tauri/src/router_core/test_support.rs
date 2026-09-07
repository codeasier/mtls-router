use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body::{Body, Frame};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::TlsAcceptor;

use super::{parse_pem_certificates, parse_pem_private_key, TlsMaterials};

pub struct TestCerts {
    pub ca: String,
    pub client_cert: String,
    pub client_key: String,
    pub server_cert: String,
    pub server_key: String,
}

impl TestCerts {
    pub fn generate() -> Self {
        let ca_key = KeyPair::generate().expect("ca key");
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
        ca_params.distinguished_name.push(DnType::CommonName, "ca");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        let ca_cert = ca_params.self_signed(&ca_key).expect("ca cert");

        let (client_cert, client_key) = issue_leaf(&ca_cert, &ca_key, "client");
        let (server_cert, server_key) = issue_leaf(&ca_cert, &ca_key, "server");
        Self {
            ca: ca_cert.pem(),
            client_cert,
            client_key,
            server_cert,
            server_key,
        }
    }

    pub fn materials(&self) -> TlsMaterials {
        TlsMaterials::new(
            self.client_cert.clone(),
            self.client_key.clone(),
            self.ca.clone(),
        )
    }
}

fn issue_leaf(
    ca_cert: &rcgen::Certificate,
    ca_key: &KeyPair,
    common_name: &str,
) -> (String, String) {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("leaf params");
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    params.subject_alt_names = vec![SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let key = KeyPair::generate().expect("leaf key");
    let cert = params.signed_by(&key, ca_cert, ca_key).expect("leaf cert");
    (cert.pem(), key.serialize_pem())
}

pub struct MtlsServerOptions {
    pub status: u16,
    pub delay: Duration,
    pub server_versions: Vec<&'static rustls::SupportedProtocolVersion>,
}

impl Default for MtlsServerOptions {
    fn default() -> Self {
        Self {
            status: 204,
            delay: Duration::ZERO,
            server_versions: vec![&rustls::version::TLS12, &rustls::version::TLS13],
        }
    }
}

pub struct TestMtlsServer {
    pub url: String,
    pub certs: TestCerts,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestMtlsServer {
    pub fn shutdown(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for TestMtlsServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn boxed_bytes(bytes: impl Into<Bytes>) -> BoxBody<Bytes, Infallible> {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed()
}

pub struct BytesChannelBody {
    rx: mpsc::Receiver<Bytes>,
}

impl Body for BytesChannelBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.get_mut().rx.poll_recv(cx) {
            Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub fn boxed_channel(rx: mpsc::Receiver<Bytes>) -> BoxBody<Bytes, Infallible> {
    BytesChannelBody { rx }.boxed()
}

pub async fn start_mtls_handler<H, F>(handler: H) -> TestMtlsServer
where
    H: Fn(Request<Incoming>) -> F + Send + Sync + 'static,
    F: Future<Output = Response<BoxBody<Bytes, Infallible>>> + Send + 'static,
{
    start_mtls_optional_handler(move |request| {
        let response = handler(request);
        async move { Some(response.await) }
    })
    .await
}

/// Like [`start_mtls_handler`], but `None` drops the TLS/HTTP connection after
/// the request is received so the proxy observes an upstream reset.
pub async fn start_mtls_optional_handler<H, F>(handler: H) -> TestMtlsServer
where
    H: Fn(Request<Incoming>) -> F + Send + Sync + 'static,
    F: Future<Output = Option<Response<BoxBody<Bytes, Infallible>>>> + Send + 'static,
{
    let certs = TestCerts::generate();
    let server_certs = parse_pem_certificates(certs.server_cert.as_bytes()).expect("server certs");
    let server_key = parse_pem_private_key(certs.server_key.as_bytes()).expect("server key");
    let mut client_cas = RootCertStore::empty();
    for certificate in parse_pem_certificates(certs.ca.as_bytes()).expect("ca") {
        client_cas.add(certificate).expect("ca root");
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(client_cas))
        .build()
        .expect("client verifier");
    let server_config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("server versions")
            .with_client_cert_verifier(verifier)
            .with_single_cert(server_certs, server_key)
            .expect("server config");
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((tcp, _)) = accepted else { break; };
                    let acceptor = acceptor.clone();
                    let handler = handler.clone();
                    tokio::spawn(async move {
                        let Ok(tls) = acceptor.accept(tcp).await else { return; };
                        let io = TokioIo::new(tls);
                        let service = service_fn(move |request| {
                            let handler = handler.clone();
                            async move {
                                match handler(request).await {
                                    Some(response) => Ok(response),
                                    None => Err(std::io::Error::other("drop upstream response")),
                                }
                            }
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, service)
                            .await;
                    });
                }
            }
        }
    });
    TestMtlsServer {
        url: format!("https://127.0.0.1:{}", addr.port()),
        certs,
        shutdown: Some(shutdown_tx),
    }
}

pub async fn start_mtls_server(options: MtlsServerOptions) -> TestMtlsServer {
    let certs = TestCerts::generate();
    let server_certs = parse_pem_certificates(certs.server_cert.as_bytes()).expect("server certs");
    let server_key = parse_pem_private_key(certs.server_key.as_bytes()).expect("server key");
    let mut client_cas = RootCertStore::empty();
    for certificate in parse_pem_certificates(certs.ca.as_bytes()).expect("ca") {
        client_cas.add(certificate).expect("ca root");
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(client_cas))
        .build()
        .expect("client verifier");
    let server_config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&options.server_versions)
            .expect("server versions")
            .with_client_cert_verifier(verifier)
            .with_single_cert(server_certs, server_key)
            .expect("server config");
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let status = options.status;
    let delay = options.delay;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((tcp, _)) = accepted else { break; };
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        let Ok(tls) = acceptor.accept(tcp).await else { return; };
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        let io = TokioIo::new(tls);
                        let service = service_fn(move |_request| async move {
                            let response = Response::builder()
                                .status(StatusCode::from_u16(status).expect("status"))
                                .body(Full::new(Bytes::new()))
                                .expect("response");
                            Ok::<_, Infallible>(response)
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, service)
                            .await;
                    });
                }
            }
        }
    });
    TestMtlsServer {
        url: format!("https://127.0.0.1:{}", addr.port()),
        certs,
        shutdown: Some(shutdown_tx),
    }
}
