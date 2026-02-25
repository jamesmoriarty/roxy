use std::{
    net::{TcpListener, TcpStream},
    io,
    io::{Read, Write},
    env,
};
use tracing::info_span;
use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace::{TracerProvider, Config}, Resource};
use opentelemetry_semantic_conventions::resource::SERVICE_NAME;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, fmt::format::FmtSpan};

fn init_tracing() -> TracerProvider {
    let endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4318".to_string());

    let service_name = env::var("OTEL_SERVICE_NAME")
        .unwrap_or_else(|_| "roxy".to_string());

    let exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(endpoint)
        .build_span_exporter()
        .expect("failed to create OTLP span exporter");

    let resource = Resource::new(vec![KeyValue::new(SERVICE_NAME, service_name)]);
    let config = Config::default().with_resource(resource);

    let provider = TracerProvider::builder()
        .with_config(config)
        .with_simple_exporter(exporter)
        .build();

    opentelemetry::global::set_tracer_provider(provider.clone());
    let tracer = provider.tracer("roxy");

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".parse().unwrap()),
        )
        // Stdout: print span close events so durations and attributes are visible
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::CLOSE))
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .init();

    provider
}

fn main() {
    let _provider = init_tracing();
    start();
    opentelemetry::global::shutdown_tracer_provider();
}

fn get_bind_address() -> String {
    env::var("ROXY_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string())
}

fn start() {
    let bind_addr = get_bind_address();
    tracing::info!(bind_addr, "starting proxy server");
    match TcpListener::bind(&bind_addr) {
        Ok(l) => {
            l.set_nonblocking(false).unwrap();
            run(l)
        }
        Err(e) => tracing::error!(bind_addr, error = %e, "failed to bind"),
    }
}

fn run(listener: TcpListener) {
    for stream in listener.incoming() {
        let handle = std::thread::spawn(move || {
            if let Err(e) = handle_connection(stream.expect("handle connection failed")) {
                tracing::error!(error = %e, "connection error");
            }
        });
        handle.join().unwrap();

        #[cfg(test)]
        break;
    }
}

fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    let span = info_span!("connection", peer = ?stream.peer_addr().ok());
    let _enter = span.enter();

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let mut buffer = [0; 512];
    match stream.read(&mut buffer) {
        Ok(n) => {
            tracing::trace!(raw = %String::from_utf8_lossy(&buffer[..n]));
            match req.parse(&buffer[..n]) {
                Ok(httparse::Status::Complete(_)) => {
                    tracing::debug!(method = ?req.method, path = ?req.path, "parsed request");
                    match req.method {
                        Some("GET")     => handle_get(&mut stream, &req),
                        Some("CONNECT") => handle_connect(&mut stream, &req),
                        Some(method) => {
                            tracing::warn!(method, "unsupported method");
                            let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\nMethod Not Allowed");
                            Ok(())
                        }
                        None => {
                            tracing::warn!("no method in request");
                            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nBad Request");
                            Ok(())
                        }
                    }
                }
                Ok(httparse::Status::Partial) => {
                    tracing::warn!("incomplete request");
                    Err(io::Error::new(io::ErrorKind::InvalidData, "incomplete request"))
                }
                Err(e) => {
                    tracing::error!(error = %e, "parse failed");
                    Err(io::Error::new(io::ErrorKind::InvalidData, "parse failed"))
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "read failed");
            Err(e)
        }
    }
}

fn handle_get(stream: &mut TcpStream, req: &httparse::Request) -> io::Result<()> {
    let method = req.method.unwrap_or("GET");
    let path   = req.path.unwrap_or("/");

    let span = info_span!(
        "http.get",
        "http.method" = method,
        "url.path"    = path,
        "server.address"         = tracing::field::Empty,
        "server.port"            = tracing::field::Empty,
        "http.response.body.size" = tracing::field::Empty,
    );
    let _enter = span.enter();

    let host_hdr = req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("Host"));
    let host = match host_hdr.and_then(|h| std::str::from_utf8(h.value).ok()) {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::error!("missing Host header");
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nMissing Host header");
            return Ok(());
        }
    };

    let (hostname, port) = if let Some(colon) = host.rfind(':') {
        (&host[..colon], host[colon + 1..].parse::<i64>().unwrap_or(80))
    } else {
        (host, 80i64)
    };
    span.record("server.address", hostname);
    span.record("server.port", port);

    let target = if host.contains(':') { host.to_string() } else { format!("{}:80", host) };

    tracing::debug!(%target, "connecting to upstream");

    match TcpStream::connect(&target) {
        Ok(mut remote) => {
            let _ = remote.set_nonblocking(false);

            let mut path = req.path.unwrap_or("/").to_string();
            if path.starts_with("http://") {
                path = path[7..].find('/').map_or("/".into(), |pos| path[7 + pos..].to_string());
            }

            let method  = req.method.unwrap_or("GET");
            let version = match req.version { Some(1) => "HTTP/1.1", Some(0) => "HTTP/1.0", _ => "HTTP/1.1" };

            let mut request_buf = format!("{} {} {}\r\n", method, path, version);
            for header in req.headers.iter() {
                if header.name.eq_ignore_ascii_case("Proxy-Connection") { continue; }
                if let Ok(val) = std::str::from_utf8(header.value) {
                    request_buf.push_str(&format!("{}: {}\r\n", header.name, val));
                }
            }
            request_buf.push_str("Connection: close\r\n\r\n");

            if let Err(e) = remote.write_all(request_buf.as_bytes()) {
                tracing::error!(error = %e, "write to upstream failed");
                let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\nBad Gateway");
                return Err(e);
            }

            tracing::info!(%path, %target, "forwarding request");

            let mut buf = [0u8; 8192];
            let mut bytes_relayed = 0u64;
            loop {
                match remote.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        bytes_relayed += n as u64;
                        if let Err(e) = stream.write_all(&buf[..n]) {
                            tracing::error!(error = %e, "write to client failed");
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "read from upstream failed");
                        break;
                    }
                }
            }

            span.record("http.response.body.size", bytes_relayed);
            tracing::debug!(bytes_relayed, "response relayed");
            Ok(())
        }
        Err(e) => {
            tracing::error!(error = %e, %target, "upstream connection failed");
            let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\nBad Gateway");
            Err(e)
        }
    }
}

