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
}

impl ConnectError {
    pub(super) fn into_startup(self) -> StartupError {
        match self {
            Self::Setup => StartupError::new(StartupReason::ProbeSetupFailed),
            Self::Transport => StartupError::new(StartupReason::UpstreamProbeFailed),
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

pub(super) async fn open_mtls_http1<B>(
    transport: &MtlsTransport,
    url: &Uri,
) -> Result<(SendRequest<B>, tokio::task::JoinHandle<()>), ConnectError>
where
    B: Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let host = url.host().ok_or(ConnectError::Setup)?;
    let port = url.port_u16().unwrap_or(443);
    let server_name = ServerName::try_from(host.to_owned()).map_err(|_| ConnectError::Setup)?;
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|_| ConnectError::Transport)?;
    let tls = transport
        .connector()
        .connect(server_name, tcp)
        .await
        .map_err(|_| ConnectError::Transport)?;
    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|_| ConnectError::Transport)?;
    let connection_task = tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok((sender, connection_task))
}
