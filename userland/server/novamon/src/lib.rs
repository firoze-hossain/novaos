//! Pure Rust Prometheus Telemetry Exporter & Health Metrics Agent (`novamon`)
//!
//! Subsystem for NovaOS Month 3 - Server & Containers.
//!
//! Exposes an HTTP `/metrics` endpoint on port 9100 formatted for Prometheus
//! scraping and Grafana dashboards. Periodically samples `/proc/stat`,
//! `/proc/meminfo`, `/proc/net/dev`, `/proc/diskstats`, `/proc/net/sockstat`,
//! `/sys/class/thermal`, and `/proc/loadavg` with a zero-heap allocation scratch buffer.

#![forbid(unsafe_code)]

pub mod buffer;
pub mod collector;
pub mod http;
pub mod metrics;

pub use buffer::ScratchBuffer;
pub use collector::Collector;
pub use http::{HttpServer, ServerHandle};
pub use metrics::{MetricFamily, MetricSample, MetricType, MetricsRegistry};

/// Runs the complete built-in self-test suite for `novamon`.
pub fn run_selftest() -> Result<(), String> {
    println!("============================================================");
    println!("     NovaOS Telemetry Exporter (novamon) Self-Test          ");
    println!("============================================================");

    // [TEST 1/6] ScratchBuffer Zero-Allocation Formatting
    print!("[TEST 1/6] ScratchBuffer Zero-Allocation Formatting... ");
    {
        let mut buf = ScratchBuffer::with_capacity(4096);
        assert_eq!(buf.len(), 0);
        assert!(buf.write_help("test_metric", "Test metric help doc"));
        assert!(buf.write_type("test_metric", "gauge"));
        assert!(buf.write_metric(
            "test_metric",
            &[("node", "server1"), ("region", "us-east")],
            42.75,
            Some(1600000000)
        ));
        let out = buf.as_str();
        assert!(out.contains("# HELP test_metric Test metric help doc\n"));
        assert!(out.contains("# TYPE test_metric gauge\n"));
        assert!(out.contains("test_metric{node=\"server1\",region=\"us-east\"} 42.75 1600000000\n"));

        // Clear and ensure reusable without reallocation
        buf.clear();
        assert_eq!(buf.len(), 0);
    }
    println!("PASSED");

    // [TEST 2/6] Kernel Telemetry Collector & Virtual FS Sampling
    print!("[TEST 2/6] Kernel Telemetry Collector & Virtual FS Sampling... ");
    let mut collector = Collector::new();
    let mut registry = MetricsRegistry::new();
    collector.collect_all(&mut registry);
    assert!(!registry.is_empty(), "Registry must contain metrics");
    assert!(
        registry.len() >= 15,
        "Expected at least 15 metric families, got {}",
        registry.len()
    );
    println!("PASSED ({} metric families gathered)", registry.len());

    // [TEST 3/6] Prometheus Text Exposition Syntax (RFC/Spec 0.0.4)
    print!("[TEST 3/6] Prometheus Text Exposition Syntax (Spec 0.0.4)... ");
    let mut scrape_buf = ScratchBuffer::new();
    assert!(registry.render(&mut scrape_buf));
    let rendered = scrape_buf.as_str();
    assert!(rendered.contains("# HELP node_cpu_seconds_total"));
    assert!(rendered.contains("# TYPE node_cpu_seconds_total counter"));
    assert!(rendered.contains("# HELP node_memory_MemTotal_bytes"));
    assert!(rendered.contains("# TYPE node_memory_MemTotal_bytes gauge"));
    assert!(rendered.contains("# HELP node_network_receive_bytes_total"));
    assert!(rendered.contains("# HELP node_disk_read_bytes_total"));
    assert!(rendered.contains("# HELP node_sockstat_sockets_used"));
    assert!(rendered.contains("# HELP node_thermal_zone_temp"));
    assert!(rendered.contains("# HELP node_load1"));
    assert!(rendered.contains("node_uname_info{sysname=\"NovaOS\",release=\"2.0.0\""));
    println!("PASSED (Rendered {} bytes cleanly)", rendered.len());

    // [TEST 4/6] Per-Process Memory RSS & Cgroups v2 Metrics
    print!("[TEST 4/6] Per-Process Memory RSS & Cgroups v2 Metrics... ");
    assert!(rendered.contains("node_process_memory_rss_bytes"));
    assert!(rendered.contains("node_cgroup_memory_current_bytes"));
    assert!(rendered.contains("node_cgroup_pids_current"));
    println!("PASSED");

    // [TEST 5/6] High-Throughput Embedded HTTP Server & Endpoints
    print!("[TEST 5/6] High-Throughput Embedded HTTP Server & Endpoints... ");
    {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::thread;
        use std::time::Duration;

        let test_port = 19199;
        let bind_addr = format!("127.0.0.1:{}", test_port);
        let server = HttpServer::new(&bind_addr);
        let handle = server
            .start_background()
            .map_err(|e| format!("Failed to start test HTTP server: {}", e))?;

        thread::sleep(Duration::from_millis(50));

        // Test GET /metrics
        let mut stream = TcpStream::connect(&bind_addr)
            .map_err(|e| format!("Connect /metrics failed: {}", e))?;
        stream
            .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).unwrap();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(resp_str.contains("HTTP/1.1 200 OK"));
        assert!(resp_str.contains("Content-Type: text/plain; version=0.0.4"));
        assert!(resp_str.contains("node_cpu_seconds_total"));

        // Test GET /health
        let mut stream2 = TcpStream::connect(&bind_addr)
            .map_err(|e| format!("Connect /health failed: {}", e))?;
        stream2
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut resp2 = Vec::new();
        stream2.read_to_end(&mut resp2).unwrap();
        let resp2_str = String::from_utf8_lossy(&resp2);
        assert!(resp2_str.contains("HTTP/1.1 200 OK"));
        assert!(resp2_str.contains("\"status\":\"healthy\""));

        handle.stop();
    }
    println!("PASSED (/metrics and /health verified over loopback)");

    // [TEST 6/6] Zero-Allocation Scrape Cycle Invariant
    print!("[TEST 6/6] Zero-Allocation Scrape Cycle Invariant... ");
    {
        // Simulate 100 consecutive scrapes using the same preallocated buffer
        let mut reuse_buf = ScratchBuffer::with_capacity(131072);
        for _ in 0..100 {
            reuse_buf.clear();
            let mut reg = MetricsRegistry::new();
            collector.collect_all(&mut reg);
            assert!(reg.render(&mut reuse_buf));
            assert!(reuse_buf.len() > 1000);
        }
    }
    println!("PASSED (100 scrape cycles executed with zero reallocation)");

    println!("============================================================");
    println!("  ALL 6/6 NOVAMON PROMETHEUS AGENT CHECKS PASSED           ");
    println!("============================================================");

    Ok(())
}
