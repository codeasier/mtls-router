use router_contract::{
    cli_state::{self, ProcessRecord},
    process,
};
use router_core::{
    BuildInfo, RouterDefaults, RouterEnv, RouterFlags, RouterSupervisor, SupervisorConfig,
    SupervisorLimits, TlsMaterials,
};
use router_identity::{
    private_files, IdentityError, IdentityStore, IdentityTrust, NativeClient, RELAY_ORIGIN,
};
use serde_json::json;
use std::{
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

const CLIENT_CA: &str = include_str!(concat!(env!("ROUTER_TRUST_DIR"), "/client-auth-ca.pem"));
const SERVER_CA: &str = include_str!(concat!(env!("ROUTER_TRUST_DIR"), "/upstream-ca.pem"));

#[derive(Debug)]
enum Error {
    Arguments,
    Identity(IdentityError),
    Listen,
    State,
    Shutdown,
}
impl Error {
    fn code(&self) -> &'static str {
        match self {
            Self::Arguments => "CLI_INVALID_ARGUMENTS",
            Self::Identity(e) => e.code(),
            Self::Listen => "CLI_LISTEN_FAILED",
            Self::State => "CLI_STATE_UNAVAILABLE",
            Self::Shutdown => "CLI_SHUTDOWN_FAILED",
        }
    }
}
impl From<IdentityError> for Error {
    fn from(value: IdentityError) -> Self {
        Self::Identity(value)
    }
}
type Result<T> = std::result::Result<T, Error>;
struct Options {
    data: PathBuf,
    listen: String,
    no_browser: bool,
    command: Vec<String>,
}
fn options(args: impl Iterator<Item = String>) -> Result<Options> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let xdg = std::env::var("XDG_STATE_HOME").ok();
    let mut value = Options {
        data: std::env::var_os("MTLS_ROUTER_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                home.map(|home| cli_state::default_directory(Path::new(&home), xdg.as_deref()))
            })
            .unwrap_or_default(),
        listen: std::env::var("MTLS_ROUTER_LISTEN").unwrap_or_else(|_| "127.0.0.1:19099".into()),
        no_browser: false,
        command: Vec::new(),
    };
    let mut args = args;
    let mut no_browser_flag = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => value.data = PathBuf::from(args.next().ok_or(Error::Arguments)?),
            "--listen" => value.listen = args.next().ok_or(Error::Arguments)?,
            "--no-browser" | "--no-browser=true" => {
                value.no_browser = true;
                no_browser_flag = true;
            }
            "--no-browser=false" => {
                value.no_browser = false;
                no_browser_flag = true;
            }
            "--version" => value.command.push("--version".into()),
            _ if arg.starts_with('-') => return Err(Error::Arguments),
            _ => value.command.push(arg),
        }
    }
    if value.command == ["--version"] {
        return Ok(value);
    }
    let address: SocketAddr = value.listen.parse().map_err(|_| Error::Arguments)?;
    if address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)
        || address.port() == 0
        || value.data.as_os_str().is_empty()
    {
        return Err(Error::Arguments);
    }
    if no_browser_flag && value.command != ["login"] {
        return Err(Error::Arguments);
    }
    if value.command == ["login"] && !no_browser_flag {
        value.no_browser = match std::env::var("MTLS_ROUTER_NO_BROWSER").ok().as_deref() {
            None | Some("") | Some("false") => false,
            Some("true") => true,
            _ => return Err(Error::Arguments),
        };
    }
    Ok(value)
}
fn print(value: serde_json::Value) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &value).map_err(|_| Error::State)?;
    writeln!(stdout).map_err(|_| Error::State)?;
    stdout.flush().map_err(|_| Error::State)
}

