use std::net::IpAddr;
use std::path::{Path, PathBuf};

pub fn api_url(router_base_url: &str) -> Result<String, ()> {
    let rest = router_base_url.strip_prefix("http://").ok_or(())?;
    if rest.is_empty() || rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err(());
    }
    let (hostport, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if !path.is_empty() && path != "/" {
        return Err(());
    }
    let (host, port) = split_host_port(hostport)?;
    let ip: IpAddr = host.parse().map_err(|_| ())?;
    if !ip.is_loopback() || port.is_empty() {
        return Err(());
    }
    let host_out = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(format!("http://{host_out}:{port}/v1"))
}

pub fn valid_router_base_url(value: &str) -> bool {
    api_url(value).is_ok()
}

pub fn valid_api_base_url(value: &str) -> bool {
    valid_api_value(value)
}

pub fn valid_api_value(value: &str) -> bool {
    if !value.ends_with("/v1") {
        return false;
    }
    let trimmed = value.strip_suffix("/v1").unwrap_or(value);
    match api_url(trimmed) {
        Ok(api) => api == value.trim_end_matches('/'),
        Err(()) => false,
    }
}

pub fn safe_display_path(path: &str) -> bool {
    if path.is_empty() || !Path::new(path).is_absolute() {
        return false;
    }
    path.chars().all(|ch| !ch.is_control() && ch != '\u{001b}')
}

pub fn display_path(path: &PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

fn split_host_port(hostport: &str) -> Result<(&str, &str), ()> {
    if let Some(rest) = hostport.strip_prefix('[') {
        let close = rest.find(']').ok_or(())?;
        let host = &rest[..close];
        let after = &rest[close + 1..];
        let port = after.strip_prefix(':').ok_or(())?;
        if host.is_empty() || port.is_empty() || port.contains(':') {
            return Err(());
        }
        return Ok((host, port));
    }
    let colon = hostport.rfind(':').ok_or(())?;
    let host = &hostport[..colon];
    let port = &hostport[colon + 1..];
    if host.is_empty() || port.is_empty() || host.contains(':') {
        return Err(());
    }
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_url_accepts_numeric_loopback_only() {
        assert_eq!(
            api_url("http://127.0.0.1:19099").unwrap(),
            "http://127.0.0.1:19099/v1"
        );
        assert_eq!(
            api_url("http://127.0.0.1:19099/").unwrap(),
            "http://127.0.0.1:19099/v1"
        );
        assert_eq!(
            api_url("http://[::1]:19099").unwrap(),
            "http://[::1]:19099/v1"
        );
        assert!(api_url("http://localhost:19099").is_err());
        assert!(api_url("https://127.0.0.1:19099").is_err());
        assert!(api_url("http://127.0.0.1").is_err());
        assert!(api_url("http://127.0.0.1:19099/v1").is_err());
        assert!(api_url("http://user@127.0.0.1:19099").is_err());
        assert!(valid_api_value("http://127.0.0.1:19099/v1"));
        assert!(!valid_api_value("http://127.0.0.1:19099/v1//literal"));
    }
}
