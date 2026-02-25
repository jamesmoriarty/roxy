# Roxy

A fast, lightweight HTTP/HTTPS proxy server written in Rust.

## Features

- **HTTP Proxying**: Supports HTTP requests with automatic Host header resolution
- **HTTPS Tunneling**: Implements the CONNECT method for HTTPS tunneling
- **Configuration**: Bind address configurable via environment variables
- **Tracing**: OpenTelemetry tracing with OTLP export (Jaeger, Tempo, etc.)

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

### Tracing

Roxy exports OpenTelemetry traces via OTLP HTTP. Configure the collector endpoint:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 OTEL_SERVICE_NAME=roxy cargo run
```

**Docker Compose** starts roxy alongside Jaeger for local trace visualization:

```bash
docker compose up
```

Then open the Jaeger UI at http://localhost:16686 and send a request through the proxy:

```bash
curl -x localhost:8080 http://example.com
```

Spans are emitted for each connection (`connection`), HTTP GET (`http.get`), and CONNECT tunnel (`http.connect`).

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

- `httparse` - HTTP request parser
- `tracing` / `tracing-subscriber` - Structured logging and tracing
- `tracing-opentelemetry` - Bridge between tracing and OpenTelemetry
- `opentelemetry` / `opentelemetry_sdk` - OpenTelemetry SDK
- `opentelemetry-otlp` - OTLP exporter (HTTP/proto, reqwest blocking)
