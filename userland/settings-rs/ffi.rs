//! userland/settings-rs/ffi.rs - FFI bindings to NovaOS syscalls (VFS, graphics, mouse, firewall).

#![allow(dead_code, static_mut_refs)]

/// Matches kernel/drivers/mouse/ps2mouse.h's mouse_state_t.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

/// Matches nova_fb_abi.h's nova_fb_info_t (36 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbInfo {
    pub struct_size: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub backend: u32,
    pub backend_mask: u32,
    pub caps: u32,
    pub max_surface_dim: u32,
    pub max_surfaces: u32,
}

/// Matches nova_fb_abi.h's nova_fb_create_t (28 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbCreate {
    pub width: u32,
    pub height: u32,
    pub flags: u32,
    pub handle: u32,
    pub pixels: u32,
    pub stride: u32,
    pub size: u32,
}

/// Matches nova_fb_abi.h's nova_fb_present_t (32 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbPresent {
    pub handle: u32,
    pub src_x: i32,
    pub src_y: i32,
    pub width: i32,
    pub height: i32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub flags: u32,
}

pub const NOVA_FB_BACKEND_ANY: u32 = 0;

// ===========================================================================
// NovaOS Stateful Firewall ABI (nova_fw_abi.h)
// ===========================================================================

pub const NOVA_SYS_FW_INFO: u32 = 73;
pub const NOVA_SYS_FW_CTL: u32 = 74;

pub const NOVA_FW_OP_ADD_RULE: u32 = 1;
pub const NOVA_FW_OP_DEL_RULE: u32 = 2;
pub const NOVA_FW_OP_FLUSH_RULES: u32 = 3;
pub const NOVA_FW_OP_SET_POLICY: u32 = 4;
pub const NOVA_FW_OP_SET_ENABLED: u32 = 5;
pub const NOVA_FW_OP_LOCK: u32 = 6;
pub const NOVA_FW_OP_FLUSH_CONNS: u32 = 7;
pub const NOVA_FW_OP_PROBE: u32 = 8;
pub const NOVA_FW_OP_GET_RULE: u32 = 9;
pub const NOVA_FW_OP_GET_CONN: u32 = 10;

pub const NOVA_FW_APPEND: u32 = 0xFFFFFFFF;

pub const NOVA_FW_DIR_IN: u32 = 0;
pub const NOVA_FW_DIR_OUT: u32 = 1;

pub const NOVA_FW_PROTO_ICMP: u32 = 1;
pub const NOVA_FW_PROTO_TCP: u32 = 6;
pub const NOVA_FW_PROTO_UDP: u32 = 17;

pub const NOVA_FW_ST_NEW: u32 = 1;
pub const NOVA_FW_ST_ESTABLISHED: u32 = 2;
pub const NOVA_FW_ST_RELATED: u32 = 4;
pub const NOVA_FW_ST_INVALID: u32 = 8;

pub const NOVA_FW_RL_DIR: usize = 0;
pub const NOVA_FW_RL_PROTO: usize = 1;
pub const NOVA_FW_RL_ACCEPT: usize = 2;
pub const NOVA_FW_RL_LOG: usize = 3;
pub const NOVA_FW_RL_SRC: usize = 4;
pub const NOVA_FW_RL_SRC_MASK: usize = 5;
pub const NOVA_FW_RL_DST: usize = 6;
pub const NOVA_FW_RL_DST_MASK: usize = 7;
pub const NOVA_FW_RL_SP_SET: usize = 8;
pub const NOVA_FW_RL_SP_LO: usize = 9;
pub const NOVA_FW_RL_SP_HI: usize = 10;
pub const NOVA_FW_RL_DP_SET: usize = 11;
pub const NOVA_FW_RL_DP_LO: usize = 12;
pub const NOVA_FW_RL_DP_HI: usize = 13;
pub const NOVA_FW_RL_MISC: usize = 14;
pub const NOVA_FW_RL_HITS: usize = 15;
pub const NOVA_FW_RL_BYTES: usize = 16;

