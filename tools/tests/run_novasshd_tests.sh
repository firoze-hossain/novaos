#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Pure Rust OpenSSH-Compatible SSH-2 Server Daemon (novasshd)
#
# Validates OpenSSH-compatible SSH-2 daemon features:
# 1. Cryptographic Key Exchange & Ciphers:
#    - Curve25519 / X25519 ECDH key exchange (RFC 8731)
#    - Ed25519 digital signatures (RFC 8032) & RSA PKCS#1 v1.5
#    - ChaCha20-Poly1305 authenticated encryption (chacha20-poly1305@openssh.com)
#    - AES-256-GCM authenticated encryption (aes256-gcm@openssh.com)
#    - SHA-256, SHA-512, HMAC-SHA256, and PBKDF2-HMAC-SHA256
# 2. Authentication Subsystem:
#    - ~/.ssh/authorized_keys public key parsing and verification
#    - Password authentication integrated with NovaOS USERS.CFG
# 3. Pseudo-Terminal (PTY) Controller:
#    - RFC 4254 terminal modes and POSIX struct winsize decoding
#    - Interactive shell execution (/bin/novash)
#    - Exec command channels with return codes
# 4. SFTP Protocol v3 & SCP File Transfer:
#    - SFTP packet engine (INIT, REALPATH, OPEN, WRITE, READ, CLOSE)
#    - Secure SCP sink and source transfer handling
# 5. CLI & Daemon Workflow:
#    - novasshd -p, --gen-keys, --selftest, -d, -v, -h
#
# Usage:
#   ./tools/tests/run_novasshd_tests.sh      (or: make novasshd-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/server/novasshd host tests (Pure Rust OpenSSH-2 Server Daemon) ---"
    
    # 1. Run cargo unit test suite
    echo "[1/4] Running Cargo unit test suite..."
    (cd userland/server/novasshd && cargo test --quiet)
    
    # 2. Run automated self-test suite
    echo "[2/4] Running novasshd --selftest verification..."
    (cd userland/server/novasshd && cargo run --quiet -- --selftest)

    # 3. Test key generation and CLI flags
    echo "[3/4] Verifying host key generation and CLI flags..."
    BIN="userland/server/novasshd/target/debug/novasshd"
    
    # Check version output
    VER_OUT=$("$BIN" -v)
    echo "$VER_OUT" | grep -q "novasshd v2.0.0" || { echo "Error: Incorrect version output: $VER_OUT"; exit 1; }

    # Check help output
    HELP_OUT=$("$BIN" -h)
    echo "$HELP_OUT" | grep -q "OpenSSH-Compatible SSH-2 Server Daemon" || { echo "Error: Help text missing"; exit 1; }

    # Check key generation
    KEY_OUT=$("$BIN" --gen-keys)
    echo "$KEY_OUT" | grep -q "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5" || { echo "Error: Invalid host key generation: $KEY_OUT"; exit 1; }

    # 4. Test SSH-2 protocol version banner exchange via simulated socket or test harness
    echo "[4/4] Testing SSH-2 network banner exchange..."
    TEST_PORT=22222
    # Start novasshd in background on unprivileged port
    "$BIN" -p "$TEST_PORT" >/tmp/novasshd_test.log 2>&1 &
    SSHD_PID=$!
    trap 'kill $SSHD_PID 2>/dev/null || true; rm -f /tmp/novasshd_test.log' EXIT

    # Connect and read SSH version string (with retry loop for parallel CI load)
    BANNER=$(python3 -c "
import socket, time
s = socket.socket()
s.settimeout(2.0)
banner = ''
for _ in range(30):
    try:
        s.connect(('127.0.0.1', $TEST_PORT))
        data = s.recv(1024)
        banner = data.decode('utf-8', errors='ignore').strip()
        break
    except Exception as e:
        time.sleep(0.1)
s.close()
if banner:
    print(banner)
else:
    print('ERROR: Connection failed after retries')
")

    kill $SSHD_PID 2>/dev/null || true
    trap - EXIT
    rm -f /tmp/novasshd_test.log

    echo "Received banner: $BANNER"
    echo "$BANNER" | grep -q "SSH-2.0-NovaSSH_2.0.0" || { echo "Error: Unexpected SSH banner: $BANNER"; exit 1; }

    echo "[PASS] All Pure Rust OpenSSH-Compatible SSH-2 Server Daemon (novasshd) tests passed successfully."
else
    echo "novasshd-test: cargo not found - skipping the suite"
fi
