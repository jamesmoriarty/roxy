use std::{
    net::{TcpListener, TcpStream},
    io,
    io::{Read, Write},
    env,
};
use log::{info, trace, warn, debug};

fn main() {
    env_logger::init();
    start();
}

fn get_bind_address() -> String {
    env::var("ROXY_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string())
}

fn start() {
    let bind_addr = get_bind_address();
    info!("Starting proxy server on {}", bind_addr);
    match TcpListener::bind(&bind_addr) {
        Ok(l) => {
            l.set_nonblocking(false).unwrap();
            run(l)
        },
        Err(e) => {
            warn!("Failed to bind to address {}: {}", bind_addr, e);
        }
    };
}


fn run(listener: TcpListener)  {
    for stream in listener.incoming() {
        let handle = std::thread::spawn(move || {
            match handle_connection(stream.expect("handle connection failed")) {
                Ok(_) => info!("Handled connection successfully"),
                Err(e) => warn!("Error handling connection: {}", e),
            }
        });

        handle.join().unwrap();
        
        #[cfg(test)]
        break; // TODO - add test instrument or remove this to keep server running
    }
}

fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let mut buffer = [0; 512];
    match stream.read(&mut buffer) {
        Ok(n) => {
            trace!("Request: {}", String::from_utf8_lossy(&buffer[..n]));
            match req.parse(&buffer[..n]) {
                Ok(httparse::Status::Complete(_)) => {
                    info!("Parsed request successfully");
                    debug!("Request: {:?}", req);
                    match req.method {
                        Some("GET") => {
                            return handle_get(&mut stream, &req)
                        },
                        Some("CONNECT") => {
                            return handle_connect(&mut stream, &req)
                        },
                        Some(method) => {
                            warn!("Unsupported HTTP method: {}", method);
                            let response = "HTTP/1.1 405 Method Not Allowed\r\n\r\nMethod Not Allowed";
                            match stream.write_all(response.as_bytes()) {
                                Ok(_) => return Ok(()),
                                Err(e) => {
                                    warn!("Failed to write response: {}", e);
                                    return Err(e);
                                }
                            }
                        },
                        None => {
                            warn!("No HTTP method found in request");
                            let response = "HTTP/1.1 400 Bad Request\r\n\r\nBad Request";
                            match stream.write_all(response.as_bytes()) {
                                Ok(_) => return Ok(()),
                                Err(e) => {
                                    warn!("Failed to write response: {}", e);
                                    return Err(e);
                                }
                            }
                        }
                    }
                },
                Ok(httparse::Status::Partial) => {
                    warn!("Incomplete request received");
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Incomplete request"));
                },
                Err(e) => {
                    warn!("Failed to parse request: {}", e);
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Failed to parse request"));
                }
            }
        },
        Err(e) => {
            warn!("Failed to read from stream: {}", e);
            return Err(e);
        }
    };
}

fn handle_get(stream: &mut TcpStream, req: &httparse::Request) -> io::Result<()> {
    // Find Host header
    let host_hdr = req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("Host"));
    let host = match host_hdr.and_then(|h| std::str::from_utf8(h.value).ok()) {
        Some(s) if !s.is_empty() => s,
        _ => {
            let response = "HTTP/1.1 400 Bad Request\r\n\r\nMissing Host header";
            let _ = stream.write_all(response.as_bytes());
            return Ok(());
        }
    };

    // Build target address (default port 80)
    let target = if host.contains(':') {
        host.to_string()
    } else {
        format!("{}:80", host)
    };

    match TcpStream::connect(&target) {
        Ok(mut remote) => {
            // Ensure blocking mode for simplicity
            let _ = remote.set_nonblocking(false);

            // Prepare path: if absolute URL was provided, strip scheme+host
            let mut path = req.path.unwrap_or("/").to_string();
            if path.starts_with("http://") {
                if let Some(pos) = path[7..].find('/') {
                    path = path[7 + pos..].to_string();
                } else {
                    path = "/".to_string();
                }
            }

            let method = req.method.unwrap_or("GET");
            let version = match req.version {
                Some(1) => "HTTP/1.1",
                Some(0) => "HTTP/1.0",
                _ => "HTTP/1.1",
            };

            // Rebuild request to send to origin
            let mut request_buf = String::new();
            request_buf.push_str(&format!("{} {} {}\r\n", method, path, version));
            for header in req.headers.iter() {
                // Skip proxy-specific headers
                if header.name.eq_ignore_ascii_case("Proxy-Connection") {
                    continue;
                }
                if let Ok(val) = std::str::from_utf8(header.value) {
                    request_buf.push_str(&format!("{}: {}\r\n", header.name, val));
                }
            }
            // Force connection close to simplify streaming lifetime
            request_buf.push_str("Connection: close\r\n\r\n");

            if let Err(e) = remote.write_all(request_buf.as_bytes()) {
                warn!("Failed to write to remote {}: {}", target, e);
                let response = "HTTP/1.1 502 Bad Gateway\r\n\r\nBad Gateway";
                let _ = stream.write_all(response.as_bytes());
                return Err(e);
            }

            // Relay response from remote back to client
            let mut buf = [0u8; 8192];
            loop {
                match remote.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Err(e) = stream.write_all(&buf[..n]) {
                            warn!("Failed to write to client: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read from remote {}: {}", target, e);
                        break;
                    }
                }
            }

            Ok(())
        }
        Err(e) => {
            warn!("Failed to connect to target {}: {}", target, e);
            let response = "HTTP/1.1 502 Bad Gateway\r\n\r\nBad Gateway";
            let _ = stream.write_all(response.as_bytes());
            Err(e)
        }
    }
}