/// Matches nova_fw_abi.h's nova_fw_info_t (24 * 4 = 96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct NovaFwInfo {
    pub enabled: u32,
    pub locked: u32,
    pub policy_in: u32,   // 1 accept, 0 drop
    pub policy_out: u32,  // 1 accept, 0 drop
    pub rules: u32,
    pub conns: u32,
    pub expects: u32,
    pub accepted_in: u32,
    pub accepted_out: u32,
    pub dropped_in: u32,
    pub dropped_out: u32,
    pub invalid: u32,
    pub fragments: u32,
    pub table_full: u32,
    pub half_open: u32,
    pub new_conns: u32,
    pub related: u32,
    pub unsupported: u32,
    pub malformed: u32,
    pub logged: u32,
    pub max_rules: u32,
    pub max_conns: u32,
    pub tick_hz: u32,
    pub reserved: u32,
}

/// Matches nova_fw_abi.h's nova_fw_req_t (380 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaFwReq {
    pub op: u32,
    pub arg0: u32,
    pub arg1: u32,
    pub text: [u8; 128],
    pub probe_in: [u32; 18],
    pub probe_out: [u32; 8],
    pub rule: [u32; 18],
    pub conn: [u32; 16],
}

impl Default for NovaFwReq {
    fn default() -> Self {
        NovaFwReq {
            op: 0,
            arg0: 0,
            arg1: 0,
            text: [0; 128],
            probe_in: [0; 18],
            probe_out: [0; 8],
            rule: [0; 18],
            conn: [0; 16],
        }
    }
}

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_yield();
    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;

    // VFS syscalls
    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
    pub fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32;
    pub fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32;
    pub fn sys_delete_file(filename: *const u8) -> i32;

    // Framebuffer graphics API
    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_destroy(handle: u32) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;

    // Firewall syscalls (SYS_FW_INFO, SYS_FW_CTL)
    pub fn sys_fw_info(out: *mut NovaFwInfo) -> i32;
    pub fn sys_fw_ctl(req: *mut NovaFwReq) -> i32;

    // Service Manager syscalls (SYS_SERVICE_STATUS)
    pub fn sys_service_query(out: *mut NovaServiceStatus) -> i32;
    pub fn sys_service_control(action: u32, name: *const u8) -> i32;
}

// ===========================================================================
// Service Supervisor ABI (nova_svc_abi.h)
// ===========================================================================

pub const NOVA_SYS_SERVICE_STATUS: u32 = 82;

pub const NOVA_SVC_OP_QUERY: u32 = 1;
pub const NOVA_SVC_OP_PUBLISH: u32 = 2;
pub const NOVA_SVC_OP_CONTROL: u32 = 3;
pub const NOVA_SVC_OP_GET_ACTION: u32 = 4;

pub const NOVA_SVC_STATE_STOPPED: u32 = 0;
pub const NOVA_SVC_STATE_RUNNING: u32 = 1;
pub const NOVA_SVC_STATE_RESTARTING: u32 = 2;
pub const NOVA_SVC_STATE_CRASHED: u32 = 3;
pub const NOVA_SVC_STATE_BLOCKED: u32 = 4;

pub const NOVA_SVC_ACTION_NONE: u32 = 0;
pub const NOVA_SVC_ACTION_RESTART: u32 = 1;
pub const NOVA_SVC_ACTION_STOP: u32 = 2;
pub const NOVA_SVC_ACTION_START: u32 = 3;

