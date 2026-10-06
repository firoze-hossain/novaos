/*
 * kernel/security/mac.c - Phase 87: where mandatory access control is applied.
 * The decisions are made by kernel/rust/mac.rs; this file decides WHERE to ask
 * and WHAT to do with the answer. See mac.h for the two kinds of check and
 * userland/libc/include/nova_mac_abi.h for the model.
 *
 * Every question put to Rust here is about a process's STACK of profile ids
 * (process_t.mac_stack / mac_n). An empty stack is an unconfined process and
 * costs one load and one compare per syscall; nothing in the Rust engine is
 * touched for it.
 */
#include "mac.h"
#include "../task/process.h"
#include "../arch/x86/mm/paging.h"
#include "../fs/vfs.h"
#include "../include/kernel.h"
#include "../lib/spinlock.h"
#include "../lib/string.h"
#include "../../userland/libc/include/nova_mac_abi.h"

extern int rust_mac_load(const uint8_t* name, uint32_t name_len, const uint8_t* text, uint32_t text_len);
extern void rust_mac_explain(const uint8_t* text, uint32_t text_len, uint32_t* out_kind, uint32_t* out_line);
extern int rust_mac_exec_stack(const uint16_t* parent, uint32_t parent_n, uint32_t new_id, uint16_t* out, uint32_t* out_n);
extern int rust_mac_check_syscall(const uint16_t* stack, uint32_t n, int pid, uint32_t sysno);
extern int rust_mac_check_file(const uint16_t* stack, uint32_t n, int pid, const uint8_t* name, uint32_t name_len, uint32_t op);
extern int rust_mac_check_net(const uint16_t* stack, uint32_t n, int pid, uint32_t op, uint32_t ip, uint32_t port);
extern int rust_mac_check_exec(const uint16_t* stack, uint32_t n, int pid, const uint8_t* name, uint32_t name_len);
extern int rust_mac_is_policy_file(const uint8_t* name, uint32_t name_len);
extern int rust_mac_profile_info(uint32_t id, uint8_t* buf, uint32_t cap, uint32_t* counts);
extern void rust_mac_global(uint32_t* out);
extern void rust_mac_freeze(void);
extern int rust_mac_is_frozen(void);
extern uint32_t rust_mac_selftest(uint32_t* out_total);

typedef char mac_stack_matches_abi[(MAC_MAX_STACK == NOVA_MAC_MAX_STACK) ? 1 : -1];

/* One profile is read at a time, into this buffer. It is one byte larger than
 * the largest profile the engine accepts, so an over-long file shows up as
 * too long instead of being silently truncated into a valid shorter one. */
#define PROFILE_BUF_SIZE 4097
static uint8_t profile_buf[PROFILE_BUF_SIZE];
static spinlock_t profile_lock;

void mac_reset(process_t* p) {
    for (int i = 0; i < MAC_MAX_STACK; i++) {
        p->mac_stack[i] = 0;
    }
    p->mac_n = 0;
    p->mac_denied = 0;
    p->mac_complained = 0;
    p->mac_last_kind = 0;
    p->mac_last_arg = 0;
    p->mac_embedded = false;
}

void mac_init(void) {
    spinlock_init(&profile_lock);
    uint32_t total = 0;
    uint32_t held = rust_mac_selftest(&total);
    kernel_log("[ %s ] Mandatory access control: per-app profiles (NAME.MAC beside "
               "NAME.ELF), deny by default, applied on top of the capability model; "
               "policy engine self-test %d/%d\n",
               (held == total) ? "OK" : "FAIL", (int)held, (int)total);
}

static uint32_t cstr_len(const char* s) {
    uint32_t n = 0;
    while (s[n] != '\0') {
        n++;
    }
    return n;
}

/* Counts and logs a denial (v < 0) or a complaint (v > 0) by profile |v|.
 * Logging is limited per process so that a hostile program cannot flood the
 * serial log by hammering a denied call. */
static void note(process_t* p, int v, uint32_t kind, uint32_t arg, const char* what, const char* detail) {
    char pname[NOVA_MAC_NAME_LEN + 1];
    uint32_t counts[3];
    uint32_t id = (uint32_t)(v < 0 ? -v : v);
    pname[0] = '?';
    pname[1] = '\0';
    (void)rust_mac_profile_info(id, (uint8_t*)pname, sizeof pname, counts);
    uint32_t count;
    if (v < 0) {
        p->mac_denied++;
        p->mac_last_kind = kind;
        p->mac_last_arg = arg;
        count = p->mac_denied;
    } else {
        p->mac_complained++;
        count = p->mac_complained;
    }
    if (count <= 8 || (count & 255u) == 0) {
        kernel_log("[MAC] %s pid %d '%s': %s %s (profile '%s'%s)\n",
                   v < 0 ? "denied" : "would deny", p->pid, p->name, what, detail,
                   pname, v < 0 ? "" : ", complain mode");
    }
}

