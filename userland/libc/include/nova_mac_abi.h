#ifndef NOVA_MAC_ABI_H
#define NOVA_MAC_ABI_H

/*
 * nova_mac_abi.h - Phase 87: the ABI of NovaOS's mandatory access control
 * (SYS_MAC_INFO, SYS_MAC_CTL), shared verbatim by the kernel
 * (kernel/security/mac.c) and userland (novasys.h, mactest, macjail).
 * Dependency-free: plain `unsigned int` / `char` (checked below).
 *
 * WHAT IT IS
 *
 * The capability model (allowed_files[], allowed_hosts[], can_spawn) lets
 * whoever launches a program decide what it may open and where it may connect,
 * and says nothing at all about the other ~60 syscalls. MAC is the second,
 * stricter layer: a profile - NAME.MAC, a text file next to NAME.ELF - lists
 * everything the program may do, and the kernel applies it when the program
 * starts. The program cannot change it, and it applies to root too. Anything
 * not listed is denied. An operation must be allowed by BOTH the capability
 * model and the profile.
 *
 * A profile (one directive per line, '#' starts a comment):
 *
 *     mode enforce|complain            complain = log what would be denied, allow it
 *     allow syscall <name>... | *      write open read ... (see kernel/rust/mac.rs); exit and
 *                                      mac_info are always allowed
 *     allow file <rwd> <pattern>...    r = open for reading, w = write/create, d = delete;
 *                                      * matches any run, ? one character, case-insensitive
 *     allow net <cbs> <ip|*>:<port|lo-hi|*>...   c = connect, b = bind, s = send
 *     allow exec <pattern>...          programs it may start
 *
 * HOW CONFINEMENT PASSES ON. fork() copies it. Starting a program adds THAT
 * program's profile (if it has one) to the parent's, so a confined program can
 * never shed confinement by starting a more permissive one: the child is bound
 * by every profile on its stack (at most 4). Nothing removes an entry.
 *
 * FAILS CLOSED. A profile that does not parse means the program is not
 * started. A denied syscall returns -1.
 *
 * POLICY FILES. Only an unconfined root may write or delete a *.MAC file, and
 * not even that once SYS_MAC_CTL(FREEZE) has been called: that is one-way until
 * reboot, and afterwards no new or changed profile can be loaded.
 */

#define NOVA_SYS_MAC_INFO 71 /* EBX = nova_mac_info_t* (out) */
#define NOVA_SYS_MAC_CTL  72 /* EBX = op, ECX = arg; returns 0, 1 or -1 */

#define NOVA_MAC_CTL_FREEZE   1u /* admin only: lock the policy until reboot */
#define NOVA_MAC_CTL_IS_ADMIN 2u /* returns 1 if the caller is an unconfined root, else 0 */

#define NOVA_MAC_MAX_STACK 4
#define NOVA_MAC_NAME_LEN  16

/* what the kernel last denied THIS process */
#define NOVA_MAC_KIND_NONE    0u
#define NOVA_MAC_KIND_SYSCALL 1u
#define NOVA_MAC_KIND_FILE    2u
#define NOVA_MAC_KIND_NET     3u
#define NOVA_MAC_KIND_EXEC    4u
#define NOVA_MAC_KIND_POLICY  5u /* a write to a *.MAC file by someone who may not */

typedef struct {
    unsigned int depth;            /* profiles on this process's stack; 0 = unconfined */
    unsigned int frozen;           /* the policy is frozen */
    unsigned int denied;           /* operations denied to THIS process */
    unsigned int complained;       /* operations a complain-mode profile would have denied it */
    unsigned int last_kind;        /* NOVA_MAC_KIND_*: the last thing denied to this process */
    unsigned int last_arg;         /* the syscall number, file op, net op or port, per kind */
    unsigned int loaded;           /* profiles loaded system-wide */
    unsigned int total_allowed;    /* system-wide counters, over all confined processes */
    unsigned int total_denied;
    unsigned int total_complained;
    char names[NOVA_MAC_MAX_STACK][NOVA_MAC_NAME_LEN];
    unsigned int prof_allowed[NOVA_MAC_MAX_STACK];
    unsigned int prof_denied[NOVA_MAC_MAX_STACK];
    unsigned int prof_complained[NOVA_MAC_MAX_STACK];
} nova_mac_info_t;

typedef char nova_mac_abi_check[(sizeof(nova_mac_info_t) == 40 + 64 + 48) ? 1 : -1];

#endif
