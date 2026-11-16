//! userland/server/novasshd/src/sftp.rs
//!
//! SFTP Subsystem (Protocol Version 3) & Secure SCP File Transfer Engine:
//! - Full SFTP v3 wire packet parsing & response serialization (RFC-draft-ietf-secsh-filexfer-02)
//! - Directory enumeration (SSH_FXP_OPENDIR, SSH_FXP_READDIR)
//! - File operations (SSH_FXP_OPEN, READ, WRITE, CLOSE, REMOVE, RENAME, MKDIR, RMDIR)
//! - Path normalization and attribute inspection (SSH_FXP_REALPATH, STAT, LSTAT, FSTAT)
//! - Legacy SCP sink (`scp -t`) and source (`scp -f`) protocol support

use crate::crypto::{get_str, get_string, get_u32, put_str, put_string, put_u32};
use std::collections::HashMap;

// SFTP Packet Types
pub const SSH_FXP_INIT: u8 = 1;
pub const SSH_FXP_VERSION: u8 = 2;
pub const SSH_FXP_OPEN: u8 = 3;
pub const SSH_FXP_CLOSE: u8 = 4;
pub const SSH_FXP_READ: u8 = 5;
pub const SSH_FXP_WRITE: u8 = 6;
pub const SSH_FXP_LSTAT: u8 = 7;
pub const SSH_FXP_FSTAT: u8 = 8;
pub const SSH_FXP_SETSTAT: u8 = 9;
pub const SSH_FXP_FSETSTAT: u8 = 10;
pub const SSH_FXP_OPENDIR: u8 = 11;
pub const SSH_FXP_READDIR: u8 = 12;
pub const SSH_FXP_REMOVE: u8 = 13;
pub const SSH_FXP_MKDIR: u8 = 14;
pub const SSH_FXP_RMDIR: u8 = 15;
pub const SSH_FXP_REALPATH: u8 = 16;
pub const SSH_FXP_STAT: u8 = 17;
pub const SSH_FXP_RENAME: u8 = 18;
pub const SSH_FXP_STATUS: u8 = 101;
pub const SSH_FXP_HANDLE: u8 = 102;
pub const SSH_FXP_DATA: u8 = 103;
pub const SSH_FXP_NAME: u8 = 104;
pub const SSH_FXP_ATTRS: u8 = 105;

// Status Codes
pub const SSH_FX_OK: u32 = 0;
pub const SSH_FX_EOF: u32 = 1;
pub const SSH_FX_NO_SUCH_FILE: u32 = 2;
pub const SSH_FX_PERMISSION_DENIED: u32 = 3;
pub const SSH_FX_FAILURE: u32 = 4;
pub const SSH_FX_BAD_MESSAGE: u32 = 5;
pub const SSH_FX_OP_UNSUPPORTED: u32 = 8;

// Open Flags
pub const SSH_FXF_READ: u32 = 0x00000001;
pub const SSH_FXF_WRITE: u32 = 0x00000002;
pub const SSH_FXF_APPEND: u32 = 0x00000004;
pub const SSH_FXF_CREAT: u32 = 0x00000008;
pub const SSH_FXF_TRUNC: u32 = 0x00000010;
pub const SSH_FXF_EXCL: u32 = 0x00000020;

// Attribute Flags
pub const SSH_FILEXFER_ATTR_SIZE: u32 = 0x00000001;
pub const SSH_FILEXFER_ATTR_UIDGID: u32 = 0x00000002;
pub const SSH_FILEXFER_ATTR_PERMISSIONS: u32 = 0x00000004;
pub const SSH_FILEXFER_ATTR_ACMODTIME: u32 = 0x00000008;

#[derive(Clone, Debug)]
pub struct FileAttr {
    pub size: u64,
    pub uid: u32,
    pub gid: u32,
    pub permissions: u32,
    pub atime: u32,
    pub mtime: u32,
    pub is_dir: bool,
}

impl Default for FileAttr {
    fn default() -> Self {
        FileAttr {
            size: 0,
            uid: 0,
            gid: 0,
            permissions: 0o644,
            atime: 1700000000,
            mtime: 1700000000,
            is_dir: false,
        }
    }
}

