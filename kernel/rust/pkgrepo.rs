//! kernel/rust/pkgrepo.rs - Month 4 (Server & security core):
//! Package-repository network protocol support, server side.
//!
//! Provides the server-side counterpart to the existing `pkg install-http`
//! client (Phase 64), enabling a NovaOS machine to serve signed packages
//! (`.PKG` files) to other machines over HTTP/1.1 built directly on top of
//! `kernel/rust/tcp.rs`'s RFC 793 TCP stack and `kernel/rust/pkgsign.rs`'s
//! HMAC-SHA256 signature verification.
//!
//! # Protocol Contract
//!
//! The server implements a lightweight, zero-allocation HTTP/1.1 engine:
//!
//! - `GET /packages/<NAME>.PKG` (or `/<NAME>.PKG` / `/packages/<NAME>`):
//!   Looks up the package file in the root filesystem, verifies that it is
//!   a valid NovaOS package (`magic == "NVPK"`), and confirms its cryptographic
//!   signature matches `rust_pkg_verify_signature()`. If valid, responds:
//!   `HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: <size>\r\nConnection: close\r\n\r\n`
//!   followed by the complete package bytes.
//!   If the package is not found: `404 Not Found`.
//!   If the signature verification fails (corrupted or tampered): `403 Forbidden`
//!   (`403 Signature Invalid`), refusing to serve unverified code.
//!
//! - `HEAD /packages/<NAME>.PKG`:
//!   Sends identical headers with Content-Length without the package body.
//!
//! - `GET /packages` / `GET /packages/index` / `GET /index`:
//!   Generates a repository manifest index dynamically, listing all available
//!   signed packages with their versions, sizes, and descriptions.
//!
//! - `GET /status` / `GET /health`:
//!   Returns HTTP 200 with repository telemetry (status, uptime, requests,
//!   packages served, bytes transferred).
//!
//! - Non-GET/HEAD methods:
//!   `405 Method Not Allowed\r\nAllow: GET, HEAD\r\n...`
//!
//! - Malformed requests:
//!   `400 Bad Request\r\n...`
//!
//! # Safety & Concurrency
//!
//! All operations are zero-heap allocation, bounded in size (static 64KB
//! package buffer), and protected by `SpinLock` for SMP safety. A non-blocking
//! poll function (`rust_pkgrepo_poll()`) integrates cleanly into `net_poll()`.

#[cfg(not(test))]
use crate::spinlock::SpinLock;

#[cfg(test)]
pub struct SpinLock<T>(std::sync::Mutex<T>);

#[cfg(test)]
impl<T> SpinLock<T> {
    pub const fn new(val: T) -> Self {
        Self(std::sync::Mutex::new(val))
    }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}

#[cfg(test)]
#[path = "sha256.rs"]
mod sha256;
#[cfg(test)]
#[path = "hmac_sha256.rs"]
mod hmac_sha256;
#[cfg(test)]
#[path = "pkgsign_core.rs"]
mod pkgsign_core;

pub const DEFAULT_PORT: u16 = 8080;
pub const MAX_REQUEST_SIZE: usize = 1024;
pub const MAX_HEADER_SIZE: usize = 512;
pub const MAX_PACKAGE_SIZE: usize = 65536; /* 64KB */
pub const PKG_HEADER_SIZE: usize = 128;
pub const PKG_SIGNATURE_OFFSET: usize = 96;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct PkgRepoStatus {
    pub is_running: u32,
    pub port: u16,
    pub _reserved: u16,
    pub requests_total: u32,
    pub packages_served: u32,
    pub bytes_served: u32,
}

pub struct PkgRepoServer {
    pub active: bool,
    pub listen_sock: i32,
    pub port: u16,
    pub requests_total: u32,
    pub packages_served: u32,
    pub bytes_served: u32,
}

impl PkgRepoServer {
    pub const fn new() -> Self {
        Self {
            active: false,
            listen_sock: -1,
            port: DEFAULT_PORT,
            requests_total: 0,
            packages_served: 0,
            bytes_served: 0,
        }
    }
}

static SERVER: SpinLock<PkgRepoServer> = SpinLock::new(PkgRepoServer::new());

