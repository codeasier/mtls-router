use std::net::IpAddr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Listener {
    pub authority: String,
    pub router_base_url: String,
    pub api_base_url: String,
}

pub fn normalize_listener(value: &str) -> Result<Listener, &'static str> {
    let (host, port_text) =
        split_host_port(value).ok_or("listen must be a numeric loopback host:port")?;
    if host.is_empty() || port_text.is_empty() {
        return Err("listen must be a numeric loopback host:port");
    }
    let address: IpAddr = host
        .parse()
        .map_err(|_| "listen must use a numeric 127/8 or ::1 address")?;
    let loopback = match address {
        IpAddr::V4(v4) => v4.octets()[0] == 127,
        IpAddr::V6(v6) => v6.is_loopback(),
    };
    if !loopback {
        return Err("listen must use a numeric 127/8 or ::1 address");
    }
    let port: u16 = port_text
        .parse()
        .map_err(|_| "listen port must be between 1 and 65535")?;
    if port == 0 {
        return Err("listen port must be between 1 and 65535");
    }
    let authority = match address {
        IpAddr::V6(_) => format!("[{address}]:{port}"),
        IpAddr::V4(_) => format!("{address}:{port}"),
    };
    let base = format!("http://{authority}");
    Ok(Listener {
        authority,
        router_base_url: base.clone(),
        api_base_url: format!("{base}/v1"),
    })
}

fn split_host_port(value: &str) -> Option<(&str, &str)> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':')?;
        return Some((host, port));
    }
    value.rsplit_once(':')
}