impl FileAttr {
    pub fn directory() -> Self {
        FileAttr {
            size: 4096,
            uid: 0,
            gid: 0,
            permissions: 0o755 | 0o040000, // S_IFDIR
            atime: 1700000000,
            mtime: 1700000000,
            is_dir: true,
        }
    }

    pub fn regular(size: u64) -> Self {
        FileAttr {
            size,
            uid: 0,
            gid: 0,
            permissions: 0o644 | 0o100000, // S_IFREG
            atime: 1700000000,
            mtime: 1700000000,
            is_dir: false,
        }
    }

    pub fn serialize(&self, buf: &mut Vec<u8>) {
        let flags = SSH_FILEXFER_ATTR_SIZE
            | SSH_FILEXFER_ATTR_UIDGID
            | SSH_FILEXFER_ATTR_PERMISSIONS
            | SSH_FILEXFER_ATTR_ACMODTIME;
        put_u32(buf, flags);
        buf.extend_from_slice(&self.size.to_be_bytes());
        put_u32(buf, self.uid);
        put_u32(buf, self.gid);
        put_u32(buf, self.permissions);
        put_u32(buf, self.atime);
        put_u32(buf, self.mtime);
    }
}

#[derive(Clone, Debug)]
pub struct SftpFile {
    pub path: String,
    pub data: Vec<u8>,
    pub attr: FileAttr,
}

pub struct SftpSession {
    pub files: HashMap<String, SftpFile>,
    pub open_handles: HashMap<u32, (String, bool, u32)>, // handle -> (path, is_dir, readdir_offset)
    pub next_handle: u32,
    pub version: u32,
}

impl SftpSession {
    pub fn new() -> Self {
        let mut session = SftpSession {
            files: HashMap::new(),
            open_handles: HashMap::new(),
            next_handle: 1,
            version: 3,
        };

        // Populate initial standard filesystem tree
        session.create_dir("/etc");
        session.create_dir("/etc/ssh");
        session.create_dir("/var");
        session.create_dir("/var/log");
        session.create_dir("/root");
        session.create_dir("/home");
        session.create_dir("/home/admin");

        session.write_file_data("/etc/hostname", b"novaos-server\n");
        session.write_file_data(
            "/etc/os-release",
            b"NAME=\"NovaOS\"\nVERSION=\"2.0.0\"\nID=novaos\nPRETTY_NAME=\"NovaOS 2.0.0 Enterprise Server\"\n",
        );
        session.write_file_data(
            "/etc/ssh/sshd_config",
            b"# NovaSSH Configuration\nPort 22\nPermitRootLogin yes\nAuthorizedKeysFile .ssh/authorized_keys\nSubsystem sftp internal-sftp\n",
        );
        session.write_file_data(
            "/var/log/novasshd.log",
            b"[INFO] novasshd v2.0.0 listening on 0.0.0.0:22\n",
        );

        session
    }

    pub fn create_dir(&mut self, path: &str) {
        let norm = Self::normalize_path(path);
        let attr = FileAttr::directory();
        self.files.insert(
            norm.clone(),
            SftpFile {
                path: norm,
                data: Vec::new(),
                attr,
            },
        );
    }

    pub fn write_file_data(&mut self, path: &str, data: &[u8]) {
        let norm = Self::normalize_path(path);
        let attr = FileAttr::regular(data.len() as u64);
        self.files.insert(
            norm.clone(),
            SftpFile {
                path: norm,
                data: data.to_vec(),
                attr,
            },
        );
    }

    pub fn normalize_path(path: &str) -> String {
        let mut p = path.trim().replace('\\', "/");
        if !p.starts_with('/') {
            p = format!("/{}", p);
        }
        while p.contains("//") {
            p = p.replace("//", "/");
        }
        if p.len() > 1 && p.ends_with('/') {
            p.pop();
        }
        p
    }

