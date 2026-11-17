//! NovaOS Prometheus Telemetry Exporter & Health Metrics Agent (`novamon`)
//!
//! Entry point and CLI dispatcher for `novamon`.

#![forbid(unsafe_code)]

use std::env;
use std::process;

use novamon::{Collector, HttpServer, MetricsRegistry, ScratchBuffer};

fn print_help() {
    println!("NovaOS Prometheus Telemetry Exporter & Health Metrics Agent (novamon v2.0.0)");
    println!("Usage: novamon [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -p, --port <PORT>     Port to bind HTTP server to (default: 9100)");
    println!("  -b, --bind <ADDR>     IP address to bind HTTP server to (default: 0.0.0.0)");
    println!("  -1, --oneshot, --dump Collect metrics once, print Prometheus format to stdout, and exit");
    println!("      --selftest        Run comprehensive automated self-test verification suite");
    println!("  -v, --version         Print version information and exit");
    println!("  -h, --help            Print this help message and exit");
    println!();
    println!("Endpoints:");
    println!("  http://<host>:9100/metrics  Prometheus text exposition format (version 0.0.4)");
    println!("  http://<host>:9100/health   JSON health status and agent metadata");
    println!("  http://<host>:9100/         Overview and Grafana scrape configuration");
}

fn print_version() {
    println!("novamon v2.0.0 (NovaOS Prometheus Telemetry Exporter & Health Metrics Agent)");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut port = 9100u16;
    let mut bind_addr = "0.0.0.0".to_string();
    let mut oneshot = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "-v" | "--version" => {
                print_version();
                return;
            }
            "--selftest" => {
                if let Err(e) = novamon::run_selftest() {
                    eprintln!("novamon: self-test failed: {}", e);
                    process::exit(1);
                }
                return;
            }
            "-1" | "--oneshot" | "--dump" => {
                oneshot = true;
            }
            "-p" | "--port" => {
                if i + 1 < args.len() {
                    i += 1;
                    match args[i].parse::<u16>() {
                        Ok(p) => port = p,
                        Err(_) => {
                            eprintln!("novamon: invalid port '{}'", args[i]);
                            process::exit(1);
                        }
                    }
                } else {
                    eprintln!("novamon: option '{}' requires an argument", args[i]);
                    process::exit(1);
                }
            }
            "-b" | "--bind" => {
                if i + 1 < args.len() {
                    i += 1;
                    bind_addr = args[i].clone();
                } else {
                    eprintln!("novamon: option '{}' requires an argument", args[i]);
                    process::exit(1);
                }
            }
            other => {
                eprintln!("novamon: unrecognized option '{}'", other);
                eprintln!("Try 'novamon --help' for more information.");
                process::exit(1);
            }
        }
        i += 1;
    }

    if oneshot {
        // Collect metrics and dump to stdout
        let mut collector = Collector::new();
        let mut registry = MetricsRegistry::new();
        let mut buffer = ScratchBuffer::new();

        collector.collect_all(&mut registry);
        registry.render(&mut buffer);

        print!("{}", buffer.as_str());
        return;
    }

    let full_bind = format!("{}:{}", bind_addr, port);
    let server = HttpServer::new(&full_bind);

    if let Err(e) = server.serve() {
        eprintln!("novamon: server error: {}", e);
        process::exit(1);
    }
}
