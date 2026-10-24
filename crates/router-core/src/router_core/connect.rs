use http::Uri;
use http_body::Body;
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use rustls_pki_types::ServerName;
use tokio::net::TcpStream;

use super::{MtlsTransport, StartupError, StartupReason};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectError {
    Setup,
    Transport,
    Certificate(super::CertificateState),
}

impl ConnectError {
    pub(super) fn into_startup(self) -> StartupError {
        match self {
            Self::Setup => StartupError::new(StartupReason::ProbeSetupFailed),
            Self::Transport | Self::Certificate(_) => {
                StartupError::new(StartupReason::UpstreamProbeFailed)
            }
        }
    }
}

pub(super) fn origin_form_path(url: &Uri) -> String {
    match url.path_and_query() {
        Some(path) if !path.as_str().is_empty() => path.as_str().to_owned(),
        _ => "/".to_owned(),
    }
}

pub(super) fn upstream_host_header(url: &Uri) -> Result<String, ConnectError> {
    let host = url.host().ok_or(ConnectError::Setup)?;
    let header = match url.port_u16() {
        Some(port) if host.contains(':') => format!("[{host}]:{port}"),
        Some(port) => format!("{host}:{port}"),
        None if host.contains(':') => format!("[{host}]"),
        None => host.to_owned(),
    };
    Ok(header)
}

// A server that rejects our client certificate answers with a certificate
// alert. Local validation of the server chain (unknown issuer, expired,
// hostname mismatch) is not a client-certificate rejection and must stay a
// transport failure so the UI does not blame the device certificate.
fn client_certificate_rejected(error: &std::io::Error) -> bool {
    let Some(error) = error
        .get_ref()
        .and_then(|error| error.downcast_ref::<rustls::Error>())
    else {
        return false;
    };
    match error {
        rustls::Error::AlertReceived(alert) => matches!(
            alert,
            rustls::AlertDescription::BadCertificate
                | rustls::AlertDescription::UnsupportedCertificate
                | rustls::AlertDescription::CertificateRevoked
                | rustls::AlertDescription::CertificateExpired
                | rustls::AlertDescription::CertificateUnknown
                | rustls::AlertDescription::UnknownCA
                | rustls::AlertDescription::AccessDenied
                | rustls::AlertDescription::CertificateRequired
        ),
        _ => false,
    }
}

pub(super) async fn open_mtls_http1<B>(
    transport: &MtlsTransport,
    url: &Uri,
) -> Result<
    (
        SendRequest<B>,
        tokio::task::JoinHandle<()>,
        Option<(u64, String)>,
    ),
    ConnectError,
>
where
    B: Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let host = url.host().ok_or(ConnectError::Setup)?;
    let port = url.port_u16().unwrap_or(443);
    let server_name = ServerName::try_from(host.to_owned()).map_err(|_| ConnectError::Setup)?;
    let (connector, generation) = transport
        .connector_for_request()
        .await
        .map_err(ConnectError::Certificate)?;
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|_| ConnectError::Transport)?;
    let tls = match connector.connect(server_name, tcp).await {
        Ok(tls) => tls,
        Err(error) if transport.is_personal() && client_certificate_rejected(&error) => {
            transport.feedback(generation, true).await;
            return Err(ConnectError::Certificate(super::CertificateState::Rejected));
        }
        Err(_) => return Err(ConnectError::Transport),
    };
    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|_| ConnectError::Transport)?;
    let connection_task = tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok((sender, connection_task, generation))
}

#[cfg(test)]
mod tests {
    use super::client_certificate_rejected;

    fn wrapped(error: rustls::Error) -> std::io::Error {
        std::io::Error::other(error)
    }

    #[test]
    fn peer_certificate_alerts_are_client_rejections() {
        for alert in [
            rustls::AlertDescription::BadCertificate,
            rustls::AlertDescription::UnsupportedCertificate,
            rustls::AlertDescription::CertificateRevoked,
            rustls::AlertDescription::CertificateExpired,
            rustls::AlertDescription::CertificateUnknown,
            rustls::AlertDescription::UnknownCA,
            rustls::AlertDescription::AccessDenied,
            rustls::AlertDescription::CertificateRequired,
        ] {
            assert!(client_certificate_rejected(&wrapped(
                rustls::Error::AlertReceived(alert)
            )));
        }
    }

    #[test]
    fn local_server_certificate_validation_is_not_a_client_rejection() {
        assert!(!client_certificate_rejected(&wrapped(
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer)
        )));
        assert!(!client_certificate_rejected(&std::io::Error::from(
            std::io::ErrorKind::ConnectionReset
        )));
    }
}