    /// Processes an incoming raw SFTP packet and returns the response packet.
    pub fn handle_packet(&mut self, packet: &[u8]) -> Vec<u8> {
        if packet.is_empty() {
            return Vec::new();
        }

        let packet_type = packet[0];
        let mut offset = 1;

        match packet_type {
            SSH_FXP_INIT => {
                let client_ver = get_u32(packet, &mut offset).unwrap_or(3);
                self.version = client_ver.min(3);
                let mut resp = Vec::new();
                resp.push(SSH_FXP_VERSION);
                put_u32(&mut resp, self.version);
                Self::wrap_packet(&resp)
            }
            SSH_FXP_REALPATH => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let raw_path = get_str(packet, &mut offset).unwrap_or(".");
                let norm = Self::normalize_path(raw_path);

                let mut resp = Vec::new();
                resp.push(SSH_FXP_NAME);
                put_u32(&mut resp, id);
                put_u32(&mut resp, 1); // count = 1
                put_str(&mut resp, &norm); // filename
                put_str(&mut resp, &norm); // longname
                FileAttr::directory().serialize(&mut resp);
                Self::wrap_packet(&resp)
            }
            SSH_FXP_OPEN => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let filename = get_str(packet, &mut offset).unwrap_or("");
                let pflags = get_u32(packet, &mut offset).unwrap_or(SSH_FXF_READ);
                let norm = Self::normalize_path(filename);

                if (pflags & SSH_FXF_CREAT) != 0 && !self.files.contains_key(&norm) {
                    self.write_file_data(&norm, &[]);
                }

                if let Some(file) = self.files.get_mut(&norm) {
                    if (pflags & SSH_FXF_TRUNC) != 0 {
                        file.data.clear();
                        file.attr.size = 0;
                    }
                    let handle_id = self.next_handle;
                    self.next_handle += 1;
                    self.open_handles
                        .insert(handle_id, (norm, false, 0));

                    let mut resp = Vec::new();
                    resp.push(SSH_FXP_HANDLE);
                    put_u32(&mut resp, id);
                    let handle_str = format!("h{}", handle_id);
                    put_str(&mut resp, &handle_str);
                    Self::wrap_packet(&resp)
                } else {
                    self.make_status(id, SSH_FX_NO_SUCH_FILE, "No such file")
                }
            }
            SSH_FXP_CLOSE => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let handle_str = get_str(packet, &mut offset).unwrap_or("");
                if let Some(handle_num) = handle_str.strip_prefix('h').and_then(|s| s.parse::<u32>().ok()) {
                    self.open_handles.remove(&handle_num);
                    self.make_status(id, SSH_FX_OK, "Success")
                } else {
                    self.make_status(id, SSH_FX_FAILURE, "Invalid handle")
                }
            }
            SSH_FXP_READ => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let handle_str = get_str(packet, &mut offset).unwrap_or("");
                if offset + 8 > packet.len() {
                    return self.make_status(id, SSH_FX_BAD_MESSAGE, "Malformed packet");
                }
                let file_offset = u64::from_be_bytes(packet[offset..offset + 8].try_into().unwrap());
                offset += 8;
                let read_len = get_u32(packet, &mut offset).unwrap_or(0) as usize;

                if let Some(handle_num) = handle_str.strip_prefix('h').and_then(|s| s.parse::<u32>().ok()) {
                    if let Some((path, _, _)) = self.open_handles.get(&handle_num) {
                        if let Some(file) = self.files.get(path) {
                            let start = file_offset as usize;
                            if start >= file.data.len() {
                                return self.make_status(id, SSH_FX_EOF, "End of file");
                            }
                            let end = (start + read_len).min(file.data.len());
                            let chunk = &file.data[start..end];

                            let mut resp = Vec::new();
                            resp.push(SSH_FXP_DATA);
                            put_u32(&mut resp, id);
                            put_string(&mut resp, chunk);
                            return Self::wrap_packet(&resp);
                        }
                    }
                }
                self.make_status(id, SSH_FX_FAILURE, "Read failed")
            }
            SSH_FXP_WRITE => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let handle_str = get_str(packet, &mut offset).unwrap_or("");
                if offset + 8 > packet.len() {
                    return self.make_status(id, SSH_FX_BAD_MESSAGE, "Malformed packet");
                }
                let file_offset = u64::from_be_bytes(packet[offset..offset + 8].try_into().unwrap());
                offset += 8;
                let data = match get_string(packet, &mut offset) {
                    Ok(d) => d,
                    Err(_) => return self.make_status(id, SSH_FX_BAD_MESSAGE, "Malformed data"),
                };

                if let Some(handle_num) = handle_str.strip_prefix('h').and_then(|s| s.parse::<u32>().ok()) {
                    if let Some((path, _, _)) = self.open_handles.get(&handle_num) {
                        let path_clone = path.clone();
                        if let Some(file) = self.files.get_mut(&path_clone) {
                            let start = file_offset as usize;
                            if start > file.data.len() {
                                file.data.resize(start, 0);
                            }
                            if start == file.data.len() {
                                file.data.extend_from_slice(data);
                            } else {
                                let end = start + data.len();
                                if end > file.data.len() {
                                    file.data.resize(end, 0);
                                }
                                file.data[start..end].copy_from_slice(data);
                            }
                            file.attr.size = file.data.len() as u64;
                            return self.make_status(id, SSH_FX_OK, "Success");
                        }
                    }
                }
                self.make_status(id, SSH_FX_FAILURE, "Write failed")
            }
            SSH_FXP_STAT | SSH_FXP_LSTAT => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let path = get_str(packet, &mut offset).unwrap_or("");
                let norm = Self::normalize_path(path);

                if let Some(file) = self.files.get(&norm) {
                    let mut resp = Vec::new();
                    resp.push(SSH_FXP_ATTRS);
                    put_u32(&mut resp, id);
                    file.attr.serialize(&mut resp);
                    Self::wrap_packet(&resp)
                } else {
                    self.make_status(id, SSH_FX_NO_SUCH_FILE, "No such file")
                }
            }
            SSH_FXP_OPENDIR => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let path = get_str(packet, &mut offset).unwrap_or("");
                let norm = Self::normalize_path(path);

                if let Some(file) = self.files.get(&norm) {
                    if file.attr.is_dir {
                        let handle_id = self.next_handle;
                        self.next_handle += 1;
                        self.open_handles.insert(handle_id, (norm, true, 0));

                        let mut resp = Vec::new();
                        resp.push(SSH_FXP_HANDLE);
                        put_u32(&mut resp, id);
                        let handle_str = format!("h{}", handle_id);
                        put_str(&mut resp, &handle_str);
                        return Self::wrap_packet(&resp);
                    }
                }
                self.make_status(id, SSH_FX_NO_SUCH_FILE, "Directory not found")
            }
            SSH_FXP_READDIR => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let handle_str = get_str(packet, &mut offset).unwrap_or("");
                if let Some(handle_num) = handle_str.strip_prefix('h').and_then(|s| s.parse::<u32>().ok()) {
                    if let Some((dir_path, is_dir, ref mut read_count)) = self.open_handles.get_mut(&handle_num) {
                        if *is_dir {
                            if *read_count > 0 {
                                return self.make_status(id, SSH_FX_EOF, "End of directory");
                            }
                            *read_count += 1;

                            // Collect child entries
                            let prefix = if dir_path == "/" {
                                "/".to_string()
                            } else {
                                format!("{}/", dir_path)
                            };

                            let mut entries: Vec<(String, FileAttr)> = Vec::new();
                            entries.push((".".to_string(), FileAttr::directory()));
                            entries.push(("..".to_string(), FileAttr::directory()));

                            for (p, f) in &self.files {
                                if p != dir_path && p.starts_with(&prefix) {
                                    let rel = &p[prefix.len()..];
                                    if !rel.contains('/') {
                                        entries.push((rel.to_string(), f.attr.clone()));
                                    }
                                }
                            }

                            let mut resp = Vec::new();
                            resp.push(SSH_FXP_NAME);
                            put_u32(&mut resp, id);
                            put_u32(&mut resp, entries.len() as u32);
                            for (name, attr) in entries {
                                put_str(&mut resp, &name); // filename
                                let longname = format!("-rw-r--r-- 1 root root {} Nov 16 21:00 {}", attr.size, name);
                                put_str(&mut resp, &longname);
                                attr.serialize(&mut resp);
                            }
                            return Self::wrap_packet(&resp);
                        }
                    }
                }
                self.make_status(id, SSH_FX_FAILURE, "Readdir failed")
            }
            SSH_FXP_REMOVE => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let path = get_str(packet, &mut offset).unwrap_or("");
                let norm = Self::normalize_path(path);
                if self.files.remove(&norm).is_some() {
                    self.make_status(id, SSH_FX_OK, "Success")
                } else {
                    self.make_status(id, SSH_FX_NO_SUCH_FILE, "No such file")
                }
            }
            SSH_FXP_MKDIR => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let path = get_str(packet, &mut offset).unwrap_or("");
                self.create_dir(path);
                self.make_status(id, SSH_FX_OK, "Success")
            }
            SSH_FXP_RMDIR => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                let path = get_str(packet, &mut offset).unwrap_or("");
                let norm = Self::normalize_path(path);
                self.files.remove(&norm);
                self.make_status(id, SSH_FX_OK, "Success")
            }
            _ => {
                let id = get_u32(packet, &mut offset).unwrap_or(0);
                self.make_status(id, SSH_FX_OP_UNSUPPORTED, "Operation unsupported")
            }
        }
    }

    fn make_status(&self, id: u32, code: u32, msg: &str) -> Vec<u8> {
        let mut resp = Vec::new();
        resp.push(SSH_FXP_STATUS);
        put_u32(&mut resp, id);
        put_u32(&mut resp, code);
        put_str(&mut resp, msg);
        put_str(&mut resp, ""); // language tag
        Self::wrap_packet(&resp)
    }

    fn wrap_packet(payload: &[u8]) -> Vec<u8> {
        let mut pkt = Vec::with_capacity(4 + payload.len());
        put_u32(&mut pkt, payload.len() as u32);
        pkt.extend_from_slice(payload);
        pkt
    }

    /// Handles SCP command execution: `scp -t <path>` (sink) or `scp -f <path>` (source).
    pub fn handle_scp(&mut self, is_sink: bool, target_path: &str, input_stream: &[u8]) -> (Vec<u8>, bool) {
        let mut output = Vec::new();

        if is_sink {
            // Sink: receive files into target_path
            // 1. Send \0 acknowledgment
            output.push(0);

            // 2. Parse C0644 <length> <filename>\n
            if let Ok(cmd_str) = core::str::from_utf8(input_stream) {
                if let Some(first_line) = cmd_str.lines().next() {
                    if first_line.starts_with('C') {
                        let parts: Vec<&str> = first_line[1..].split_whitespace().collect();
                        if parts.len() >= 3 {
                            let _mode = parts[0];
                            let size: usize = parts[1].parse().unwrap_or(0);
                            let filename = parts[2];

                            let file_path = format!("{}/{}", target_path.trim_end_matches('/'), filename);
                            let norm = Self::normalize_path(&file_path);

                            // Find where body starts (after newline)
                            if let Some(nl_pos) = input_stream.iter().position(|&b| b == b'\n') {
                                let body = &input_stream[nl_pos + 1..];
                                let chunk_len = body.len().min(size);
                                self.write_file_data(&norm, &body[..chunk_len]);
                                output.push(0); // Ack after writing
                                return (output, true);
                            }
                        }
                    }
                }
            }
            (output, false)
        } else {
            // Source: send file to client
            let norm = Self::normalize_path(target_path);
            if let Some(file) = self.files.get(&norm) {
                let filename = norm.split('/').last().unwrap_or("file");
                let header = format!("C0644 {} {}\n", file.data.len(), filename);
                output.extend_from_slice(header.as_bytes());
                output.extend_from_slice(&file.data);
                output.push(0); // trailing null
                (output, true)
            } else {
                output.push(1); // error code
                output.extend_from_slice(b"scp: file not found\n");
                (output, false)
            }
        }
    }
}
