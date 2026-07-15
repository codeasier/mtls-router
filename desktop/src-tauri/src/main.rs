#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "--verify-manager-handshake")
    {
        if args.len() != 1 {
            eprintln!("--verify-manager-handshake accepts no arguments");
            std::process::exit(2);
        }
        if let Err(error) = mtls_router_desktop_lib::verify_manager_handshake() {
            eprintln!("manager handshake verification failed: {error}");
            std::process::exit(1);
        }
        println!(
            "verified manager handshake version={} deployment_id={} protocol={} target={}",
            env!("MTLS_MANAGER_VERSION"),
            env!("MTLS_DEPLOYMENT_ID"),
            env!("MTLS_MANAGEMENT_PROTOCOL_VERSION"),
            env!("MTLS_MANAGER_TARGET")
        );
        return;
    }
    mtls_router_desktop_lib::run();
}