#[cfg(not(test))]
extern "C" {
    fn vfs_read_file(filename: *const u8, buf: *mut u8, buf_size: u32) -> i32;
    fn kernel_log(fmt: *const u8, ...);
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum HttpMethod {
    Get,
    Head,
    Other,
}

#[derive(Copy, Clone, Debug)]
pub struct HttpRequest<'a> {
    pub method: HttpMethod,
    pub path: &'a [u8],
}

/// Parses an HTTP/1.1 request line: `<METHOD> <PATH> HTTP/1.x`
pub fn parse_http_request(req: &[u8]) -> Result<HttpRequest<'_>, i32> {
    if req.is_empty() {
        return Err(-1);
    }

    let mut line_end = req.len();
    for i in 0..req.len() {
        if req[i] == b'\r' || req[i] == b'\n' {
            line_end = i;
            break;
        }
    }
    if line_end == 0 {
        return Err(-1);
    }
    let line = &req[..line_end];

    let mut space1 = None;
    let mut space2 = None;
    for (i, &b) in line.iter().enumerate() {
        if b == b' ' {
            if space1.is_none() {
                space1 = Some(i);
            } else if space2.is_none() {
                space2 = Some(i);
                break;
            }
        }
    }

    let s1 = match space1 {
        Some(pos) => pos,
        None => return Err(-1),
    };
    let s2 = match space2 {
        Some(pos) => pos,
        None => return Err(-1),
    };

    let method_slice = &line[..s1];
    let path_slice = &line[s1 + 1..s2];

    let method = if method_slice == b"GET" {
        HttpMethod::Get
    } else if method_slice == b"HEAD" {
        HttpMethod::Head
    } else {
        HttpMethod::Other
    };

    Ok(HttpRequest {
        method,
        path: path_slice,
    })
}

/// Normalizes a requested URL path into an 8.3 uppercase package filename
/// (e.g., `/packages/EDITOR.PKG` -> `EDITOR.PKG`, `/editor` -> `EDITOR.PKG`).
/// Returns true if a valid package filename was extracted.
pub fn extract_package_filename(path: &[u8], out: &mut [u8; 13]) -> bool {
    let mut p = path;
    if p.starts_with(b"/packages/") {
        p = &p[10..];
    } else if p.starts_with(b"/packages") {
        p = &p[9..];
    }
    if p.starts_with(b"/") {
        p = &p[1..];
    }

    if p.is_empty() || p.len() > 12 {
        return false;
    }

    let mut base_len = 0;
    let mut has_ext = false;
    let mut dot_pos = 0;

    for (i, &b) in p.iter().enumerate() {
        if b == b'?' || b == b'#' {
            break;
        }
        if b == b'.' {
            has_ext = true;
            dot_pos = i;
            base_len = i;
            break;
        }
        base_len = i + 1;
    }

    if base_len == 0 || base_len > 8 {
        return false;
    }

    for (i, b) in p[..base_len].iter().enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    let mut pos = base_len;

    if has_ext {
        let ext = &p[dot_pos..];
        if ext.len() == 4
            && (ext[1] == b'p' || ext[1] == b'P')
            && (ext[2] == b'k' || ext[2] == b'K')
            && (ext[3] == b'g' || ext[3] == b'G')
        {
            out[pos] = b'.';
            out[pos + 1] = b'P';
            out[pos + 2] = b'K';
            out[pos + 3] = b'G';
            pos += 4;
        } else {
            return false;
        }
    } else {
        out[pos] = b'.';
        out[pos + 1] = b'P';
        out[pos + 2] = b'K';
        out[pos + 3] = b'G';
        pos += 4;
    }

    out[pos] = 0;
    true
}

