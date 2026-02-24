# Roxy

A fast, lightweight HTTP/HTTPS proxy server written in Rust.

## Features

- **HTTP Proxying**: Supports HTTP requests with automatic Host header resolution
- **HTTPS Tunneling**: Implements the CONNECT method for HTTPS tunneling
- **Configuration**: Bind address configurable via environment variables
- **Logging**: Comprehensive debug logging support via `env_logger`

## Building

```bash
cargo build --release
```

## Running

Start the proxy server on the default address `127.0.0.1:8080`:

```bash
cargo run
```

### Configuration

Configure the bind address using the `ROXY_BIND` environment variable:

```bash
ROXY_BIND=0.0.0.0:3128 cargo run
```

Or after building:

```bash
ROXY_BIND=127.0.0.1:9090 ./target/release/roxy
```

### Logging

Enable debug logging with the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug cargo run
```

## Usage

### As an HTTP Proxy

```bash
# Configure your browser or application to use the proxy at 127.0.0.1:8080
# Then use it with curl:
curl -x 127.0.0.1:8080 http://example.com
```

### HTTPS Tunneling

The proxy automatically handles HTTPS requests using the CONNECT method:

```bash
curl -x 127.0.0.1:8080 https://example.com
```

## Testing

Run the test suite:

```bash
cargo test
```

Tests verify both HTTP and HTTPS proxying functionality.

## Implementation Notes

- Handles HTTP/1.0 and HTTP/1.1 requests
- Supports up to 64 headers per request
- Buffers 512 bytes for initial request parsing and 8KB for response relaying
- Implements HTTP/1.1 pipelining support through persistent connection handling
- Used for bypassing firewalls, content filtering, and anonymity

## Dependencies

- `log` - Logging facade
- `env_logger` - Environment-based logger implementation
- `httparse` - HTTP request parser
- `test-log` - Test logging utilities