bool mac_gate_syscall(uint32_t sysno) {
    process_t* p = process_current();
    if (p == NULL || p->mac_n == 0) {
        return false;
    }
    int v = rust_mac_check_syscall(p->mac_stack, p->mac_n, p->pid, sysno);
    if (v == 0) {
        return false;
    }
    char num[12];
    int i = 0, k = 0;
    char tmp[12];
    uint32_t x = sysno;
    do {
        tmp[k++] = (char)('0' + (x % 10));
        x /= 10;
    } while (x != 0 && k < 11);
    while (k > 0) {
        num[i++] = tmp[--k];
    }
    num[i] = '\0';
    note(p, v, NOVA_MAC_KIND_SYSCALL, sysno, "syscall", num);
    return v < 0;
}

bool mac_copy_name(const process_t* p, char* dst, uint32_t cap, const char* src) {
    if (p == NULL || dst == NULL || cap < 2 || src == NULL) {
        return false;
    }
    if (!p->is_user || p->mac_embedded) {
        for (uint32_t i = 0; i + 1 < cap; i++) {
            dst[i] = src[i];
            if (src[i] == '\0') {
                return true;
            }
        }
        dst[0] = '\0';
        return false;
    }
    uint32_t base = (uint32_t)src;
    for (uint32_t i = 0; i + 1 < cap; i++) {
        /* one byte at a time: a name ending a page short of an unmapped one
         * must not fault, and neither must one that starts in the kernel */
        if (!paging_user_range_ok(base + i, 1, false)) {
            dst[0] = '\0';
            return false;
        }
        char c = *(volatile const char*)(base + i);
        dst[i] = c;
        if (c == '\0') {
            return true;
        }
    }
    dst[0] = '\0';
    return false;
}

bool mac_file_allowed(process_t* p, const char* kname, uint32_t op) {
    if (p == NULL) {
        return false;
    }
    if (p->mac_n == 0) {
        return true;
    }
    int v = rust_mac_check_file(p->mac_stack, p->mac_n, p->pid, (const uint8_t*)kname, cstr_len(kname), op);
    if (v == 0) {
        return true;
    }
    note(p, v, NOVA_MAC_KIND_FILE, op, op == MAC_FILE_READ ? "read" : (op == MAC_FILE_WRITE ? "write" : "delete"), kname);
    return v > 0;
}

bool mac_net_allowed(process_t* p, uint32_t op, uint32_t ip, uint16_t port) {
    if (p == NULL) {
        return false;
    }
    if (p->mac_n == 0) {
        return true;
    }
    int v = rust_mac_check_net(p->mac_stack, p->mac_n, p->pid, op, ip, port);
    if (v == 0) {
        return true;
    }
    note(p, v, NOVA_MAC_KIND_NET, port,
         op == MAC_NET_CONNECT ? "connect" : (op == MAC_NET_BIND ? "bind" : "send"),
         "to a peer its profile does not list");
    return v > 0;
}

static bool is_admin(const process_t* p) {
    return p->uid == 0 && p->mac_n == 0;
}

bool mac_policy_write_allowed(process_t* p, const char* kname) {
    if (p == NULL) {
        return false;
    }
    if (!rust_mac_is_policy_file((const uint8_t*)kname, cstr_len(kname))) {
        return true;
    }
    if (is_admin(p) && !rust_mac_is_frozen()) {
        return true;
    }
    p->mac_denied++;
    p->mac_last_kind = NOVA_MAC_KIND_POLICY;
    p->mac_last_arg = 0;
    kernel_log("[MAC] denied pid %d '%s': policy file '%s' may only be changed by an "
               "unconfined root, and never after the policy is frozen\n",
               p->pid, p->name, kname);
    return false;
}

/* The profile NAME for a program path: its base name up to the first '.',
 * at most NAME_LEN characters. Returns the length (0 = no usable name). */
static uint32_t base_of(const char* path, char* out, uint32_t cap) {
    uint32_t n = 0;
    while (path[n] != '\0' && path[n] != '.' && n + 1 < cap) {
        out[n] = path[n];
        n++;
    }
    out[n] = '\0';
    return n;
}