/// Helper to format a unsigned integer into an ASCII buffer.
pub fn format_uint(mut val: usize, buf: &mut [u8]) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut digits = [0u8; 16];
    let mut n = 0;
    while val > 0 && n < digits.len() {
        digits[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    let len = n.min(buf.len());
    for i in 0..len {
        buf[i] = digits[n - 1 - i];
    }
    len
}

/// Builds standard HTTP 200 response headers with Content-Type and Content-Length.
pub fn build_http_200_headers(
    out_buf: &mut [u8],
    content_type: &[u8],
    content_len: usize,
) -> usize {
    let mut pos = 0;
    let mut append = |bytes: &[u8]| {
        let copy = bytes.len().min(out_buf.len().saturating_sub(pos));
        out_buf[pos..pos + copy].copy_from_slice(&bytes[..copy]);
        pos += copy;
    };

    append(b"HTTP/1.1 200 OK\r\nContent-Type: ");
    append(content_type);
    append(b"\r\nContent-Length: ");
    let mut num_buf = [0u8; 16];
    let num_len = format_uint(content_len, &mut num_buf);
    append(&num_buf[..num_len]);
    append(b"\r\nConnection: close\r\n\r\n");

    pos
}

/// Formats a 404 Not Found response.
pub fn build_http_404(out_buf: &mut [u8]) -> usize {
    const BODY: &[u8] = b"404 Not Found\n";
    let mut pos = 0;
    let mut append = |bytes: &[u8]| {
        let copy = bytes.len().min(out_buf.len().saturating_sub(pos));
        out_buf[pos..pos + copy].copy_from_slice(&bytes[..copy]);
        pos += copy;
    };
    append(b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nConnection: close\r\n\r\n");
    append(BODY);
    pos
}

/// Formats a 400 Bad Request response.
pub fn build_http_400(out_buf: &mut [u8]) -> usize {
    const BODY: &[u8] = b"400 Bad Request\n";
    let mut pos = 0;
    let mut append = |bytes: &[u8]| {
        let copy = bytes.len().min(out_buf.len().saturating_sub(pos));
        out_buf[pos..pos + copy].copy_from_slice(&bytes[..copy]);
        pos += copy;
    };
    append(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 16\r\nConnection: close\r\n\r\n");
    append(BODY);
    pos
}

/// Formats a 403 Forbidden response (used when package signature fails verification).
pub fn build_http_403_signature_invalid(out_buf: &mut [u8]) -> usize {
    const BODY: &[u8] = b"403 Signature Invalid\n";
    let mut pos = 0;
    let mut append = |bytes: &[u8]| {
        let copy = bytes.len().min(out_buf.len().saturating_sub(pos));
        out_buf[pos..pos + copy].copy_from_slice(&bytes[..copy]);
        pos += copy;
    };
    append(b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 22\r\nConnection: close\r\n\r\n");
    append(BODY);
    pos
}

/// Formats a 405 Method Not Allowed response.
pub fn build_http_405(out_buf: &mut [u8]) -> usize {
    const BODY: &[u8] = b"405 Method Not Allowed\n";
    let mut pos = 0;
    let mut append = |bytes: &[u8]| {
        let copy = bytes.len().min(out_buf.len().saturating_sub(pos));
        out_buf[pos..pos + copy].copy_from_slice(&bytes[..copy]);
        pos += copy;
    };
    append(b"HTTP/1.1 405 Method Not Allowed\r\nAllow: GET, HEAD\r\nContent-Type: text/plain\r\nContent-Length: 23\r\nConnection: close\r\n\r\n");
    append(BODY);
    pos
}

/// Validates package header integrity and cryptographic HMAC-SHA256 signature.
pub fn verify_pkg_bytes(pkg_bytes: &[u8]) -> bool {
    if pkg_bytes.len() < PKG_HEADER_SIZE {
        return false;
    }
    if &pkg_bytes[0..4] != b"NVPK" {
        return false;
    }

    let payload_size = u32::from_le_bytes([
        pkg_bytes[92],
        pkg_bytes[93],
        pkg_bytes[94],
        pkg_bytes[95],
    ]) as usize;

    if pkg_bytes.len() < PKG_HEADER_SIZE + payload_size {
        return false;
    }

    #[cfg(not(test))]
    unsafe {
        crate::pkgsign::rust_pkg_verify_signature(
            pkg_bytes.as_ptr(),
            PKG_HEADER_SIZE as u32,
            PKG_SIGNATURE_OFFSET as u32,
            pkg_bytes[PKG_HEADER_SIZE..].as_ptr(),
            payload_size as u32,
        )
    }

    #[cfg(test)]
    {
        // In host test mode, test verifying with pkgsign_core logic
        let mut header_zeroed = [0u8; PKG_HEADER_SIZE];
        header_zeroed.copy_from_slice(&pkg_bytes[..PKG_HEADER_SIZE]);
        for b in &mut header_zeroed[PKG_SIGNATURE_OFFSET..PKG_SIGNATURE_OFFSET + 32] {
            *b = 0;
        }
        let expected = crate::pkgsign_core::compute_signature(
            &header_zeroed,
            &pkg_bytes[PKG_HEADER_SIZE..PKG_HEADER_SIZE + payload_size],
        );
        let actual = &pkg_bytes[PKG_SIGNATURE_OFFSET..PKG_SIGNATURE_OFFSET + 32];
        expected == actual
    }
}

/// Generates a repository index text manifest listing known signed packages.
pub fn generate_manifest_index(out_buf: &mut [u8]) -> usize {
    let mut pos = 0;
    let mut append = |bytes: &[u8]| {
        let copy = bytes.len().min(out_buf.len().saturating_sub(pos));
        out_buf[pos..pos + copy].copy_from_slice(&bytes[..copy]);
        pos += copy;
    };

    append(b"# NovaOS Package Repository Index\n");
    append(b"# Format: RFC 822 Package Stanzas\n\n");

    const KNOWN_PACKAGES: [&[u8]; 2] = [b"EDITOR.PKG\0", b"GAME.PKG\0"];
    for &pkg_name_cstr in &KNOWN_PACKAGES {
        let mut file_buf = [0u8; 1024];
        let bytes_read = {
            #[cfg(not(test))]
            unsafe {
                vfs_read_file(pkg_name_cstr.as_ptr(), file_buf.as_mut_ptr(), file_buf.len() as u32)
            }
            #[cfg(test)]
            {
                mock_vfs_read(pkg_name_cstr, &mut file_buf)
            }
        };

        if bytes_read >= PKG_HEADER_SIZE as i32 {
            let n = bytes_read as usize;
            if verify_pkg_bytes(&file_buf[..n]) {
                let name = extract_cstr(&file_buf[4..20]);
                let ver = extract_cstr(&file_buf[20..28]);
                let desc = extract_cstr(&file_buf[28..92]);
                let payload_size = u32::from_le_bytes([
                    file_buf[92],
                    file_buf[93],
                    file_buf[94],
                    file_buf[95],
                ]);

                append(b"Package: ");
                append(name);
                append(b"\nVersion: ");
                append(ver);
                append(b"\nSize: ");
                let mut sz_buf = [0u8; 16];
                let sz_len = format_uint(payload_size as usize, &mut sz_buf);
                append(&sz_buf[..sz_len]);
                append(b"\nDescription: ");
                append(desc);
                append(b"\nFilename: ");
                let fname_len = pkg_name_cstr.iter().position(|&b| b == 0).unwrap_or(pkg_name_cstr.len());
                append(&pkg_name_cstr[..fname_len]);
                append(b"\n\n");
            }
        }
    }

    pos
}

fn extract_cstr(slice: &[u8]) -> &[u8] {
    let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    &slice[..len]
}

/// Generates repository telemetry status text.
pub fn generate_status_text(out_buf: &mut [u8], status: &PkgRepoStatus) -> usize {
    let mut pos = 0;
    let mut append = |bytes: &[u8]| {
        let copy = bytes.len().min(out_buf.len().saturating_sub(pos));
        out_buf[pos..pos + copy].copy_from_slice(&bytes[..copy]);
        pos += copy;
    };

    append(b"NovaOS Package Repository Server v1.0 (Rust)\n");
    append(b"Status: ");
    if status.is_running != 0 {
        append(b"running\nPort: ");
        let mut p_buf = [0u8; 8];
        let p_len = format_uint(status.port as usize, &mut p_buf);
        append(&p_buf[..p_len]);
    } else {
        append(b"stopped");
    }
    append(b"\nRequests Total: ");
    let mut q_buf = [0u8; 16];
    let q_len = format_uint(status.requests_total as usize, &mut q_buf);
    append(&q_buf[..q_len]);
    append(b"\nPackages Served: ");
    let mut pk_buf = [0u8; 16];
    let pk_len = format_uint(status.packages_served as usize, &mut pk_buf);
    append(&pk_buf[..pk_len]);
    append(b"\nBytes Served: ");
    let mut b_buf = [0u8; 16];
    let b_len = format_uint(status.bytes_served as usize, &mut b_buf);
    append(&b_buf[..b_len]);
    append(b"\n");

    pos
}

/// Handles an incoming client TCP socket: reads request, dispatches HTTP, sends response.
pub fn handle_client(sock: i32) {
    #[cfg(not(test))]
    use crate::tcp::{rust_tcp_close, rust_tcp_recv, rust_tcp_send};

    let mut req_buf = [0u8; MAX_REQUEST_SIZE];
    let n = {
        #[cfg(not(test))]
        unsafe {
            rust_tcp_recv(sock, req_buf.as_mut_ptr(), req_buf.len() as u32)
        }
        #[cfg(test)]
        {
            mock_tcp_recv(sock, &mut req_buf)
        }
    };

    if n <= 0 {
        #[cfg(not(test))]
        unsafe {
            rust_tcp_close(sock);
        }
        return;
    }

    let req_len = n as usize;
    let parsed = match parse_http_request(&req_buf[..req_len]) {
        Ok(r) => r,
        Err(_) => {
            let mut resp = [0u8; 256];
            let _len = build_http_400(&mut resp);
            #[cfg(not(test))]
            unsafe {
                rust_tcp_send(sock, resp.as_ptr(), _len as u32);
                rust_tcp_close(sock);
            }
            return;
        }
    };

    {
        let mut s = SERVER.lock();
        s.requests_total = s.requests_total.wrapping_add(1);
    }

    if parsed.method == HttpMethod::Other {
        let mut resp = [0u8; 256];
        let _len = build_http_405(&mut resp);
        #[cfg(not(test))]
        unsafe {
            rust_tcp_send(sock, resp.as_ptr(), _len as u32);
            rust_tcp_close(sock);
        }
        return;
    }

    let _is_head = parsed.method == HttpMethod::Head;
    let path = parsed.path;

    if path == b"/status" || path == b"/health" {
        let status = {
            let s = SERVER.lock();
            PkgRepoStatus {
                is_running: if s.active { 1 } else { 0 },
                port: s.port,
                _reserved: 0,
                requests_total: s.requests_total,
                packages_served: s.packages_served,
                bytes_served: s.bytes_served,
            }
        };
        let mut body = [0u8; 512];
        let body_len = generate_status_text(&mut body, &status);
        let mut resp = [0u8; MAX_HEADER_SIZE];
        let _hdr_len = build_http_200_headers(&mut resp, b"text/plain", body_len);

        #[cfg(not(test))]
        unsafe {
            rust_tcp_send(sock, resp.as_ptr(), _hdr_len as u32);
            if !_is_head {
                rust_tcp_send(sock, body.as_ptr(), body_len as u32);
            }
            rust_tcp_close(sock);
        }
        return;
    }

    if path == b"/packages" || path == b"/packages/" || path == b"/packages/index" || path == b"/index" {
        let mut body = [0u8; 2048];
        let body_len = generate_manifest_index(&mut body);
        let mut resp = [0u8; MAX_HEADER_SIZE];
        let _hdr_len = build_http_200_headers(&mut resp, b"text/plain", body_len);

        #[cfg(not(test))]
        unsafe {
            rust_tcp_send(sock, resp.as_ptr(), _hdr_len as u32);
            if !_is_head {
                rust_tcp_send(sock, body.as_ptr(), body_len as u32);
            }
            rust_tcp_close(sock);
        }
        return;
    }

    // Try package file extraction
    let mut filename = [0u8; 13];
    if extract_package_filename(path, &mut filename) {
        static mut FILE_BUF: [u8; MAX_PACKAGE_SIZE] = [0u8; MAX_PACKAGE_SIZE];
        let bytes_read = {
            #[cfg(not(test))]
            unsafe {
                vfs_read_file(filename.as_ptr(), FILE_BUF.as_mut_ptr(), FILE_BUF.len() as u32)
            }
            #[cfg(test)]
            {
                #[allow(static_mut_refs)]
                unsafe { mock_vfs_read(&filename, &mut FILE_BUF) }
            }
        };

        if bytes_read < PKG_HEADER_SIZE as i32 {
            let mut resp = [0u8; 256];
            let _len = build_http_404(&mut resp);
            #[cfg(not(test))]
            unsafe {
                rust_tcp_send(sock, resp.as_ptr(), _len as u32);
                rust_tcp_close(sock);
            }
            return;
        }

        let pkg_len = bytes_read as usize;
        let valid_sig = unsafe { verify_pkg_bytes(&FILE_BUF[..pkg_len]) };

        if !valid_sig {
            let mut resp = [0u8; 256];
            let _len = build_http_403_signature_invalid(&mut resp);
            #[cfg(not(test))]
            unsafe {
                rust_tcp_send(sock, resp.as_ptr(), _len as u32);
                rust_tcp_close(sock);
            }
            return;
        }

        // Valid signed package! Serve it.
        let mut resp = [0u8; MAX_HEADER_SIZE];
        let _hdr_len = build_http_200_headers(&mut resp, b"application/octet-stream", pkg_len);

        #[cfg(not(test))]
        unsafe {
            rust_tcp_send(sock, resp.as_ptr(), _hdr_len as u32);
            if !_is_head {
                rust_tcp_send(sock, FILE_BUF.as_ptr(), pkg_len as u32);
            }
            rust_tcp_close(sock);
        }

        {
            let mut s = SERVER.lock();
            s.packages_served = s.packages_served.wrapping_add(1);
            s.bytes_served = s.bytes_served.wrapping_add(pkg_len as u32);
        }
        return;
    }

    // Unrecognized path
    let mut resp = [0u8; 256];
    let _len = build_http_404(&mut resp);
    #[cfg(not(test))]
    unsafe {
        rust_tcp_send(sock, resp.as_ptr(), _len as u32);
        rust_tcp_close(sock);
    }
}

/// Starts listening on `port`. Returns 0 on success, or negative error code.
#[no_mangle]
pub extern "C" fn rust_pkgrepo_start(port: u16) -> i32 {
    let mut s = SERVER.lock();
    if s.active {
        return -16; /* -EBUSY */
    }

    #[cfg(not(test))]
    {
        use crate::tcp::{rust_tcp_bind, rust_tcp_close, rust_tcp_listen, rust_tcp_socket};
        let sock = rust_tcp_socket();
        if sock < 0 {
            return -1;
        }
        if rust_tcp_bind(sock, port) != 0 {
            rust_tcp_close(sock);
            return -98; /* -EADDRINUSE */
        }
        if rust_tcp_listen(sock, 4) != 0 {
            rust_tcp_close(sock);
            return -1;
        }
        s.active = true;
        s.listen_sock = sock;
        s.port = port;
    }

    #[cfg(test)]
    {
        s.active = true;
        s.listen_sock = 100;
        s.port = port;
    }

    0
}

/// Stops the repository server. Returns 0 on success, or -1 if not running.
#[no_mangle]
pub extern "C" fn rust_pkgrepo_stop() -> i32 {
    let mut s = SERVER.lock();
    if !s.active {
        return -1;
    }

    #[cfg(not(test))]
    {
        use crate::tcp::rust_tcp_close;
        if s.listen_sock >= 0 {
            rust_tcp_close(s.listen_sock);
        }
    }

    s.active = false;
    s.listen_sock = -1;
    0
}

/// Fills `out_status` with current telemetry.
#[no_mangle]
pub extern "C" fn rust_pkgrepo_status(out_status: *mut PkgRepoStatus) -> i32 {
    if out_status.is_null() {
        return -14; /* -EFAULT */
    }
    let s = SERVER.lock();
    let st = PkgRepoStatus {
        is_running: if s.active { 1 } else { 0 },
        port: s.port,
        _reserved: 0,
        requests_total: s.requests_total,
        packages_served: s.packages_served,
        bytes_served: s.bytes_served,
    };
    unsafe {
        core::ptr::write(out_status, st);
    }
    0
}

/// Non-blocking poll hook, called from `net_poll()` on every tick.
#[no_mangle]
pub extern "C" fn rust_pkgrepo_poll() {
    let (active, sock) = {
        let s = SERVER.lock();
        (s.active, s.listen_sock)
    };

    if !active || sock < 0 {
        return;
    }

    #[cfg(not(test))]
    {
        use crate::tcp::rust_tcp_accept_nonblock;
        let client = rust_tcp_accept_nonblock(sock);
        if client >= 0 {
            handle_client(client);
        }
    }
}

/// Syscall gateway dispatcher for `SYS_PKGREPO_CTL` (78).
#[no_mangle]
pub extern "C" fn rust_pkgrepo_ctl(cmd: u32, arg: *mut u8) -> i32 {
    match cmd {
        1 => {
            /* START: arg is integer port */
            let port = if arg.is_null() {
                DEFAULT_PORT
            } else {
                arg as usize as u16
            };
            rust_pkgrepo_start(port)
        }
        2 => {
            /* STOP */
            rust_pkgrepo_stop()
        }
        3 => {
            /* STATUS: arg is *mut PkgRepoStatus */
            rust_pkgrepo_status(arg as *mut PkgRepoStatus)
        }
        _ => -22, /* -EINVAL */
    }
}

/// Kernel-side boot self-test: exercises HTTP parsing, path extraction,
/// response headers, signature checking, manifest generation, and lifecycle.
#[no_mangle]
pub extern "C" fn rust_pkgrepo_selftest() -> i32 {
    let mut failures = 0;

    // Test 1: HTTP Request Parsing
    let req1 = b"GET /packages/EDITOR.PKG HTTP/1.1\r\nHost: 10.0.2.15\r\n\r\n";
    match parse_http_request(req1) {
        Ok(r) => {
            if r.method != HttpMethod::Get || r.path != b"/packages/EDITOR.PKG" {
                failures |= 1;
            }
        }
        Err(_) => failures |= 1,
    }

    let req2 = b"HEAD /status HTTP/1.1\r\n\r\n";
    match parse_http_request(req2) {
        Ok(r) => {
            if r.method != HttpMethod::Head || r.path != b"/status" {
                failures |= 2;
            }
        }
        Err(_) => failures |= 2,
    }

    let req3 = b"POST /packages/EDITOR.PKG HTTP/1.1\r\n\r\n";
    match parse_http_request(req3) {
        Ok(r) => {
            if r.method != HttpMethod::Other {
                failures |= 4;
            }
        }
        Err(_) => failures |= 4,
    }

    // Test 2: Filename extraction and normalization
    let mut out_fn = [0u8; 13];
    if !extract_package_filename(b"/packages/editor.pkg", &mut out_fn) || &out_fn[..10] != b"EDITOR.PKG" {
        failures |= 8;
    }
    let mut out_fn2 = [0u8; 13];
    if !extract_package_filename(b"/game", &mut out_fn2) || &out_fn2[..8] != b"GAME.PKG" {
        failures |= 16;
    }

    // Test 3: Response headers construction
    let mut hdr_buf = [0u8; 256];
    let hlen = build_http_200_headers(&mut hdr_buf, b"application/octet-stream", 190);
    if hlen == 0 || !hdr_buf[..hlen].starts_with(b"HTTP/1.1 200 OK\r\n") {
        failures |= 32;
    }

    // Test 4: Status code responses (404, 400, 403, 405)
    let mut r404 = [0u8; 128];
    let l404 = build_http_404(&mut r404);
    if !r404[..l404].starts_with(b"HTTP/1.1 404 Not Found") {
        failures |= 64;
    }

    let mut r403 = [0u8; 128];
    let l403 = build_http_403_signature_invalid(&mut r403);
    if !r403[..l403].starts_with(b"HTTP/1.1 403 Forbidden") {
        failures |= 128;
    }

    // Test 5: Synthetic Package Signature Check
    let mut synth_pkg = [0u8; 128 + 64];
    synth_pkg[0..4].copy_from_slice(b"NVPK");
    synth_pkg[4..10].copy_from_slice(b"TESTPK");
    synth_pkg[92..96].copy_from_slice(&64u32.to_le_bytes());
    // Fill payload
    for i in 0..64 {
        synth_pkg[128 + i] = (i + 1) as u8;
    }
    // Zero signature for signing
    for b in &mut synth_pkg[96..128] {
        *b = 0;
    }
    let sig = crate::pkgsign_core::compute_signature(&synth_pkg[..128], &synth_pkg[128..128 + 64]);
    synth_pkg[96..128].copy_from_slice(&sig);

    // Valid signature must pass
    if !verify_pkg_bytes(&synth_pkg) {
        failures |= 256;
    }

    // Tampered payload must fail
    synth_pkg[128] ^= 0xFF;
    if verify_pkg_bytes(&synth_pkg) {
        failures |= 512;
    }
    synth_pkg[128] ^= 0xFF; // restore

    // Tampered signature must fail
    synth_pkg[96] ^= 0x01;
    if verify_pkg_bytes(&synth_pkg) {
        failures |= 1024;
    }

    // Test 6: Server lifecycle via ctl
    let mut status = PkgRepoStatus::default();
    let res_st1 = rust_pkgrepo_ctl(3, &mut status as *mut _ as *mut u8);
    if res_st1 != 0 || status.is_running != 0 {
        failures |= 2048;
    }

    let res_start = rust_pkgrepo_ctl(1, 8080 as *mut u8);
    if res_start != 0 {
        failures |= 4096;
    }

    let res_st2 = rust_pkgrepo_ctl(3, &mut status as *mut _ as *mut u8);
    if res_st2 != 0 || status.is_running != 1 || status.port != 8080 {
        failures |= 8192;
    }

    let res_stop = rust_pkgrepo_ctl(2, core::ptr::null_mut());
    if res_stop != 0 {
        failures |= 16384;
    }

    let _ = rust_pkgrepo_ctl(3, &mut status as *mut _ as *mut u8);
    if status.is_running != 0 {
        failures |= 32768;
    }

    failures
}

#[cfg(test)]
fn mock_vfs_read(fname: &[u8], buf: &mut [u8]) -> i32 {
    if fname.starts_with(b"EDITOR.PKG") {
        // Fabricate valid signed EDITOR.PKG
        let mut pkg = [0u8; 128 + 62];
        pkg[0..4].copy_from_slice(b"NVPK");
        pkg[4..10].copy_from_slice(b"EDITOR");
        pkg[20..25].copy_from_slice(b"1.0.0");
        pkg[28..40].copy_from_slice(b"Text Editor");
        pkg[92..96].copy_from_slice(&62u32.to_le_bytes());
        for i in 0..62 {
            pkg[128 + i] = b'A';
        }
        let sig = crate::pkgsign_core::compute_signature(&pkg[..128], &pkg[128..128 + 62]);
        pkg[96..128].copy_from_slice(&sig);

        let copy_len = pkg.len().min(buf.len());
        buf[..copy_len].copy_from_slice(&pkg[..copy_len]);
        copy_len as i32
    } else {
        -1
    }
}

#[cfg(test)]
static mut MOCK_RECV_DATA: Option<Vec<u8>> = None;

#[cfg(test)]
fn mock_tcp_recv(_sock: i32, buf: &mut [u8]) -> i32 {
    unsafe {
        if let Some(ref d) = MOCK_RECV_DATA {
            let len = d.len().min(buf.len());
            buf[..len].copy_from_slice(&d[..len]);
            len as i32
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_request_parsing() {
        let req = b"GET /packages/EDITOR.PKG HTTP/1.1\r\nHost: 10.0.2.15:8080\r\n\r\n";
        let parsed = parse_http_request(req).unwrap();
        assert_eq!(parsed.method, HttpMethod::Get);
        assert_eq!(parsed.path, b"/packages/EDITOR.PKG");

        let head = b"HEAD /index HTTP/1.0\r\n\r\n";
        let p_head = parse_http_request(head).unwrap();
        assert_eq!(p_head.method, HttpMethod::Head);
        assert_eq!(p_head.path, b"/index");

        let bad = b"GARBAGE";
        assert!(parse_http_request(bad).is_err());
    }

    #[test]
    fn test_extract_package_filename() {
        let mut out = [0u8; 13];
        assert!(extract_package_filename(b"/packages/editor.pkg", &mut out));
        assert_eq!(&out[..10], b"EDITOR.PKG");

        let mut out2 = [0u8; 13];
        assert!(extract_package_filename(b"/game", &mut out2));
        assert_eq!(&out2[..8], b"GAME.PKG");

        let mut out3 = [0u8; 13];
        assert!(!extract_package_filename(b"/packages/toolongpackagename.pkg", &mut out3));
    }

    #[test]
    fn test_response_builders() {
        let mut buf = [0u8; 256];
        let len = build_http_200_headers(&mut buf, b"text/plain", 42);
        let s = std::str::from_utf8(&buf[..len]).unwrap();
        assert!(s.contains("HTTP/1.1 200 OK"));
        assert!(s.contains("Content-Length: 42"));

        let mut buf404 = [0u8; 256];
        let l404 = build_http_404(&mut buf404);
        let s404 = std::str::from_utf8(&buf404[..l404]).unwrap();
        assert!(s404.contains("404 Not Found"));
    }

    #[test]
    fn test_server_selftest() {
        assert_eq!(rust_pkgrepo_selftest(), 0);
    }
}
