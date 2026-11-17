#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Prometheus Telemetry Exporter & Health Metrics Agent (novamon)
#
# Validates:
# 1. Zero-Heap Allocation ScratchBuffer formatting.
# 2. Kernel Telemetry Collector:
#    - /proc/stat: CPU total and per-core utilization & cumulative seconds
#    - /proc/meminfo: MemTotal, MemFree, MemAvailable, Buffers, Cached, Swap
#    - /proc/diskstats: block device read/write throughput and IOPS
#    - /proc/net/dev: packet and byte rates, errors, drops per interface
#    - /proc/net/sockstat: sockets used, TCP inuse/orphan/tw/alloc, UDP
#    - /sys/class/thermal: core and package thermal sensors
#    - /proc/loadavg & uptime: load averages (1m, 5m, 15m) and system uptime
#    - /proc/[pid]/statm: per-process RSS for monitored daemons
#    - /sys/fs/cgroup: cgroups v2 resource metrics
# 3. Prometheus Exposition Format 0.0.4 text syntax.
# 4. Live Embedded HTTP Server (/metrics, /health, /, 404).
# 5. CLI commands: novamon -p, -b, --oneshot, --dump, --selftest, -v, -h.
#
# Usage:
#   ./tools/tests/run_novamon_tests.sh      (or: make novamon-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/server/novamon host tests (Prometheus Telemetry Exporter) ---"

    # 1. Run novamon Cargo unit test suite
    echo "[1/4] Running novamon Cargo unit test suite..."
    (cd userland/server/novamon && cargo test --quiet)

    # 2. Run novamon automated self-test suite
    echo "[2/4] Running novamon --selftest verification..."
    (cd userland/server/novamon && cargo run --quiet -- --selftest)

    # 3. Test novamon CLI --oneshot and Prometheus syntax output
    echo "[3/4] Testing novamon CLI --oneshot exposition syntax..."
    BIN="userland/server/novamon/target/debug/novamon"
    if [ ! -f "$BIN" ]; then
        (cd userland/server/novamon && cargo build --quiet)
    fi

    # Verify version output
    VER_OUT=$("$BIN" --version)
    echo "$VER_OUT" | grep -q "novamon v2.0.0" || { echo "Error: Incorrect version output: $VER_OUT"; exit 1; }

    # Collect metrics via oneshot
    ONESHOT_OUT=$("$BIN" --oneshot)

    # Verify key Prometheus metrics exist in oneshot dump
    echo "$ONESHOT_OUT" | grep -q "# HELP node_cpu_seconds_total" || { echo "Error: missing node_cpu_seconds_total"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "# TYPE node_cpu_seconds_total counter" || { echo "Error: missing counter type"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "# HELP node_memory_MemTotal_bytes" || { echo "Error: missing node_memory_MemTotal_bytes"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "# TYPE node_memory_MemTotal_bytes gauge" || { echo "Error: missing gauge type"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "node_network_receive_bytes_total" || { echo "Error: missing network receive metrics"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "node_disk_read_bytes_total" || { echo "Error: missing disk read metrics"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "node_sockstat_sockets_used" || { echo "Error: missing socket stats"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "node_thermal_zone_temp" || { echo "Error: missing thermal sensors"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "node_load1" || { echo "Error: missing load1"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "node_uname_info" || { echo "Error: missing uname info"; exit 1; }
    echo "$ONESHOT_OUT" | grep -q "node_process_memory_rss_bytes" || { echo "Error: missing process RSS metrics"; exit 1; }

    # 4. Test live HTTP server scrape on unprivileged port
    echo "[4/4] Testing live HTTP server scrape on port 19100..."
    TEST_PORT=19100
    "$BIN" -p "$TEST_PORT" -b 127.0.0.1 >/tmp/novamon_test.log 2>&1 &
    MON_PID=$!
    trap 'kill $MON_PID 2>/dev/null || true; rm -f /tmp/novamon_test.log' EXIT

    # Wait up to 3 seconds for server to bind
    SERVER_READY=false
    for _ in $(seq 1 30); do
        if python3 -c "import socket; s = socket.socket(); s.connect(('127.0.0.1', $TEST_PORT)); s.close()" 2>/dev/null; then
            SERVER_READY=true
            break
        fi
        sleep 0.1
    done

    if [ "$SERVER_READY" != "true" ]; then
        echo "Error: novamon server failed to listen on port $TEST_PORT"
        cat /tmp/novamon_test.log
        exit 1
    fi

    # Query /metrics via Python HTTP client
    METRICS_RESP=$(python3 -c "
import urllib.request
with urllib.request.urlopen('http://127.0.0.1:$TEST_PORT/metrics', timeout=5) as r:
    headers = dict(r.info())
    body = r.read().decode('utf-8')
    assert r.status == 200, f'Status {r.status}'
    assert 'text/plain; version=0.0.4' in headers.get('Content-Type', ''), f'Bad Content-Type {headers}'
    assert 'node_cpu_seconds_total' in body, 'Missing cpu metric'
    assert 'node_memory_MemTotal_bytes' in body, 'Missing memory metric'
    print(f'HTTP 200 OK: {len(body)} bytes Prometheus exposition received')
")
    echo "  $METRICS_RESP"

    # Query /health via Python HTTP client
    HEALTH_RESP=$(python3 -c "
import urllib.request, json
with urllib.request.urlopen('http://127.0.0.1:$TEST_PORT/health', timeout=5) as r:
    assert r.status == 200
    data = json.loads(r.read().decode('utf-8'))
    assert data.get('status') == 'healthy'
    assert data.get('service') == 'novamon'
    print(f'HTTP 200 OK: health status={data.get(\"status\")}, service={data.get(\"service\")}')
")
    echo "  $HEALTH_RESP"

    # Query / index overview via Python HTTP client
    INDEX_RESP=$(python3 -c "
import urllib.request
with urllib.request.urlopen('http://127.0.0.1:$TEST_PORT/', timeout=5) as r:
    assert r.status == 200
    body = r.read().decode('utf-8')
    assert 'NovaOS Telemetry Exporter' in body
    print(f'HTTP 200 OK: landing dashboard verified')
")
    echo "  $INDEX_RESP"

    # Clean shutdown
    kill $MON_PID 2>/dev/null || true
    wait $MON_PID 2>/dev/null || true
    trap - EXIT
    rm -f /tmp/novamon_test.log

    echo "[PASS] All Prometheus Telemetry Exporter & Health Metrics Agent (novamon) tests passed successfully."
else
    echo "novamon-test: cargo not found - skipping the suite"
fi
