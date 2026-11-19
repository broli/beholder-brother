use beholder_brother::brother::cache::DashboardCache;
use beholder_brother::return_today;
use lexopt::prelude::*;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

struct BrotherConfig {
    log_dir: PathBuf,
    bind_addr: SocketAddr,
    force: bool,
}

impl Default for BrotherConfig {
    fn default() -> Self {
        Self {
            log_dir: PathBuf::from("/var/log/beholder"),
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 6060)),
            force: false,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(cfg) = parse_args()? else {
        return Ok(());
    };

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        eprintln!("\nInterrupt received, shutting down brother...");
        r.store(false, Ordering::SeqCst);
    })?;

    let server = Server::http(cfg.bind_addr)
        .map_err(|e| format!("failed to bind {}: {e}", cfg.bind_addr))?;
    println!(
        "brother listening on http://{} (log-dir: {}, force: {})",
        cfg.bind_addr,
        cfg.log_dir.display(),
        cfg.force
    );

    let mut cache = DashboardCache::new();
    // Warm the cache on startup so the first browser load is instant.
    if let Ok(today) = return_today()
        && let Err(err) = cache.build_dashboard(&cfg.log_dir, today, cfg.force)
    {
        eprintln!("Warning: initial dashboard build failed: {err}");
    }

    while running.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(500)) {
            Ok(Some(request)) => {
                handle_request(request, &mut cache, &cfg.log_dir, cfg.force);
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("HTTP accept error: {err}");
                break;
            }
        }
    }

    println!("brother stopped cleanly.");
    Ok(())
}

fn handle_request(request: Request, cache: &mut DashboardCache, log_dir: &Path, force: bool) {
    if request.method() != &Method::Get {
        let resp =
            Response::from_string("405 Method Not Allowed\n").with_status_code(StatusCode(405));
        let _ = request.respond(resp);
        return;
    }

    let path = request.url().split('?').next().unwrap_or("/");
    match path {
        "/" => {
            let today = match return_today() {
                Ok(d) => d,
                Err(err) => {
                    let resp = Response::from_string(format!("500 Internal Server Error: {err}\n"))
                        .with_status_code(StatusCode(500));
                    let _ = request.respond(resp);
                    return;
                }
            };
            match cache.build_dashboard(log_dir, today, force) {
                Ok(html) => {
                    let mut resp = Response::from_string(html);
                    if let Ok(ct) = Header::from_bytes("Content-Type", "text/html; charset=utf-8") {
                        resp.add_header(ct);
                    }
                    if let Ok(nosniff) = Header::from_bytes("X-Content-Type-Options", "nosniff") {
                        resp.add_header(nosniff);
                    }
                    if let Ok(cc) = Header::from_bytes("Cache-Control", "no-store") {
                        resp.add_header(cc);
                    }
                    let _ = request.respond(resp);
                }
                Err(err) => {
                    let resp = Response::from_string(format!("500 Internal Server Error: {err}\n"))
                        .with_status_code(StatusCode(500));
                    let _ = request.respond(resp);
                }
            }
        }
        "/health" => {
            let mut resp = Response::from_string("ok\n");
            if let Ok(ct) = Header::from_bytes("Content-Type", "text/plain; charset=utf-8") {
                resp.add_header(ct);
            }
            let _ = request.respond(resp);
        }
        _ => {
            let resp = Response::from_string("404 Not Found\n").with_status_code(StatusCode(404));
            let _ = request.respond(resp);
        }
    }
}

fn print_usage() {
    println!(
        r#"Usage: brother [OPTIONS]

Options:
    -h, --help              Print this help message
    -v, --version           Print version information
    -l, --log-dir <PATH>    Directory containing beholder logs (default: /var/log/beholder)
    -b, --bind <ADDR>       HTTP listen address (default: 127.0.0.1:6060)
    -f, --force             Bypass cache and reload all log files from disk on each request
"#
    );
}

fn parse_args() -> Result<Option<BrotherConfig>, Box<dyn std::error::Error>> {
    let mut parser = lexopt::Parser::from_env();
    let mut cfg = BrotherConfig::default();

    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                print_usage();
                return Ok(None);
            }
            Short('v') | Long("version") => {
                println!("brother Version {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Short('l') | Long("log-dir") => {
                cfg.log_dir = PathBuf::from(parser.value()?);
            }
            Short('b') | Long("bind") => {
                cfg.bind_addr = parser.value()?.parse()?;
            }
            Short('f') | Long("force") => {
                cfg.force = true;
            }
            _ => return Err(arg.unexpected().into()),
        }
    }
    Ok(Some(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    #[test]
    fn test_http_routes_root_health_and_404() -> Result<(), Box<dyn std::error::Error>> {
        let server = Server::http("127.0.0.1:0").map_err(|e| format!("{e}"))?;
        let addr = server.server_addr().to_ip().ok_or("missing bound ip")?;

        let log_dir =
            std::env::temp_dir().join(format!("brother_http_test_{}", std::process::id()));
        std::fs::create_dir_all(&log_dir)?;
        let log_dir_clone = log_dir.clone();

        let handle = std::thread::spawn(move || {
            let mut cache = DashboardCache::new();
            for _ in 0..3 {
                if let Ok(Some(req)) = server.recv_timeout(Duration::from_secs(2)) {
                    handle_request(req, &mut cache, &log_dir_clone, false);
                }
            }
        });

        let send_get = |path: &str| -> std::io::Result<String> {
            let mut stream = TcpStream::connect(addr)?;
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )?;
            let mut resp = String::new();
            stream.read_to_string(&mut resp)?;
            Ok(resp)
        };

        let root_resp = send_get("/")?;
        assert!(root_resp.starts_with("HTTP/1.1 200"));
        assert!(root_resp.contains("<!doctype html>"));

        let health_resp = send_get("/health")?;
        assert!(health_resp.starts_with("HTTP/1.1 200"));
        assert!(health_resp.ends_with("ok\n"));

        let missing_resp = send_get("/does-not-exist")?;
        assert!(missing_resp.starts_with("HTTP/1.1 404"));

        let _ = handle.join();
        let _ = std::fs::remove_dir_all(&log_dir);
        Ok(())
    }
}