#[tokio::main]
async fn main() {
    let result = match options(std::env::args().skip(1)) {
        Ok(args) => run(args).await,
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        let _ = print(json!({"code":error.code(),"error":"operation could not complete"}));
        std::process::exit(1)
    }
}
async fn run(options: Options) -> Result<()> {
    let command = options
        .command
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if command == ["--version"] {
        return print(
            json!({"version":env!("ROUTER_VERSION"),"commit":env!("ROUTER_COMMIT"),"deployment_id":env!("ROUTER_DEPLOYMENT_ID"),"target":env!("ROUTER_TARGET"),"management_protocol_version":"4","upstream_auth":"per_user_mtls","auth_protocol_version":"1"}),
        );
    }
    if !matches!(
        command.as_slice(),
        ["login"] | ["identity", "status"] | ["identity", "renew"] | ["logout"] | ["serve"]
    ) {
        return Err(Error::Arguments);
    }
    private_files::directory(&options.data)?;
    let trust = IdentityTrust::new(CLIENT_CA.into())?;
    let store = IdentityStore::for_runtime(&options.data, trust)?;
    let client = NativeClient::new(store.clone(), SERVER_CA.into())?;
    match command.as_slice() {
        ["identity", "status"] => {
            print(serde_json::to_value(client.status().await?).map_err(|_| Error::State)?)
        }
        ["identity", "renew"] => {
            print(serde_json::to_value(client.renew().await?).map_err(|_| Error::State)?)
        }
        ["logout"] => {
            print(serde_json::to_value(client.logout().await?).map_err(|_| Error::State)?)
        }
        ["login"] => {
            let authorization = client.connect("linux_cli", "CodeasierRouter Linux").await?;
            let generation = client.status().await?.generation;
            print(
                json!({"verification_uri":authorization.verification_uri,"short_code":authorization.short_code}),
            )?;
            if !options.no_browser
                && (std::env::var_os("DISPLAY").is_some()
                    || std::env::var_os("WAYLAND_DISPLAY").is_some())
            {
                let _ = Command::new("xdg-open")
                    .arg(&authorization.verification_uri)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let summary = client.status().await?;
                if summary.generation != generation {
                    return Err(IdentityError::Conflict.into());
                }
                if summary.phase == "connected" {
                    return print(serde_json::to_value(summary).map_err(|_| Error::State)?);
                }
                match client.poll().await {
                    Ok(summary) if summary.phase == "connected" => {
                        return print(serde_json::to_value(summary).map_err(|_| Error::State)?)
                    }
                    Ok(_) | Err(IdentityError::Busy) | Err(IdentityError::Conflict) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        ["serve"] => serve(&options, store, client).await,
        _ => Err(Error::Arguments),
    }
}
struct RecordGuard {
    path: PathBuf,
    record: ProcessRecord,
}
impl Drop for RecordGuard {
    fn drop(&mut self) {
        if let Ok(Some(data)) = private_files::read(&self.path) {
            if serde_json::from_slice::<ProcessRecord>(&data).ok().as_ref() == Some(&self.record) {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}
async fn serve(options: &Options, store: IdentityStore, client: NativeClient) -> Result<()> {
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| Error::Shutdown)?;
    let _lease = private_files::Lock::acquire(&options.data.join("serve.lock"))?;
    let logger = Arc::new(|entry: router_core::ProxyLog| {
        if let router_core::ProxyLog::Access(record) = entry {
            if let Ok(line) = serde_json::to_string(&record) {
                eprintln!("{line}")
            }
        }
    });
    let supervisor = RouterSupervisor::spawn(SupervisorConfig {
        defaults: RouterDefaults {
            listen_addr: options.listen.clone(),
            upstream_url: RELAY_ORIGIN.into(),
            ..RouterDefaults::default()
        },
        env: RouterEnv::default(),
        flags: RouterFlags::default(),
        materials: TlsMaterials::from_identity(store, SERVER_CA),
        identity: BuildInfo::new(
            env!("ROUTER_VERSION"),
            env!("ROUTER_COMMIT"),
            "unknown",
            env!("ROUTER_DEPLOYMENT_ID"),
        )
        .with_personal_auth(),
        limits: SupervisorLimits::default(),
        logger: Some(logger),
    });
    let address = supervisor.start().await.map_err(|_| Error::Listen)?;
    let identity = process::inspect(std::process::id() as i32).map_err(|_| Error::State)?;
    let record = ProcessRecord::new(
        identity,
        address.clone(),
        env!("ROUTER_VERSION").into(),
        env!("ROUTER_DEPLOYMENT_ID").into(),
    );
    let path = options.data.join(cli_state::STATE_FILE);
    private_files::atomic_write(
        &path,
        &serde_json::to_vec(&record).map_err(|_| Error::State)?,
    )?;
    let _record = RecordGuard { path, record };
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let identity_task = tokio::spawn(async move {
        client.run_maintenance(receiver).await;
    });
    print(json!({"state":"listening","listen_addr":address}))?;
    #[cfg(unix)]
    {
        tokio::select! {_ = term.recv()=>{},_ = tokio::signal::ctrl_c()=>{}}
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.map_err(|_| Error::Shutdown)?;
    }
    let _ = stop.send(true);
    let _ = identity_task.await;
    supervisor.shutdown().await.map_err(|_| Error::Shutdown)?;
    Ok(())
}
