//! Embedded High-Throughput HTTP Server for Prometheus scraping.
//!
//! Exposes:
//! - `GET /metrics`: Prometheus text exposition format (0.0.4) rendered via zero-heap allocation scratch buffer.
//! - `GET /health` & `GET /healthz`: JSON health status and agent metadata.
//! - `GET /`: Landing dashboard with links to telemetry endpoints and Prometheus configuration instructions.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::buffer::ScratchBuffer;
use crate::collector::Collector;
use crate::metrics::MetricsRegistry;

/// Embedded HTTP server for serving Prometheus metrics and health endpoints.
pub struct HttpServer {
    bind_addr: String,
    collector: Arc<Mutex<Collector>>,
    scratch_buffer: Arc<Mutex<ScratchBuffer>>,
    start_time: Instant,
    shutdown: Arc<AtomicBool>,
}

pub struct ServerHandle {
    shutdown: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl ServerHandle {
    pub fn stop(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(h) = self.thread_handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

impl HttpServer {
    pub fn new(bind_addr: impl Into<String>) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            collector: Arc::new(Mutex::new(Collector::new())),
            scratch_buffer: Arc::new(Mutex::new(ScratchBuffer::new())),
            start_time: Instant::now(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_collector(bind_addr: impl Into<String>, collector: Collector) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            collector: Arc::new(Mutex::new(collector)),
            scratch_buffer: Arc::new(Mutex::new(ScratchBuffer::new())),
            start_time: Instant::now(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts the server on a background thread.
    pub fn start_background(&self) -> Result<ServerHandle, String> {
        let listener = TcpListener::bind(&self.bind_addr)
            .map_err(|e| format!("Failed to bind to {}: {}", self.bind_addr, e))?;

        listener
            .set_nonblocking(true)
            .map_err(|e| format!("Failed to set non-blocking: {}", e))?;

        let shutdown = Arc::clone(&self.shutdown);
        shutdown.store(false, Ordering::SeqCst);

        let collector = Arc::clone(&self.collector);
        let scratch_buffer = Arc::clone(&self.scratch_buffer);
        let start_time = self.start_time;
        let shutdown_loop = Arc::clone(&shutdown);

        let thread_handle = thread::spawn(move || {
            while !shutdown_loop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        Self::handle_connection(
                            stream,
                            &collector,
                            &scratch_buffer,
                            start_time,
                        );
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        });

        Ok(ServerHandle {
            shutdown,
            thread_handle: Some(thread_handle),
        })
    }

    /// Serves HTTP requests synchronously until a signal or interrupt occurs.
    pub fn serve(&self) -> Result<(), String> {
        let listener = TcpListener::bind(&self.bind_addr)
            .map_err(|e| format!("Failed to bind to {}: {}", self.bind_addr, e))?;

        println!(
            "🚀 NovaOS Prometheus Exporter (novamon) listening on http://{}",
            self.bind_addr
        );
        println!("   Endpoints: /metrics, /health, /");

        for stream in listener.incoming() {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            match stream {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                    Self::handle_connection(
                        stream,
                        &self.collector,
                        &self.scratch_buffer,
                        self.start_time,
                    );
                }
                Err(e) => {
                    eprintln!("novamon: accept error: {}", e);
                }
            }
        }
        Ok(())
    }

    /// Dispatches a single client TCP connection with zero-heap request inspection.
    fn handle_connection(
        mut stream: TcpStream,
        collector: &Arc<Mutex<Collector>>,
        scratch_buffer: &Arc<Mutex<ScratchBuffer>>,
        start_time: Instant,
    ) {
        let mut req_buf = [0u8; 4096];
        let n = match stream.read(&mut req_buf) {
            Ok(bytes_read) if bytes_read > 0 => bytes_read,
            _ => return,
        };

        let req_slice = &req_buf[..n];

        // Zero-heap allocation request routing inspection
        if req_slice.starts_with(b"GET /metrics ") || req_slice.starts_with(b"GET /metrics\r\n") || req_slice.starts_with(b"GET /metrics\n") {
            Self::serve_metrics(&mut stream, collector, scratch_buffer);
        } else if req_slice.starts_with(b"GET /health ") || req_slice.starts_with(b"GET /healthz ") {
            Self::serve_health(&mut stream, start_time);
        } else if req_slice.starts_with(b"GET / ") || req_slice.starts_with(b"GET /index.html ") {
            Self::serve_index(&mut stream);
        } else {
            Self::serve_not_found(&mut stream);
        }
    }

    /// Serves `/metrics` formatted into the preallocated `ScratchBuffer` without heap churn.
    fn serve_metrics(
        stream: &mut TcpStream,
        collector: &Arc<Mutex<Collector>>,
        scratch_buffer: &Arc<Mutex<ScratchBuffer>>,
    ) {
        let mut buffer = match scratch_buffer.lock() {
            Ok(b) => b,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Reset buffer without deallocating memory
        buffer.clear();

        // Sample metrics
        let mut registry = MetricsRegistry::new();
        {
            let mut col = match collector.lock() {
                Ok(c) => c,
                Err(poisoned) => poisoned.into_inner(),
            };
            col.collect_all(&mut registry);
        }

        registry.render(&mut buffer);

        let content_len = buffer.len();
        let header = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/plain; version=0.0.4; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             Server: novamon/2.0.0 (NovaOS)\r\n\
             \r\n",
            content_len
        );

        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(buffer.as_bytes());
        let _ = stream.flush();
    }

    /// Serves `/health` with JSON status.
    fn serve_health(stream: &mut TcpStream, start_time: Instant) {
        let uptime = start_time.elapsed().as_secs();
        let body = format!(
            "{{\"status\":\"healthy\",\"service\":\"novamon\",\"version\":\"2.0.0\",\"uptime_seconds\":{}}}\n",
            uptime
        );

        let header = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             Server: novamon/2.0.0 (NovaOS)\r\n\
             \r\n",
            body.len()
        );

        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
    }

    /// Serves `/` landing dashboard.
    fn serve_index(stream: &mut TcpStream) {
        let body = r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>NovaOS Prometheus Exporter (novamon)</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: #0f172a; color: #f8fafc; padding: 2rem; margin: 0; }
    h1 { color: #38bdf8; }
    a { color: #60a5fa; text-decoration: none; font-weight: bold; }
    a:hover { text-decoration: underline; }
    .card { background: #1e293b; border-radius: 8px; padding: 1.5rem; margin-top: 1.5rem; max-width: 600px; box-shadow: 0 4px 6px rgba(0,0,0,0.3); }
    code { background: #090d16; padding: 2px 6px; border-radius: 4px; font-family: monospace; color: #a5f3fc; }
    pre { background: #090d16; padding: 1rem; border-radius: 6px; overflow-x: auto; color: #cbd5e1; }
  </style>
</head>
<body>
  <h1>🌌 NovaOS Telemetry Exporter (novamon v2.0.0)</h1>
  <p>Native pure-Rust system observability and health metrics agent.</p>
  <div class="card">
    <h3>Available Endpoints</h3>
    <ul>
      <li><a href="/metrics">/metrics</a> - Prometheus text exposition format (port 9100)</li>
      <li><a href="/health">/health</a> - JSON health status and agent metadata</li>
    </ul>
  </div>
  <div class="card">
    <h3>Prometheus Scrape Configuration</h3>
    <pre>scrape_configs:
  - job_name: 'novaos'
    static_configs:
      - targets: ['localhost:9100']</pre>
  </div>
</body>
</html>
"#;

        let header = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             Server: novamon/2.0.0 (NovaOS)\r\n\
             \r\n",
            body.len()
        );

        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
    }

    /// Serves HTTP 404 Not Found.
    fn serve_not_found(stream: &mut TcpStream) {
        let body = "404 Not Found\n";
        let header = format!(
            "HTTP/1.1 404 Not Found\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body.len()
        );

        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_server_endpoints() {
        let server = HttpServer::new("127.0.0.1:19182");
        let handle = server.start_background().expect("Failed to start test server");

        thread::sleep(Duration::from_millis(50));

        // 1. Test /health
        {
            let mut stream = TcpStream::connect("127.0.0.1:19182").expect("Failed to connect");
            stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut resp = Vec::new();
            stream.read_to_end(&mut resp).unwrap();
            let resp_str = String::from_utf8_lossy(&resp);
            assert!(resp_str.contains("200 OK"));
            assert!(resp_str.contains("\"status\":\"healthy\""));
            assert!(resp_str.contains("\"service\":\"novamon\""));
        }

        // 2. Test /metrics
        {
            let mut stream = TcpStream::connect("127.0.0.1:19182").expect("Failed to connect");
            stream.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut resp = Vec::new();
            stream.read_to_end(&mut resp).unwrap();
            let resp_str = String::from_utf8_lossy(&resp);
            assert!(resp_str.contains("200 OK"));
            assert!(resp_str.contains("text/plain; version=0.0.4"));
            assert!(resp_str.contains("node_cpu_"));
            assert!(resp_str.contains("node_memory_"));
            assert!(resp_str.contains("node_load1"));
        }

        // 3. Test /
        {
            let mut stream = TcpStream::connect("127.0.0.1:19182").expect("Failed to connect");
            stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut resp = Vec::new();
            stream.read_to_end(&mut resp).unwrap();
            let resp_str = String::from_utf8_lossy(&resp);
            assert!(resp_str.contains("200 OK"));
            assert!(resp_str.contains("NovaOS Telemetry Exporter"));
        }

        // 4. Test 404
        {
            let mut stream = TcpStream::connect("127.0.0.1:19182").expect("Failed to connect");
            stream.write_all(b"GET /unknown HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut resp = Vec::new();
            stream.read_to_end(&mut resp).unwrap();
            let resp_str = String::from_utf8_lossy(&resp);
            assert!(resp_str.contains("404 Not Found"));
        }

        handle.stop();
    }
}
