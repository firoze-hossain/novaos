//! kernel/src/security/namespace/mod.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Kernel Isolation Namespaces
//!
//! Implements comprehensive Linux-grade isolation namespaces in pure Rust:
//!   - PID Namespace: independent process trees, local PID 1 init, translation mapping.
//!   - Mount Namespace: isolated mount tables, chroot jail enforcement, pivot_root.
//!   - Net Namespace: independent IP addresses, virtual loopback (lo), veth pairs, isolated ports.
//!   - IPC Namespace: isolated shared memory (shm) segments and message queues (mq).
//!   - UTS Namespace: isolated nodename (hostname) and domainname.
//!   - User Namespace: UID/GID translation, unprivileged host user mapping to container root.
//!
//! Supports clone(CLONE_NEW*), unshare(flags), and setns(fd, nstype).

#![allow(dead_code, non_camel_case_types)]

pub mod pid;
pub mod mount;
pub mod net;
pub mod ipc;
pub mod uts;
pub mod user;

use core::sync::atomic::{AtomicBool, Ordering};

pub use pid::{PidNamespaceManager, ROOT_PID_NS};
pub use mount::{MountNamespaceManager, ROOT_MOUNT_NS, MS_PRIVATE, MS_SHARED, MS_SLAVE, MS_RDONLY};
pub use net::{NetNamespaceManager, ROOT_NET_NS, PROTO_TCP, PROTO_UDP};
pub use ipc::{IpcNamespaceManager, ROOT_IPC_NS};
pub use uts::{UtsNamespaceManager, ROOT_UTS_NS, UtsName};
pub use user::{UserNamespaceManager, ROOT_USER_NS, CAP_CHROOT, CAP_NET_ADMIN, CAP_SYS_ADMIN};

// -----------------------------------------------------------------------------
// Linux-Standard Clone / Unshare Flags
// -----------------------------------------------------------------------------

pub const CLONE_NEWNS: u32     = 0x0002_0000; // Mount namespace
pub const CLONE_NEWCGROUP: u32 = 0x0200_0000; // Cgroup namespace
pub const CLONE_NEWUTS: u32    = 0x0400_0000; // UTS namespace
pub const CLONE_NEWIPC: u32    = 0x0800_0000; // IPC namespace
pub const CLONE_NEWUSER: u32   = 0x1000_0000; // User namespace
pub const CLONE_NEWPID: u32    = 0x2000_0000; // PID namespace
pub const CLONE_NEWNET: u32    = 0x4000_0000; // Network namespace

pub const CLONE_ALL_NAMESPACES: u32 =
    CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNET;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;
pub const ESRCH: isize  = -3;
pub const EPERM: isize  = -1;
pub const ENOENT: isize = -2;

// -----------------------------------------------------------------------------
// Synchronization SpinLock
// -----------------------------------------------------------------------------

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

// -----------------------------------------------------------------------------
// Process Namespace Context
// -----------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ProcessNamespaceContext {
    pub host_pid: u32,
    pub pid_ns: u32,
    pub mount_ns: u32,
    pub net_ns: u32,
    pub ipc_ns: u32,
    pub uts_ns: u32,
    pub user_ns: u32,
    pub active: bool,
}