fn handle_connect(stream: &mut TcpStream, req: &httparse::Request) -> io::Result<()> {
    let target = if let Some(path) = req.path {
        if !path.is_empty() { path.to_string() } else { host_header(req)? }
    } else {
        host_header(req)?
    };

    let (hostname, port) = if let Some(colon) = target.rfind(':') {
        (&target[..colon], target[colon + 1..].parse::<i64>().unwrap_or(443))
    } else {
        (target.as_str(), 443i64)
    };

    let span = info_span!(
        "http.connect",
        "server.address"     = hostname,
        "server.port"        = port,
        "network.transport"  = "tcp",
        "tunnel.bytes_sent"     = tracing::field::Empty,
        "tunnel.bytes_received" = tracing::field::Empty,
    );
    let _enter = span.enter();

    tracing::debug!(%target, "connecting to tunnel target");

    match TcpStream::connect(&target) {
        Ok(remote) => {
            tracing::info!("tunnel established");

            if let Err(e) = stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n") {
                tracing::error!(error = %e, "write CONNECT response failed");
                return Err(e);
            }

            let (c1, c2) = (stream.try_clone()?, stream.try_clone()?);
            let (r1, r2) = (remote.try_clone()?, remote.try_clone()?);

            let t1 = std::thread::spawn(move || {
                let mut c = c1; let mut r = r1;
                let res = io::copy(&mut c, &mut r);
                let _ = r.shutdown(std::net::Shutdown::Write);
                res
            });
            let t2 = std::thread::spawn(move || {
                let mut r = r2; let mut c = c2;
                let res = io::copy(&mut r, &mut c);
                let _ = c.shutdown(std::net::Shutdown::Write);
                res
            });

            let bytes_sent     = t1.join().ok().and_then(|r| r.ok()).unwrap_or(0);
            let bytes_received = t2.join().ok().and_then(|r| r.ok()).unwrap_or(0);

            span.record("tunnel.bytes_sent",     bytes_sent);
            span.record("tunnel.bytes_received", bytes_received);
            tracing::info!(bytes_sent, bytes_received, "tunnel closed");
            Ok(())
        }
        Err(e) => {
            tracing::error!(error = %e, %target, "tunnel connection failed");
            let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\nBad Gateway");
            Err(e)
        }
    }
}

/// Extract the Host header value, returning a 400 error if absent.
fn host_header(req: &httparse::Request) -> io::Result<String> {
    req.headers.iter()
        .find(|h| h.name.eq_ignore_ascii_case("Host"))
        .and_then(|h| std::str::from_utf8(h.value).ok())
        .map(|s| s.to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Host header"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http() {
        let handle = std::thread::spawn(move || { start(); });
        std::thread::sleep(std::time::Duration::from_millis(200));

        let output = std::process::Command::new("curl")
            .args(["-x", "127.0.0.1:8080", "http://github.com/jamesmoriarty", "-v"])
            .output().expect("failed to execute curl");

        let combined = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(combined.contains("Connected to 127.0.0.1"), "did not see expected connection line: {}", combined);
        assert!(combined.contains("HTTP/1.1 301") || combined.contains("HTTP/1.0 301"), "did not see 301 response: {}", combined);
        assert!(combined.contains("Location: https://github.com/jamesmoriarty"), "did not see Location header: {}", combined);
        assert!(combined.to_lowercase().contains("connection: close"), "did not see connection: close: {}", combined);
        handle.join().unwrap();
    }

    #[test]
    fn test_https() {
        let handle = std::thread::spawn(move || { start(); });
        std::thread::sleep(std::time::Duration::from_millis(200));

        let output = std::process::Command::new("curl")
            .args(["-x", "127.0.0.1:8080", "https://github.com/jamesmoriarty", "-v", "-o", "/dev/null"])
            .output().expect("failed to execute curl");

        let combined = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(combined.contains("Connected to 127.0.0.1"), "did not see expected connection line: {}", combined);
        assert!(combined.contains("> CONNECT github.com:443 HTTP/1.1") || combined.contains("CONNECT github.com:443 HTTP/1.1"), "did not see CONNECT: {}", combined);
        assert!(combined.contains("HTTP/1.1 200 Connection Established") || combined.contains("HTTP/1.1 200"), "did not see 200: {}", combined);
        assert!(combined.contains("HTTP/2 200") || combined.contains("HTTP/1.1 200"), "did not see inner 200: {}", combined);
        assert!(combined.to_lowercase().contains("ssl connection") || combined.to_lowercase().contains("tls handshake") || combined.to_lowercase().contains("alpn"), "did not see TLS info: {}", combined);
        handle.join().unwrap();
    }
}
