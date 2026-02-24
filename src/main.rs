use std::{
    net::{TcpListener, TcpStream},
    io,
    io::{Read, Write},
};
use log::{info, trace, warn, debug};

fn main() {
    env_logger::init();
    start();
}

fn start() {
    match TcpListener::bind("127.0.0.1:8080") {
        Ok(l) => {
            l.set_nonblocking(false).unwrap();
            run(l)
        },
        Err(e) => {
            warn!("Failed to bind to address: {}", e);
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

fn handle_connect(stream: &mut TcpStream, _req: &httparse::Request) -> io::Result<()> {
    let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
    match stream.write_all(response.as_bytes()) {
        Ok(_) => Ok(()),
        Err(e) => {
            warn!("Failed to write response: {}", e);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;   

    #[test]
    fn test_start() {
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
}