pub const NOVA_SVC_MAX_SERVICES: usize = 16;
pub const NOVA_SVC_MAX_NAME: usize = 24;
pub const NOVA_SVC_MAX_PATH: usize = 16;
pub const NOVA_SVC_MAX_ARG: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NovaServiceInfo {
    pub name: [u8; NOVA_SVC_MAX_NAME],
    pub exec_path: [u8; NOVA_SVC_MAX_PATH],
    pub arg: [u8; NOVA_SVC_MAX_ARG],
    pub depends_on: [u8; NOVA_SVC_MAX_NAME],
    pub pid: i32,
    pub state: u32,
    pub restart_policy: u32,
    pub restart_count: u32,
    pub uptime_ticks: u32,
    pub trusted: u32,
    pub last_exit_code: i32,
    pub reserved: [u32; 3],
}

impl NovaServiceInfo {
    pub const fn empty() -> Self {
        NovaServiceInfo {
            name: [0u8; NOVA_SVC_MAX_NAME],
            exec_path: [0u8; NOVA_SVC_MAX_PATH],
            arg: [0u8; NOVA_SVC_MAX_ARG],
            depends_on: [0u8; NOVA_SVC_MAX_NAME],
            pid: -1,
            state: NOVA_SVC_STATE_STOPPED,
            restart_policy: 0,
            restart_count: 0,
            uptime_ticks: 0,
            trusted: 0,
            last_exit_code: 0,
            reserved: [0u32; 3],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NovaServiceStatus {
    pub count: u32,
    pub supervisor_pid: u32,
    pub supervisor_uptime_ticks: u32,
    pub generation: u32,
    pub pending_action: u32,
    pub pending_name: [u8; NOVA_SVC_MAX_NAME],
    pub services: [NovaServiceInfo; NOVA_SVC_MAX_SERVICES],
}

impl NovaServiceStatus {
    pub const fn empty() -> Self {
        NovaServiceStatus {
            count: 0,
            supervisor_pid: 1,
            supervisor_uptime_ticks: 0,
            generation: 0,
            pending_action: NOVA_SVC_ACTION_NONE,
            pending_name: [0u8; NOVA_SVC_MAX_NAME],
            services: [NovaServiceInfo::empty(); NOVA_SVC_MAX_SERVICES],
        }
    }
}

// ===========================================================================
// Host Unit Testing Mocks
// ===========================================================================

#[cfg(test)]
pub static mut MOCK_FW_ENABLED: u32 = 1;
#[cfg(test)]
pub static mut MOCK_FW_LOCKED: u32 = 0;
#[cfg(test)]
pub static mut MOCK_FW_POLICY_IN: u32 = 0; // drop
#[cfg(test)]
pub static mut MOCK_FW_POLICY_OUT: u32 = 1; // accept
#[cfg(test)]
pub static mut MOCK_FW_RULES_COUNT: usize = 2;
#[cfg(test)]
pub static mut MOCK_FW_RULES: [[u32; 18]; 32] = [[0; 18]; 32];
#[cfg(test)]
pub static mut MOCK_VFS_FILE_SAVED: [u8; 1024] = [0; 1024];
#[cfg(test)]
pub static mut MOCK_VFS_SAVED_LEN: usize = 0;
#[cfg(test)]
pub static mut MOCK_VFS_USERS_DATA: [u8; 716] = [0; 716];
#[cfg(test)]
pub static mut MOCK_SVC_STATUS: NovaServiceStatus = NovaServiceStatus::empty();
#[cfg(test)]
pub static mut MOCK_SVC_LAST_ACTION: u32 = 0;
#[cfg(test)]
pub static mut MOCK_SVC_LAST_NAME: [u8; NOVA_SVC_MAX_NAME] = [0u8; NOVA_SVC_MAX_NAME];

#[cfg(test)]
pub unsafe fn reset_mock_environment() {
    MOCK_FW_ENABLED = 1;
    MOCK_FW_LOCKED = 0;
    MOCK_FW_POLICY_IN = 0;
    MOCK_FW_POLICY_OUT = 1;
    MOCK_FW_RULES_COUNT = 2;
    MOCK_FW_RULES = [[0; 18]; 32];
    MOCK_VFS_SAVED_LEN = 0;
    MOCK_VFS_USERS_DATA = [0; 716];

    // Default Rule 0: rule accept in state established,related
    MOCK_FW_RULES[0][NOVA_FW_RL_DIR] = NOVA_FW_DIR_IN;
    MOCK_FW_RULES[0][NOVA_FW_RL_PROTO] = 0; // any
    MOCK_FW_RULES[0][NOVA_FW_RL_ACCEPT] = 1;
    MOCK_FW_RULES[0][NOVA_FW_RL_MISC] = NOVA_FW_ST_ESTABLISHED | NOVA_FW_ST_RELATED;
    MOCK_FW_RULES[0][NOVA_FW_RL_HITS] = 124;
    MOCK_FW_RULES[0][NOVA_FW_RL_BYTES] = 9820;

    // Default Rule 1: rule accept in proto icmp icmp-type 8 from 10.0.2.0/24 state new
    MOCK_FW_RULES[1][NOVA_FW_RL_DIR] = NOVA_FW_DIR_IN;
    MOCK_FW_RULES[1][NOVA_FW_RL_PROTO] = NOVA_FW_PROTO_ICMP;
    MOCK_FW_RULES[1][NOVA_FW_RL_ACCEPT] = 1;
    MOCK_FW_RULES[1][NOVA_FW_RL_SRC] = 0x0A000200; // 10.0.2.0
    MOCK_FW_RULES[1][NOVA_FW_RL_SRC_MASK] = 0xFFFFFF00; // /24
    MOCK_FW_RULES[1][NOVA_FW_RL_MISC] = NOVA_FW_ST_NEW | (1 << 8) | (8 << 16); // icmp_set=1, type=8
    MOCK_FW_RULES[1][NOVA_FW_RL_HITS] = 14;
    MOCK_FW_RULES[1][NOVA_FW_RL_BYTES] = 1176;

    // Default USERS.CFG: magic "NVUS", User 0: persisted (700, 700), User 1: admin (800, 0)
    MOCK_VFS_USERS_DATA[0..4].copy_from_slice(b"NVUS");
    // persisted
    let off0 = 4;
    MOCK_VFS_USERS_DATA[off0] = 9; // in_use
    let u0 = b"persisted\0";
    MOCK_VFS_USERS_DATA[off0 + 1..off0 + 1 + u0.len()].copy_from_slice(u0);
    MOCK_VFS_USERS_DATA[off0 + 33..off0 + 37].copy_from_slice(&700u32.to_le_bytes());
    MOCK_VFS_USERS_DATA[off0 + 37..off0 + 41].copy_from_slice(&700u32.to_le_bytes());
    // admin
    let off1 = 4 + 89;
    MOCK_VFS_USERS_DATA[off1] = 5; // in_use
    let u1 = b"admin\0";
    MOCK_VFS_USERS_DATA[off1 + 1..off1 + 1 + u1.len()].copy_from_slice(u1);
    MOCK_VFS_USERS_DATA[off1 + 33..off1 + 37].copy_from_slice(&800u32.to_le_bytes());
    MOCK_VFS_USERS_DATA[off1 + 37..off1 + 41].copy_from_slice(&0u32.to_le_bytes());

    // Default Supervised Services Status
    MOCK_SVC_LAST_ACTION = 0;
    MOCK_SVC_LAST_NAME = [0u8; NOVA_SVC_MAX_NAME];
    MOCK_SVC_STATUS = NovaServiceStatus::empty();
    MOCK_SVC_STATUS.count = 3;
    MOCK_SVC_STATUS.supervisor_pid = 1;
    MOCK_SVC_STATUS.supervisor_uptime_ticks = 1200;
    MOCK_SVC_STATUS.generation = 1;

    // Service 0: network-check (RUNNING, PID 12)
    let s0 = &mut MOCK_SVC_STATUS.services[0];
    let n0 = b"network-check";
    s0.name[..n0.len()].copy_from_slice(n0);
    let p0 = b"PING.ELF";
    s0.exec_path[..p0.len()].copy_from_slice(p0);
    let a0 = b"10.0.2.2";
    s0.arg[..a0.len()].copy_from_slice(a0);
    s0.pid = 12;
    s0.state = NOVA_SVC_STATE_RUNNING;
    s0.restart_policy = 2; // Always
    s0.restart_count = 0;
    s0.trusted = 0;

    // Service 1: greeting (STOPPED, clean exit)
    let s1 = &mut MOCK_SVC_STATUS.services[1];
    let n1 = b"greeting";
    s1.name[..n1.len()].copy_from_slice(n1);
    let p1 = b"HELLO.ELF";
    s1.exec_path[..p1.len()].copy_from_slice(p1);
    let d1 = b"network-check";
    s1.depends_on[..d1.len()].copy_from_slice(d1);
    s1.pid = -1;
    s1.state = NOVA_SVC_STATE_STOPPED;
    s1.restart_policy = 0; // Never
    s1.restart_count = 0;
    s1.trusted = 0;

    // Service 2: shell (RUNNING, PID 15, Trusted)
    let s2 = &mut MOCK_SVC_STATUS.services[2];
    let n2 = b"shell";
    s2.name[..n2.len()].copy_from_slice(n2);
    let p2 = b"SHELL.ELF";
    s2.exec_path[..p2.len()].copy_from_slice(p2);
    s2.pid = 15;
    s2.state = NOVA_SVC_STATE_RUNNING;
    s2.restart_policy = 2; // Always
    s2.restart_count = 0;
    s2.trusted = 1;
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_yield() {}
#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 { -1 }
#[cfg(test)]
pub unsafe fn sys_mouse_read(_out: *mut MouseState) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_open(filename: *const u8) -> i32 {
    let mut len = 0;
    while *filename.add(len) != 0 && len < 64 {
        len += 1;
    }
    let name = core::slice::from_raw_parts(filename, len);
    if name == b"USERS.CFG" {
        return 2;
    }
    if name == b"FIREWALL.CFG" {
        return 4;
    }
    3
}

#[cfg(test)]
pub unsafe fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32 {
    if handle == 2 {
        let len = 716.min(max_len as usize);
        if !buf.is_null() && len > 0 {
            buf.copy_from_nonoverlapping(MOCK_VFS_USERS_DATA.as_ptr(), len);
        }
        return len as i32;
    }
    if handle == 4 {
        let fw_data = b"# FIREWALL.CFG\npolicy in drop\npolicy out accept\nrule accept in state established,related\nrule accept in proto icmp icmp-type 8 from 10.0.2.0/24 state new\n";
        let len = fw_data.len().min(max_len as usize);
        if !buf.is_null() && len > 0 {
            buf.copy_from_nonoverlapping(fw_data.as_ptr(), len);
        }
        return len as i32;
    }
    let dummy = b"NVCFnovaos\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0firoze\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0SHELL.ELF\0\0\0\0";
    let len = dummy.len().min(max_len as usize);
    if !buf.is_null() && len > 0 {
        buf.copy_from_nonoverlapping(dummy.as_ptr(), len);
    }
    len as i32
}

#[cfg(test)]
pub unsafe fn sys_close(_handle: i32) {}
#[cfg(test)]
pub unsafe fn sys_list_files(buf: *mut u8, buf_size: i32) -> i32 {
    let dummy = b"SYSTEM.CFG 81\nUSERS.CFG 716\nSERVICES.CFG 160\nFIREWALL.CFG 512\n";
    let len = dummy.len().min(buf_size as usize);
    if !buf.is_null() && len > 0 {
        buf.copy_from_nonoverlapping(dummy.as_ptr(), len);
    }
    len as i32
}

#[cfg(test)]
pub unsafe fn sys_write_file(filename: *const u8, data: *const u8, size: u32) -> i32 {
    let s = (size as usize).min(1024);
    if !data.is_null() && s > 0 {
        MOCK_VFS_FILE_SAVED[..s].copy_from_slice(core::slice::from_raw_parts(data, s));
        MOCK_VFS_SAVED_LEN = s;

        let mut nlen = 0;
        while *filename.add(nlen) != 0 && nlen < 64 {
            nlen += 1;
        }
        let name = core::slice::from_raw_parts(filename, nlen);
        if name == b"USERS.CFG" && s >= 716 {
            MOCK_VFS_USERS_DATA.copy_from_slice(&core::slice::from_raw_parts(data, 716)[..716]);
        }
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_delete_file(_filename: *const u8) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_info(_out: *mut NovaFbInfo) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_acquire(_backend: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_create(_req: *mut NovaFbCreate) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_destroy(_handle: u32) -> i32 { 0 }
#[cfg(test)]
pub unsafe fn sys_fb_present(_req: *const NovaFbPresent) -> i32 { 0 }

#[cfg(test)]
pub unsafe fn sys_fw_info(out: *mut NovaFwInfo) -> i32 {
    if out.is_null() {
        return -14;
    }
    *out = NovaFwInfo {
        enabled: MOCK_FW_ENABLED,
        locked: MOCK_FW_LOCKED,
        policy_in: MOCK_FW_POLICY_IN,
        policy_out: MOCK_FW_POLICY_OUT,
        rules: MOCK_FW_RULES_COUNT as u32,
        conns: 1,
        expects: 0,
        accepted_in: 138,
        accepted_out: 245,
        dropped_in: 7,
        dropped_out: 0,
        invalid: 1,
        fragments: 0,
        table_full: 0,
        half_open: 0,
        new_conns: 5,
        related: 2,
        unsupported: 0,
        malformed: 0,
        logged: 4,
        max_rules: 32,
        max_conns: 64,
        tick_hz: 100,
        reserved: 0,
    };
    0
}

#[cfg(test)]
pub unsafe fn sys_fw_ctl(req: *mut NovaFwReq) -> i32 {
    if req.is_null() {
        return -14;
    }
    let r = &mut *req;
    if MOCK_FW_LOCKED != 0 {
        match r.op {
            NOVA_FW_OP_ADD_RULE
            | NOVA_FW_OP_DEL_RULE
            | NOVA_FW_OP_FLUSH_RULES
            | NOVA_FW_OP_SET_POLICY
            | NOVA_FW_OP_SET_ENABLED => return -1, // -EPERM
            _ => {}
        }
    }
    match r.op {
        NOVA_FW_OP_ADD_RULE => {
            if MOCK_FW_RULES_COUNT >= 32 {
                return -28; // ENOSPC
            }
            let idx = if r.arg0 == NOVA_FW_APPEND || (r.arg0 as usize) >= MOCK_FW_RULES_COUNT {
                MOCK_FW_RULES_COUNT
            } else {
                r.arg0 as usize
            };
            // Default synthesized rule entry for test
            let mut rule_entry = [0u32; 18];
            rule_entry[NOVA_FW_RL_DIR] = NOVA_FW_DIR_IN;
            rule_entry[NOVA_FW_RL_ACCEPT] = 1;
            rule_entry[NOVA_FW_RL_PROTO] = NOVA_FW_PROTO_TCP;
            rule_entry[NOVA_FW_RL_DP_SET] = 1;
            rule_entry[NOVA_FW_RL_DP_LO] = 22;
            rule_entry[NOVA_FW_RL_DP_HI] = 22;
            rule_entry[NOVA_FW_RL_MISC] = NOVA_FW_ST_NEW;

            if idx < MOCK_FW_RULES_COUNT {
                for i in (idx..MOCK_FW_RULES_COUNT).rev() {
                    MOCK_FW_RULES[i + 1] = MOCK_FW_RULES[i];
                }
            }
            MOCK_FW_RULES[idx] = rule_entry;
            MOCK_FW_RULES_COUNT += 1;
            0
        }
        NOVA_FW_OP_DEL_RULE => {
            let idx = r.arg0 as usize;
            if idx >= MOCK_FW_RULES_COUNT {
                return -2; // ENOENT
            }
            for i in idx..MOCK_FW_RULES_COUNT - 1 {
                MOCK_FW_RULES[i] = MOCK_FW_RULES[i + 1];
            }
            MOCK_FW_RULES[MOCK_FW_RULES_COUNT - 1] = [0; 18];
            MOCK_FW_RULES_COUNT -= 1;
            0
        }
        NOVA_FW_OP_FLUSH_RULES => {
            MOCK_FW_RULES = [[0; 18]; 32];
            MOCK_FW_RULES_COUNT = 0;
            0
        }
        NOVA_FW_OP_SET_POLICY => {
            if r.arg0 == NOVA_FW_DIR_IN {
                MOCK_FW_POLICY_IN = r.arg1;
            } else {
                MOCK_FW_POLICY_OUT = r.arg1;
            }
            0
        }
        NOVA_FW_OP_SET_ENABLED => {
            MOCK_FW_ENABLED = r.arg0;
            0
        }
        NOVA_FW_OP_LOCK => {
            MOCK_FW_LOCKED = 1;
            0
        }
        NOVA_FW_OP_FLUSH_CONNS => 0,
        NOVA_FW_OP_GET_RULE => {
            let idx = r.arg0 as usize;
            if idx >= MOCK_FW_RULES_COUNT {
                return -2; // ENOENT
            }
            r.rule = MOCK_FW_RULES[idx];
            0
        }
        _ => -22, // EINVAL
    }
}

#[cfg(test)]
pub unsafe fn sys_service_query(out: *mut NovaServiceStatus) -> i32 {
    if out.is_null() {
        return -14;
    }
    *out = MOCK_SVC_STATUS;
    0
}

#[cfg(test)]
pub unsafe fn sys_service_control(action: u32, name: *const u8) -> i32 {
    MOCK_SVC_LAST_ACTION = action;
    MOCK_SVC_LAST_NAME = [0u8; NOVA_SVC_MAX_NAME];
    if !name.is_null() {
        let mut len = 0;
        while *name.add(len) != 0 && len < NOVA_SVC_MAX_NAME - 1 {
            len += 1;
        }
        MOCK_SVC_LAST_NAME[..len].copy_from_slice(core::slice::from_raw_parts(name, len));

        for i in 0..MOCK_SVC_STATUS.count as usize {
            let sname = &MOCK_SVC_STATUS.services[i].name;
            let mut slen = 0;
            while slen < NOVA_SVC_MAX_NAME && sname[slen] != 0 { slen += 1; }
            if &sname[..slen] == &MOCK_SVC_LAST_NAME[..len] {
                if action == NOVA_SVC_ACTION_RESTART {
                    MOCK_SVC_STATUS.services[i].state = NOVA_SVC_STATE_RUNNING;
                    MOCK_SVC_STATUS.services[i].pid = 99;
                    MOCK_SVC_STATUS.services[i].restart_count += 1;
                } else if action == NOVA_SVC_ACTION_STOP {
                    MOCK_SVC_STATUS.services[i].state = NOVA_SVC_STATE_STOPPED;
                    MOCK_SVC_STATUS.services[i].pid = -1;
                } else if action == NOVA_SVC_ACTION_START {
                    MOCK_SVC_STATUS.services[i].state = NOVA_SVC_STATE_RUNNING;
                    MOCK_SVC_STATUS.services[i].pid = 101;
                }
                break;
            }
        }
    }
    0
}