bool mac_prepare_exec(process_t* sp, const char* path_in, char* path_out,
                      uint32_t cap, mac_stack_t* out) {
    out->n = 0;
    for (int i = 0; i < MAC_MAX_STACK; i++) {
        out->stack[i] = 0;
    }

    /* 1. one validated kernel copy of the path, used for EVERYTHING after */
    if (sp != NULL && sp->is_user) {
        if (!mac_copy_name(sp, path_out, cap, path_in)) {
            kernel_log("[MAC] pid %d: refusing to start a program whose name is not "
                       "a valid string\n", sp->pid);
            return false;
        }
    } else {
        if (path_in == NULL) {
            return false;
        }
        uint32_t i = 0;
        while (i + 1 < cap && path_in[i] != '\0') {
            path_out[i] = path_in[i];
            i++;
        }
        path_out[i] = '\0';
    }

    /* 2. the spawner's own profile must let it start this program */
    if (sp != NULL && sp->mac_n != 0) {
        int v = rust_mac_check_exec(sp->mac_stack, sp->mac_n, sp->pid, (const uint8_t*)path_out, cstr_len(path_out));
        if (v != 0) {
            note(sp, v, NOVA_MAC_KIND_EXEC, cstr_len(path_out), "start", path_out);
            if (v < 0) {
                return false;
            }
        }
    }

    /* 3. the program's OWN profile, if it has one */
    uint32_t new_id = 0;
    char base[NOVA_MAC_NAME_LEN + 1];
    uint32_t base_len = base_of(path_out, base, sizeof base);
    if (base_len != 0 && base_len <= 8) {
        char file[16];
        for (uint32_t i = 0; i < base_len; i++) {
            file[i] = base[i];
        }
        file[base_len] = '.';
        file[base_len + 1] = 'M';
        file[base_len + 2] = 'A';
        file[base_len + 3] = 'C';
        file[base_len + 4] = '\0';
        uint32_t flags = spinlock_acquire(&profile_lock);
        int n = vfs_read_file(file, profile_buf, PROFILE_BUF_SIZE);
        if (n >= 0) {
            int id = rust_mac_load((const uint8_t*)base, base_len, profile_buf, (uint32_t)n);
            if (id <= 0) {
                uint32_t kind = 0, line = 0;
                rust_mac_explain(profile_buf, (uint32_t)n, &kind, &line);
                spinlock_release(&profile_lock, flags);
                /* FAIL CLOSED: a profile that is there but cannot be used
                 * means the program does not start. It is never "unconfined
                 * because its policy was unreadable". */
                kernel_log("[MAC] refusing to start '%s': its profile %s cannot be used "
                           "(error %d, parse kind %d at line %d)\n",
                           path_out, file, id, (int)kind, (int)line);
                return false;
            }
            new_id = (uint32_t)id;
        }
        spinlock_release(&profile_lock, flags);
    }

    /* 4. the child's stack: the spawner's, plus this program's profile */
    int rc = rust_mac_exec_stack(sp != NULL ? sp->mac_stack : NULL,
                                 sp != NULL ? sp->mac_n : 0, new_id, out->stack, &out->n);
    if (rc != 0) {
        kernel_log("[MAC] refusing to start '%s': the process would be under more than "
                   "%d profiles\n", path_out, MAC_MAX_STACK);
        return false;
    }
    return true;
}

void mac_apply_stack(process_t* child, const mac_stack_t* st) {
    mac_reset(child);
    for (int i = 0; i < MAC_MAX_STACK; i++) {
        child->mac_stack[i] = st->stack[i];
    }
    child->mac_n = st->n;
}

void mac_inherit_fork(process_t* child, const process_t* parent) {
    mac_reset(child);
    for (int i = 0; i < MAC_MAX_STACK; i++) {
        child->mac_stack[i] = parent->mac_stack[i];
    }
    child->mac_n = parent->mac_n;
}

int mac_sys_info(uint32_t user_ptr) {
    process_t* p = process_current();
    if (p == NULL) {
        return -1;
    }
    if (!paging_user_range_ok(user_ptr, sizeof(nova_mac_info_t), true)) {
        return -14; /* -EFAULT */
    }
    nova_mac_info_t info;
    memset(&info, 0, sizeof info);
    uint32_t g[8];
    rust_mac_global(g);
    info.depth = p->mac_n;
    info.frozen = g[3];
    info.denied = p->mac_denied;
    info.complained = p->mac_complained;
    info.last_kind = p->mac_last_kind;
    info.last_arg = p->mac_last_arg;
    info.loaded = g[4];
    info.total_allowed = g[0];
    info.total_denied = g[1];
    info.total_complained = g[2];
    for (uint32_t i = 0; i < p->mac_n && i < NOVA_MAC_MAX_STACK; i++) {
        uint32_t counts[3] = {0, 0, 0};
        char name[NOVA_MAC_NAME_LEN + 1];
        name[0] = '\0';
        if (rust_mac_profile_info(p->mac_stack[i], (uint8_t*)name, sizeof name, counts) >= 0) {
            for (uint32_t k = 0; k < NOVA_MAC_NAME_LEN; k++) {
                info.names[i][k] = name[k];
            }
            info.prof_allowed[i] = counts[0];
            info.prof_denied[i] = counts[1];
            info.prof_complained[i] = counts[2];
        }
    }
    memcpy((void*)user_ptr, &info, sizeof info);
    return 0;
}

int mac_sys_ctl(uint32_t op, uint32_t arg) {
    (void)arg;
    process_t* p = process_current();
    if (p == NULL) {
        return -1;
    }
    switch (op) {
    case NOVA_MAC_CTL_IS_ADMIN:
        return is_admin(p) ? 1 : 0;
    case NOVA_MAC_CTL_FREEZE:
        if (!is_admin(p)) {
            p->mac_denied++;
            p->mac_last_kind = NOVA_MAC_KIND_POLICY;
            p->mac_last_arg = op;
            kernel_log("[MAC] denied pid %d '%s': only an unconfined root may freeze the "
                       "policy\n", p->pid, p->name);
            return -1;
        }
        rust_mac_freeze();
        kernel_log("[ OK ] MAC policy frozen by pid %d: no profile can be loaded or "
                   "changed until reboot\n", p->pid);
        return 0;
    default:
        return -1;
    }
}
