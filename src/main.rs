use std::{
    net::{TcpListener, TcpStream},
    io,
    io::{Read, Write},
};
use log::{info, trace, warn};

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
        break; // TODO - remove this to keep server running
    }
}

fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    let mut buffer = [0; 512];
    match stream.read(&mut buffer) {
        Ok(n) => {
            trace!("Request: {}", String::from_utf8_lossy(&buffer[..n]));

            let response = "HTTP/1.1 200 OK\r\n\r\nHello, World!";
            match stream.write_all(response.as_bytes()) {
                Ok(_) => return Ok(()),
                Err(e) => {
                    warn!("Failed to write response: {}", e);
                    return Err(e);
                }
            }
        },
        Err(e) => {
            warn!("Failed to read from stream: {}", e);
            return Err(e);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;   

    #[test]
    fn test_start() {
        let handle = std::thread::spawn(move || {
            start().expect("server failed to start");
        });

        // Give server a moment to bind
        std::thread::sleep(std::time::Duration::from_millis(200));

        handle.join().unwrap();
    }
}