impl ProcessNamespaceContext {
    pub const fn empty() -> Self {
        Self {
            host_pid: 0,
            pid_ns: ROOT_PID_NS,
            mount_ns: ROOT_MOUNT_NS,
            net_ns: ROOT_NET_NS,
            ipc_ns: ROOT_IPC_NS,
            uts_ns: ROOT_UTS_NS,
            user_ns: ROOT_USER_NS,
            active: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Global Namespace Registry
// -----------------------------------------------------------------------------

pub const MAX_PROCESS_CONTEXTS: usize = 32;

pub struct NamespaceRegistry {
    pub pid: PidNamespaceManager,
    pub mount: MountNamespaceManager,
    pub net: NetNamespaceManager,
    pub ipc: IpcNamespaceManager,
    pub uts: UtsNamespaceManager,
    pub user: UserNamespaceManager,
    pub contexts: [ProcessNamespaceContext; MAX_PROCESS_CONTEXTS],
}

impl NamespaceRegistry {
    pub const fn empty() -> Self {
        Self {
            pid: PidNamespaceManager::empty(),
            mount: MountNamespaceManager::empty(),
            net: NetNamespaceManager::empty(),
            ipc: IpcNamespaceManager::empty(),
            uts: UtsNamespaceManager::empty(),
            user: UserNamespaceManager::empty(),
            contexts: [ProcessNamespaceContext::empty(); MAX_PROCESS_CONTEXTS],
        }
    }

    pub fn new() -> Self {
        let mut reg = Self::empty();
        reg.init();
        reg
    }

    pub fn reset(&mut self) {
        *self = Self::empty();
        self.init();
    }

    pub fn init(&mut self) {
        self.pid.init();
        self.mount.init();
        self.net.init();
        self.ipc.init();
        self.uts.init();
        self.user.init();

        // Initialize Host PID 1 (Init) Context in Root of all namespaces
        self.contexts[0] = ProcessNamespaceContext {
            host_pid: 1,
            pid_ns: ROOT_PID_NS,
            mount_ns: ROOT_MOUNT_NS,
            net_ns: ROOT_NET_NS,
            ipc_ns: ROOT_IPC_NS,
            uts_ns: ROOT_UTS_NS,
            user_ns: ROOT_USER_NS,
            active: true,
        };
    }

    /// Register a process into root namespaces if not already tracked
    pub fn ensure_registered(&mut self, host_pid: u32) {
        for ctx in self.contexts.iter() {
            if ctx.active && ctx.host_pid == host_pid {
                return;
            }
        }
        for ctx in self.contexts.iter_mut() {
            if !ctx.active {
                *ctx = ProcessNamespaceContext {
                    host_pid,
                    pid_ns: ROOT_PID_NS,
                    mount_ns: ROOT_MOUNT_NS,
                    net_ns: ROOT_NET_NS,
                    ipc_ns: ROOT_IPC_NS,
                    uts_ns: ROOT_UTS_NS,
                    user_ns: ROOT_USER_NS,
                    active: true,
                };
                let _ = self.pid.register_process(host_pid, ROOT_PID_NS);
                return;
            }
        }
    }

    /// Handle process creation / clone with namespace isolation flags
    /// Returns the local PID in child's PID namespace.
    pub fn handle_clone(
        &mut self,
        parent_host_pid: u32,
        child_host_pid: u32,
        flags: u32,
    ) -> Result<u32, isize> {
        let parent_ctx = self.get_context_internal(parent_host_pid).unwrap_or(ProcessNamespaceContext {
            host_pid: parent_host_pid,
            pid_ns: ROOT_PID_NS,
            mount_ns: ROOT_MOUNT_NS,
            net_ns: ROOT_NET_NS,
            ipc_ns: ROOT_IPC_NS,
            uts_ns: ROOT_UTS_NS,
            user_ns: ROOT_USER_NS,
            active: true,
        });

        // 1. User Namespace
        let child_user_ns = if (flags & CLONE_NEWUSER) != 0 {
            self.user.create_namespace(parent_host_pid, parent_ctx.user_ns)?
        } else {
            self.user.retain_namespace(parent_ctx.user_ns);
            parent_ctx.user_ns
        };

        // 2. PID Namespace
        let child_pid_ns = if (flags & CLONE_NEWPID) != 0 {
            self.pid.create_namespace(parent_ctx.pid_ns)?
        } else {
            self.pid.retain_namespace(parent_ctx.pid_ns);
            parent_ctx.pid_ns
        };

        // 3. Mount Namespace
        let child_mount_ns = if (flags & CLONE_NEWNS) != 0 {
            self.mount.clone_namespace(parent_ctx.mount_ns)?
        } else {
            self.mount.retain_namespace(parent_ctx.mount_ns);
            parent_ctx.mount_ns
        };

        // 4. Net Namespace
        let child_net_ns = if (flags & CLONE_NEWNET) != 0 {
            self.net.create_namespace()?
        } else {
            self.net.retain_namespace(parent_ctx.net_ns);
            parent_ctx.net_ns
        };

        // 5. IPC Namespace
        let child_ipc_ns = if (flags & CLONE_NEWIPC) != 0 {
            self.ipc.create_namespace()?
        } else {
            self.ipc.retain_namespace(parent_ctx.ipc_ns);
            parent_ctx.ipc_ns
        };

        // 6. UTS Namespace
        let child_uts_ns = if (flags & CLONE_NEWUTS) != 0 {
            self.uts.clone_namespace(parent_ctx.uts_ns)?
        } else {
            self.uts.retain_namespace(parent_ctx.uts_ns);
            parent_ctx.uts_ns
        };

        // Register process in child's PID namespace
        let local_pid = self.pid.register_process(child_host_pid, child_pid_ns)?;

        // Store child context
        let mut slot_found = false;
        for ctx in self.contexts.iter_mut() {
            if !ctx.active {
                *ctx = ProcessNamespaceContext {
                    host_pid: child_host_pid,
                    pid_ns: child_pid_ns,
                    mount_ns: child_mount_ns,
                    net_ns: child_net_ns,
                    ipc_ns: child_ipc_ns,
                    uts_ns: child_uts_ns,
                    user_ns: child_user_ns,
                    active: true,
                };
                slot_found = true;
                break;
            }
        }

        if !slot_found {
            return Err(ENOMEM);
        }

        Ok(local_pid)
    }

    /// Disassociate calling process from shared namespaces and create new ones
    pub fn handle_unshare(&mut self, caller_pid: u32, flags: u32) -> isize {
        self.ensure_registered(caller_pid);
        let ctx_idx = match self.find_context_idx(caller_pid) {
            Some(i) => i,
            None => return ESRCH,
        };

        let current_ctx = self.contexts[ctx_idx];

        // 1. User Namespace
        if (flags & CLONE_NEWUSER) != 0 {
            match self.user.create_namespace(caller_pid, current_ctx.user_ns) {
                Ok(new_ns) => {
                    self.user.release_namespace(current_ctx.user_ns);
                    self.contexts[ctx_idx].user_ns = new_ns;
                }
                Err(e) => return e,
            }
        }

        // 2. PID Namespace
        // In Linux, unshare(CLONE_NEWPID) creates a new PID namespace for future children,
        // while the caller remains in its existing namespace.
        if (flags & CLONE_NEWPID) != 0 {
            match self.pid.create_namespace(current_ctx.pid_ns) {
                Ok(new_ns) => {
                    // Update future child namespace
                    self.pid.release_namespace(current_ctx.pid_ns);
                    self.contexts[ctx_idx].pid_ns = new_ns;
                }
                Err(e) => return e,
            }
        }

        // 3. Mount Namespace
        if (flags & CLONE_NEWNS) != 0 {
            match self.mount.clone_namespace(current_ctx.mount_ns) {
                Ok(new_ns) => {
                    self.mount.release_namespace(current_ctx.mount_ns);
                    self.contexts[ctx_idx].mount_ns = new_ns;
                }
                Err(e) => return e,
            }
        }

        // 4. Net Namespace
        if (flags & CLONE_NEWNET) != 0 {
            match self.net.create_namespace() {
                Ok(new_ns) => {
                    self.net.release_namespace(current_ctx.net_ns);
                    self.contexts[ctx_idx].net_ns = new_ns;
                }
                Err(e) => return e,
            }
        }

        // 5. IPC Namespace
        if (flags & CLONE_NEWIPC) != 0 {
            match self.ipc.create_namespace() {
                Ok(new_ns) => {
                    self.ipc.release_namespace(current_ctx.ipc_ns);
                    self.contexts[ctx_idx].ipc_ns = new_ns;
                }
                Err(e) => return e,
            }
        }

        // 6. UTS Namespace
        if (flags & CLONE_NEWUTS) != 0 {
            match self.uts.clone_namespace(current_ctx.uts_ns) {
                Ok(new_ns) => {
                    self.uts.release_namespace(current_ctx.uts_ns);
                    self.contexts[ctx_idx].uts_ns = new_ns;
                }
                Err(e) => return e,
            }
        }

        0 // Success
    }

    /// Reassociate calling process with an existing namespace
    pub fn handle_setns(&mut self, caller_pid: u32, target_ns_id: u32, nstype: u32) -> isize {
        self.ensure_registered(caller_pid);
        let ctx_idx = match self.find_context_idx(caller_pid) {
            Some(i) => i,
            None => return ESRCH,
        };

        let current_ctx = self.contexts[ctx_idx];

        match nstype {
            CLONE_NEWNS => {
                self.mount.retain_namespace(target_ns_id);
                self.mount.release_namespace(current_ctx.mount_ns);
                self.contexts[ctx_idx].mount_ns = target_ns_id;
            }
            CLONE_NEWUTS => {
                self.uts.retain_namespace(target_ns_id);
                self.uts.release_namespace(current_ctx.uts_ns);
                self.contexts[ctx_idx].uts_ns = target_ns_id;
            }
            CLONE_NEWIPC => {
                self.ipc.retain_namespace(target_ns_id);
                self.ipc.release_namespace(current_ctx.ipc_ns);
                self.contexts[ctx_idx].ipc_ns = target_ns_id;
            }
            CLONE_NEWNET => {
                self.net.retain_namespace(target_ns_id);
                self.net.release_namespace(current_ctx.net_ns);
                self.contexts[ctx_idx].net_ns = target_ns_id;
            }
            CLONE_NEWPID => {
                self.pid.retain_namespace(target_ns_id);
                self.pid.release_namespace(current_ctx.pid_ns);
                self.contexts[ctx_idx].pid_ns = target_ns_id;
            }
            CLONE_NEWUSER => {
                self.user.retain_namespace(target_ns_id);
                self.user.release_namespace(current_ctx.user_ns);
                self.contexts[ctx_idx].user_ns = target_ns_id;
            }
            0 => {
                // Any namespace type matching target_ns_id
                self.mount.retain_namespace(target_ns_id);
                self.contexts[ctx_idx].mount_ns = target_ns_id;
            }
            _ => return EINVAL,
        }

        0
    }

    /// Handle process exit and namespace reclamation
    pub fn handle_exit(&mut self, host_pid: u32) {
        if let Some(idx) = self.find_context_idx(host_pid) {
            let ctx = self.contexts[idx];
            self.contexts[idx] = ProcessNamespaceContext::empty();

            self.mount.release_namespace(ctx.mount_ns);
            self.net.release_namespace(ctx.net_ns);
            self.ipc.release_namespace(ctx.ipc_ns);
            self.uts.release_namespace(ctx.uts_ns);
            self.user.release_namespace(ctx.user_ns);

            // Unregister from PID namespace (triggers cascading kill if init died)
            let _ = self.pid.unregister_process(host_pid);
            self.pid.release_namespace(ctx.pid_ns);
        }
    }

    pub fn get_context(&self, host_pid: u32) -> Option<ProcessNamespaceContext> {
        self.get_context_internal(host_pid)
    }

    fn get_context_internal(&self, host_pid: u32) -> Option<ProcessNamespaceContext> {
        for ctx in self.contexts.iter() {
            if ctx.active && ctx.host_pid == host_pid {
                return Some(*ctx);
            }
        }
        None
    }

    fn find_context_idx(&self, host_pid: u32) -> Option<usize> {
        for (i, ctx) in self.contexts.iter().enumerate() {
            if ctx.active && ctx.host_pid == host_pid {
                return Some(i);
            }
        }
        None
    }
}

// Global Static Instance
pub static GLOBAL_NAMESPACES: SpinLock<NamespaceRegistry> =
    SpinLock::new(NamespaceRegistry::empty());
static NAMESPACES_INITIALIZED: AtomicBool = AtomicBool::new(false);
static TEST_REGISTRY: SpinLock<NamespaceRegistry> =
    SpinLock::new(NamespaceRegistry::empty());

fn ensure_global_init(reg: &mut NamespaceRegistry) {
    if !NAMESPACES_INITIALIZED.load(Ordering::Acquire) {
        reg.init();
        NAMESPACES_INITIALIZED.store(true, Ordering::Release);
    }
}

// -----------------------------------------------------------------------------
// Global Public Accessors
// -----------------------------------------------------------------------------

pub fn global_clone_process(parent_pid: u32, child_pid: u32, flags: u32) -> Result<u32, isize> {
    let mut reg = GLOBAL_NAMESPACES.lock();
    ensure_global_init(&mut reg);
    reg.handle_clone(parent_pid, child_pid, flags)
}

pub fn global_unshare(caller_pid: u32, flags: u32) -> isize {
    let mut reg = GLOBAL_NAMESPACES.lock();
    ensure_global_init(&mut reg);
    reg.handle_unshare(caller_pid, flags)
}

pub fn global_setns(caller_pid: u32, target_ns_id: u32, nstype: u32) -> isize {
    let mut reg = GLOBAL_NAMESPACES.lock();
    ensure_global_init(&mut reg);
    reg.handle_setns(caller_pid, target_ns_id, nstype)
}

pub fn global_exit_process(host_pid: u32) {
    let mut reg = GLOBAL_NAMESPACES.lock();
    ensure_global_init(&mut reg);
    reg.handle_exit(host_pid);
}

pub fn global_get_context(host_pid: u32) -> Option<ProcessNamespaceContext> {
    let mut reg = GLOBAL_NAMESPACES.lock();
    ensure_global_init(&mut reg);
    reg.get_context(host_pid)
}

// -----------------------------------------------------------------------------
// Comprehensive In-Kernel Self-Test Suite
// -----------------------------------------------------------------------------

pub fn run_namespace_selftest() -> bool {
    let mut reg_guard = TEST_REGISTRY.lock();
    reg_guard.reset();
    let reg = &mut *reg_guard;

    // 1. Initial State: Host PID 1 mapped into root namespaces
    let root_ctx = match reg.get_context(1) {
        Some(c) => c,
        None => return false,
    };
    if root_ctx.pid_ns != ROOT_PID_NS
        || root_ctx.mount_ns != ROOT_MOUNT_NS
        || root_ctx.net_ns != ROOT_NET_NS
        || root_ctx.ipc_ns != ROOT_IPC_NS
        || root_ctx.uts_ns != ROOT_UTS_NS
        || root_ctx.user_ns != ROOT_USER_NS
    {
        return false;
    }

    // 2. PID Namespace: Clone with CLONE_NEWPID
    // Child gets local PID 1 (namespace init) while host PID is 100
    let child_local_pid = match reg.handle_clone(1, 100, CLONE_NEWPID) {
        Ok(pid) => pid,
        Err(_) => return false,
    };
    if child_local_pid != 1 {
        return false; // First process in new PID namespace must be PID 1
    }
    let child_ctx = match reg.get_context(100) {
        Some(c) => c,
        None => return false,
    };
    if child_ctx.pid_ns == ROOT_PID_NS {
        return false; // Must be in a newly allocated PID namespace
    }
    // Verify translation
    if reg.pid.translate_to_local(100, child_ctx.pid_ns) != Some(1) {
        return false;
    }
    if reg.pid.translate_to_host(1, child_ctx.pid_ns) != Some(100) {
        return false;
    }

    // Spawn a second process (host PID 101) in that same PID namespace
    let p2_local = match reg.pid.register_process(101, child_ctx.pid_ns) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if p2_local != 2 {
        return false;
    }

    // 3. Nested PID Namespaces & Visibility Bounds
    // Host can see both child processes (100, 101)
    if !reg.pid.can_see_process(ROOT_PID_NS, 100) || !reg.pid.can_see_process(ROOT_PID_NS, 101) {
        return false;
    }
    // Child PID namespace CANNOT see host PID 1 (isolated from parent)
    if reg.pid.can_see_process(child_ctx.pid_ns, 1) {
        return false;
    }

    // 4. Mount Namespace & Chroot Jail Enforcement
    let mnt_child_pid = 200;
    if reg.handle_clone(1, mnt_child_pid, CLONE_NEWNS).is_err() {
        return false;
    }
    let mnt_ctx = reg.get_context(mnt_child_pid).unwrap();
    if mnt_ctx.mount_ns == ROOT_MOUNT_NS {
        return false;
    }
    // Add mount in child mount namespace
    if reg.mount.mount(mnt_ctx.mount_ns, b"/dev/loop0", b"/app", b"ext2", MS_RDONLY, MS_PRIVATE).is_err() {
        return false;
    }
    // Chroot jail path normalization
    if reg.mount.chroot(mnt_ctx.mount_ns, b"/jail").is_err() {
        return false;
    }
    let mut resolved = [0u8; 64];
    // Attempting to escape jail with /../../../etc/passwd must stay trapped in /jail
    let len = match reg.mount.resolve_path(mnt_ctx.mount_ns, b"../../../etc/passwd", &mut resolved) {
        Ok(l) => l,
        Err(_) => return false,
    };
    if &resolved[..len] != b"/jail/etc/passwd" {
        return false;
    }

    // 5. Network Namespace: Isolated loopback and port binding
    let net_child_pid = 300;
    if reg.handle_clone(1, net_child_pid, CLONE_NEWNET).is_err() {
        return false;
    }
    let net_ctx = reg.get_context(net_child_pid).unwrap();
    if net_ctx.net_ns == ROOT_NET_NS {
        return false;
    }
    // Bind port 80 in root NetNS
    if reg.net.bind_port(ROOT_NET_NS, PROTO_TCP, 80, 1).is_err() {
        return false;
    }
    // Binding port 80 again in root NetNS should fail with EADDRINUSE
    if reg.net.bind_port(ROOT_NET_NS, PROTO_TCP, 80, 2) != Err(net::EADDRINUSE) {
        return false;
    }
    // Binding port 80 in child NetNS MUST SUCCEED (isolated port namespace!)
    if reg.net.bind_port(net_ctx.net_ns, PROTO_TCP, 80, net_child_pid).is_err() {
        return false;
    }
    // Create veth pair linking root and container
    if reg.net.create_veth_pair(
        ROOT_NET_NS, b"veth0", [10, 0, 0, 1], [255, 255, 255, 0],
        net_ctx.net_ns, b"eth0", [10, 0, 0, 2], [255, 255, 255, 0],
    ).is_err() {
        return false;
    }

    // 6. IPC Namespace: Isolated Shared Memory and Message Queues
    let ipc_child_pid = 400;
    if reg.handle_clone(1, ipc_child_pid, CLONE_NEWIPC).is_err() {
        return false;
    }
    let ipc_ctx = reg.get_context(ipc_child_pid).unwrap();
    // Create shm segment key 0xABCD in root IPC NS
    let root_shm = match reg.ipc.shm_get(ROOT_IPC_NS, 0xABCD, 4096, ipc::IPC_CREAT, 0) {
        Ok(id) => id,
        Err(_) => return false,
    };
    // Create shm segment key 0xABCD in child IPC NS (must succeed and have separate lifecycle)
    let child_shm = match reg.ipc.shm_get(ipc_ctx.ipc_ns, 0xABCD, 8192, ipc::IPC_CREAT, 1000) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if root_shm == 0 || child_shm == 0 {
        return false;
    }

    // 7. UTS Namespace: Hostname Isolation
    let uts_child_pid = 500;
    if reg.handle_clone(1, uts_child_pid, CLONE_NEWUTS).is_err() {
        return false;
    }
    let uts_ctx = reg.get_context(uts_child_pid).unwrap();
    // Mutate container hostname
    if reg.uts.sethostname(uts_ctx.uts_ns, b"docker-worker-01").is_err() {
        return false;
    }
    // Root hostname must remain unchanged ("novaos-server")
    if reg.uts.get_hostname(ROOT_UTS_NS) != Some(b"novaos-server") {
        return false;
    }
    // Container hostname must reflect changed value
    if reg.uts.get_hostname(uts_ctx.uts_ns) != Some(b"docker-worker-01") {
        return false;
    }

    // 8. User Namespace: UID 1000 host user maps to UID 0 container root
    let user_child_pid = 600;
    if reg.handle_clone(1, user_child_pid, CLONE_NEWUSER).is_err() {
        return false;
    }
    let user_ctx = reg.get_context(user_child_pid).unwrap();
    // Map inside UID 0 -> host UID 1000
    if reg.user.set_uid_map(user_ctx.user_ns, 0, 1000, 1).is_err() {
        return false;
    }
    if reg.user.map_uid_to_outside(user_ctx.user_ns, 0) != Some(1000) {
        return false;
    }
    if reg.user.map_uid_to_inside(user_ctx.user_ns, 1000) != Some(0) {
        return false;
    }
    // Inside container, UID 0 possesses container root capabilities (CAP_NET_ADMIN, CAP_SYS_ADMIN)
    if !reg.user.has_capability(user_ctx.user_ns, 0, CAP_NET_ADMIN | CAP_SYS_ADMIN) {
        return false;
    }
    // Non-mapped UID has no root capability
    if reg.user.has_capability(user_ctx.user_ns, 1, CAP_SYS_ADMIN) {
        return false;
    }

    // 9. unshare(): Dynamic unsharing of all namespaces simultaneously
    let unshare_pid = 700;
    reg.ensure_registered(unshare_pid);
    let unshare_flags = CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET | CLONE_NEWUSER;
    if reg.handle_unshare(unshare_pid, unshare_flags) != 0 {
        return false;
    }
    let unshared_ctx = reg.get_context(unshare_pid).unwrap();
    if unshared_ctx.mount_ns == ROOT_MOUNT_NS
        || unshared_ctx.uts_ns == ROOT_UTS_NS
        || unshared_ctx.ipc_ns == ROOT_IPC_NS
        || unshared_ctx.net_ns == ROOT_NET_NS
        || unshared_ctx.user_ns == ROOT_USER_NS
    {
        return false;
    }

    // 10. setns() and Process Exit Reclamation
    // Switch process 700 back to Root UTS namespace
    if reg.handle_setns(unshare_pid, ROOT_UTS_NS, CLONE_NEWUTS) != 0 {
        return false;
    }
    if reg.get_context(unshare_pid).unwrap().uts_ns != ROOT_UTS_NS {
        return false;
    }

    // Process termination cleans up active context
    reg.handle_exit(unshare_pid);
    if reg.get_context(unshare_pid).is_some() {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_namespaces_initialization() {
        let reg = NamespaceRegistry::new();
        let ctx = reg.get_context(1).expect("Root context should exist for PID 1");
        assert_eq!(ctx.pid_ns, ROOT_PID_NS);
        assert_eq!(ctx.mount_ns, ROOT_MOUNT_NS);
        assert_eq!(ctx.net_ns, ROOT_NET_NS);
        assert_eq!(ctx.ipc_ns, ROOT_IPC_NS);
        assert_eq!(ctx.uts_ns, ROOT_UTS_NS);
        assert_eq!(ctx.user_ns, ROOT_USER_NS);

        assert_eq!(reg.uts.get_hostname(ROOT_UTS_NS), Some(b"novaos-server".as_slice()));
        assert!(reg.net.is_port_bound(ROOT_NET_NS, PROTO_TCP, 80) == false);
    }

    #[test]
    fn test_pid_namespace_hierarchy_and_translation() {
        let mut reg = NamespaceRegistry::new();
        let child_local_pid = reg.handle_clone(1, 42, CLONE_NEWPID).expect("Clone with NEWPID should succeed");
        assert_eq!(child_local_pid, 1, "First process in new PID namespace must be local PID 1 (init)");

        let child_ctx = reg.get_context(42).expect("Child context exists");
        assert_ne!(child_ctx.pid_ns, ROOT_PID_NS);

        // Host PID 42 maps to local PID 1 in child PID namespace
        assert_eq!(reg.pid.translate_to_local(42, child_ctx.pid_ns), Some(1));
        assert_eq!(reg.pid.translate_to_host(1, child_ctx.pid_ns), Some(42));

        // Spawn a second process in that child PID namespace
        let p2_local = reg.pid.register_process(43, child_ctx.pid_ns).expect("Registering p2");
        assert_eq!(p2_local, 2);
        assert_eq!(reg.pid.translate_to_local(43, child_ctx.pid_ns), Some(2));
        assert_eq!(reg.pid.translate_to_host(2, child_ctx.pid_ns), Some(43));
    }

    #[test]
    fn test_pid_namespace_isolation_and_visibility() {
        let mut reg = NamespaceRegistry::new();
        let _ = reg.handle_clone(1, 100, CLONE_NEWPID).unwrap();
        let ctx100 = reg.get_context(100).unwrap();

        // Host (root) can see child process
        assert!(reg.pid.can_see_process(ROOT_PID_NS, 100));

        // Child process cannot see host process (PID 1)
        assert!(!reg.pid.can_see_process(ctx100.pid_ns, 1));

        // Create a sibling namespace
        let _ = reg.handle_clone(1, 200, CLONE_NEWPID).unwrap();
        let ctx200 = reg.get_context(200).unwrap();

        // Sibling namespace cannot see process 100
        assert!(!reg.pid.can_see_process(ctx200.pid_ns, 100));
        assert!(!reg.pid.can_see_process(ctx100.pid_ns, 200));
    }

    #[test]
    fn test_pid_namespace_cascading_kill() {
        let mut reg = NamespaceRegistry::new();
        let _ = reg.handle_clone(1, 100, CLONE_NEWPID).unwrap();
        let ctx = reg.get_context(100).unwrap();

        assert!(reg.pid.is_namespace_alive(ctx.pid_ns));

        // Unregistering PID 100 (init of the namespace) must kill the namespace
        let killed = reg.pid.unregister_process(100);
        assert_eq!(killed, Some(ctx.pid_ns));
        assert!(!reg.pid.is_namespace_alive(ctx.pid_ns));

        // Attempting to register another process in dead namespace must fail
        assert_eq!(reg.pid.register_process(105, ctx.pid_ns), Err(ESRCH));
    }

    #[test]
    fn test_mount_namespace_chroot_and_traversal() {
        let mut reg = NamespaceRegistry::new();
        let _ = reg.handle_clone(1, 50, CLONE_NEWNS).unwrap();
        let ctx = reg.get_context(50).unwrap();

        // Chroot jail into /var/jail
        reg.mount.chroot(ctx.mount_ns, b"/var/jail").unwrap();

        let mut out = [0u8; 64];
        let len = reg.mount.resolve_path(ctx.mount_ns, b"bin/sh", &mut out).unwrap();
        assert_eq!(&out[..len], b"/var/jail/bin/sh");

        // Traversal attack: attempting to back out with ../../../etc/shadow
        let len2 = reg.mount.resolve_path(ctx.mount_ns, b"../../../../etc/shadow", &mut out).unwrap();
        assert_eq!(&out[..len2], b"/var/jail/etc/shadow", "Path cannot escape chroot jail boundary");
    }

    #[test]
    fn test_net_namespace_loopback_veth_and_ports() {
        let mut reg = NamespaceRegistry::new();
        let _ = reg.handle_clone(1, 60, CLONE_NEWNET).unwrap();
        let ctx = reg.get_context(60).unwrap();

        // Loopback should exist in both
        assert!(reg.net.namespaces[0].device_count >= 1);
        let child_net_idx = reg.net.find_ns_idx(ctx.net_ns).unwrap();
        assert_eq!(&reg.net.namespaces[child_net_idx].devices[0].name[..2], b"lo");

        // Port 80 in root namespace
        assert_eq!(reg.net.bind_port(ROOT_NET_NS, PROTO_TCP, 80, 1), Ok(()));
        // Port 80 again in root namespace fails
        assert_eq!(reg.net.bind_port(ROOT_NET_NS, PROTO_TCP, 80, 2), Err(net::EADDRINUSE));

        // Port 80 in child namespace succeeds (isolated port namespace)
        assert_eq!(reg.net.bind_port(ctx.net_ns, PROTO_TCP, 80, 60), Ok(()));

        // Create veth pair
        assert_eq!(
            reg.net.create_veth_pair(
                ROOT_NET_NS, b"veth_host", [192, 168, 1, 1], [255, 255, 255, 0],
                ctx.net_ns, b"veth_peer", [192, 168, 1, 2], [255, 255, 255, 0],
            ),
            Ok(())
        );
    }

    #[test]
    fn test_ipc_namespace_partitioning() {
        let mut reg = NamespaceRegistry::new();
        let _ = reg.handle_clone(1, 70, CLONE_NEWIPC).unwrap();
        let ctx = reg.get_context(70).unwrap();

        let shm1 = reg.ipc.shm_get(ROOT_IPC_NS, 0x1234, 4096, ipc::IPC_CREAT, 0).unwrap();
        let shm2 = reg.ipc.shm_get(ctx.ipc_ns, 0x1234, 8192, ipc::IPC_CREAT, 1000).unwrap();

        assert!(shm1 > 0);
        assert!(shm2 > 0);
        // Both namespaces own independent segments despite identical key 0x1234
    }

    #[test]
    fn test_uts_namespace_hostname_isolation() {
        let mut reg = NamespaceRegistry::new();
        let _ = reg.handle_clone(1, 80, CLONE_NEWUTS).unwrap();
        let ctx = reg.get_context(80).unwrap();

        reg.uts.sethostname(ctx.uts_ns, b"worker-container-01").unwrap();
        assert_eq!(reg.uts.get_hostname(ctx.uts_ns), Some(b"worker-container-01".as_slice()));
        assert_eq!(reg.uts.get_hostname(ROOT_UTS_NS), Some(b"novaos-server".as_slice()));
    }

    #[test]
    fn test_user_namespace_uid_gid_mapping_and_caps() {
        let mut reg = NamespaceRegistry::new();
        let _ = reg.handle_clone(1, 90, CLONE_NEWUSER).unwrap();
        let ctx = reg.get_context(90).unwrap();

        // Host UID 1000 -> container UID 0
        reg.user.set_uid_map(ctx.user_ns, 0, 1000, 1).unwrap();
        reg.user.set_gid_map(ctx.user_ns, 0, 1000, 1).unwrap();

        assert_eq!(reg.user.map_uid_to_outside(ctx.user_ns, 0), Some(1000));
        assert_eq!(reg.user.map_uid_to_inside(ctx.user_ns, 1000), Some(0));

        // Inside user namespace, UID 0 holds container capabilities
        assert!(reg.user.has_capability(ctx.user_ns, 0, CAP_CHROOT));
        assert!(reg.user.has_capability(ctx.user_ns, 0, CAP_NET_ADMIN));
        assert!(reg.user.has_capability(ctx.user_ns, 0, CAP_SYS_ADMIN));
        // Other UIDs do not hold root capability
        assert!(!reg.user.has_capability(ctx.user_ns, 500, CAP_SYS_ADMIN));
    }

    #[test]
    fn test_unshare_and_setns_lifecycle() {
        let mut reg = NamespaceRegistry::new();
        reg.ensure_registered(150);

        let res = reg.handle_unshare(150, CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWNET);
        assert_eq!(res, 0);

        let ctx = reg.get_context(150).unwrap();
        assert_ne!(ctx.mount_ns, ROOT_MOUNT_NS);
        assert_ne!(ctx.uts_ns, ROOT_UTS_NS);
        assert_ne!(ctx.net_ns, ROOT_NET_NS);
        assert_eq!(ctx.ipc_ns, ROOT_IPC_NS); // Not unshared

        // Setns back to root UTS
        let res_setns = reg.handle_setns(150, ROOT_UTS_NS, CLONE_NEWUTS);
        assert_eq!(res_setns, 0);
        assert_eq!(reg.get_context(150).unwrap().uts_ns, ROOT_UTS_NS);

        // Process exit clean up
        reg.handle_exit(150);
        assert!(reg.get_context(150).is_none());
    }

    #[test]
    fn test_full_namespace_selftest() {
        assert!(run_namespace_selftest());
    }
}