fn handle_connect(stream: &mut TcpStream, req: &httparse::Request) -> io::Result<()> {
    // Determine target (CONNECT uses "host:port" in the path)
    let target = if let Some(path) = req.path {
        if !path.is_empty() {
            path.to_string()
        } else {
            // fallback to Host header
            match req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("Host")).and_then(|h| std::str::from_utf8(h.value).ok()) {
                Some(h) => h.to_string(),
                None => {
                    let response = "HTTP/1.1 400 Bad Request\r\n\r\nMissing target for CONNECT";
                    let _ = stream.write_all(response.as_bytes());
                    return Ok(());
                }
            }
        }
    } else {
        match req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("Host")).and_then(|h| std::str::from_utf8(h.value).ok()) {
            Some(h) => h.to_string(),
            None => {
                let response = "HTTP/1.1 400 Bad Request\r\n\r\nMissing target for CONNECT";
                let _ = stream.write_all(response.as_bytes());
                return Ok(());
            }
        }
    };

    // Connect to the target (expect host:port)
    match TcpStream::connect(&target) {
        Ok(remote) => {
            // Reply to client that connection is established
            let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
            if let Err(e) = stream.write_all(response.as_bytes()) {
                warn!("Failed to write CONNECT response: {}", e);
                return Err(e);
            }

            // Clone streams so we can shuttle data in two threads
            let client_to_remote = match stream.try_clone() {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to clone client stream: {}", e);
                    return Err(e);
                }
            };
            let client_to_remote2 = match stream.try_clone() {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to clone client stream: {}", e);
                    return Err(e);
                }
            };

            let remote_clone1 = match remote.try_clone() {
                Ok(r) => r,
                Err(e) => {
                    warn!("Failed to clone remote stream: {}", e);
                    return Err(e);
                }
            };
            let remote_clone2 = match remote.try_clone() {
                Ok(r) => r,
                Err(e) => {
                    warn!("Failed to clone remote stream: {}", e);
                    return Err(e);
                }
            };

            // Spawn thread to copy client -> remote
            let t1 = std::thread::spawn(move || {
                let mut r = remote_clone1;
                let mut c = client_to_remote;
                let res = io::copy(&mut c, &mut r);
                let _ = r.shutdown(std::net::Shutdown::Write);
                res
            });

            // Spawn thread to copy remote -> client
            let t2 = std::thread::spawn(move || {
                let mut r = remote_clone2;
                let mut c = client_to_remote2;
                let res = io::copy(&mut r, &mut c);
                let _ = c.shutdown(std::net::Shutdown::Write);
                res
            });

            // Wait for both directions to finish
            let _ = t1.join();
            let _ = t2.join();

            Ok(())
        }
        Err(e) => {
            warn!("Failed to connect to CONNECT target {}: {}", target, e);
            let response = "HTTP/1.1 502 Bad Gateway\r\n\r\nBad Gateway";
            let _ = stream.write_all(response.as_bytes());
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;   

    #[test]
    fn test_http() {
        let handle = std::thread::spawn(move || {
            start();
        });

        // Give server a moment to bind
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Run curl as a proxy client and capture verbose output
        let output = std::process::Command::new("curl")
            .arg("-x")
            .arg("127.0.0.1:8080")
            .arg("http://github.com/jamesmoriarty")
            .arg("-v")
            .output()
            .expect("failed to execute curl");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        assert!(combined.contains("Connected to 127.0.0.1"), "did not see expected connection line: {}", combined);
        assert!(combined.contains("HTTP/1.1 301") || combined.contains("HTTP/1.0 301"), "did not see 301 response: {}", combined);
        assert!(combined.contains("Location: https://github.com/jamesmoriarty"), "did not see Location header: {}", combined);
        assert!(combined.to_lowercase().contains("connection: close"), "did not see connection: close: {}", combined);

        handle.join().unwrap();
    }

    #[test]
    fn test_https() {
        let handle = std::thread::spawn(move || {
            start();
        });

        // Give server a moment to bind
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Run curl as a proxy client for HTTPS and capture verbose output
        let output = std::process::Command::new("curl")
            .arg("-x")
            .arg("127.0.0.1:8080")
            .arg("https://github.com/jamesmoriarty")
            .arg("-v")
            .arg("-o")
            .arg("/dev/null")
            .output()
            .expect("failed to execute curl");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        assert!(combined.contains("Connected to 127.0.0.1"), "did not see expected connection line: {}", combined);
        assert!(combined.contains("> CONNECT github.com:443 HTTP/1.1") || combined.contains("CONNECT github.com:443 HTTP/1.1"), "did not see CONNECT request line: {}", combined);
        assert!(combined.contains("HTTP/1.1 200 Connection Established") || combined.contains("HTTP/1.1 200") , "did not see 200 Connection Established: {}", combined);
        assert!(combined.contains("HTTP/2 200") || combined.contains("HTTP/1.1 200"), "did not see inner HTTP 200 response: {}", combined);
        assert!(combined.to_lowercase().contains("ssl connection") || combined.to_lowercase().contains("tls handshake") || combined.to_lowercase().contains("alpn"), "did not see TLS handshake/ALPN info: {}", combined);

        handle.join().unwrap();
    }
}