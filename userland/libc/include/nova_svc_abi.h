#ifndef NOVA_SVC_ABI_H
#define NOVA_SVC_ABI_H

/*
 * nova_svc_abi.h - Month 4 (Server & admin UI): Service manager supervision status
 * (SYS_SERVICE_STATUS), shared by kernel and userland (novasys.h, novainit, settings-rs).
 *
 * Exposes live service supervision state from novainit (PID 1) to userland apps,
 * enabling the Settings app and administrators to monitor running, crashed, restarting,
 * or stopped services, and dispatch service control actions (restart, stop, start).
 */

#define NOVA_SYS_SERVICE_STATUS 82

/* Operations for SYS_SERVICE_STATUS (passed in EBX) */
#define NOVA_SVC_OP_QUERY       1u  /* ECX = nova_service_status_t* (out) */
#define NOVA_SVC_OP_PUBLISH     2u  /* ECX = const nova_service_status_t* (in, PID 1 or root) */
#define NOVA_SVC_OP_CONTROL     3u  /* ECX = action code, EDX = const char* service_name */
#define NOVA_SVC_OP_GET_ACTION  4u  /* ECX = nova_service_action_t* (out, retrieves pending action) */

/* Service Supervision States */
#define NOVA_SVC_STATE_STOPPED    0u  /* Exited cleanly or not running */
#define NOVA_SVC_STATE_RUNNING    1u  /* Actively executing (valid pid > 0) */
#define NOVA_SVC_STATE_RESTARTING 2u  /* Exited, eligible and scheduled for restart */
#define NOVA_SVC_STATE_CRASHED    3u  /* Max restarts exceeded (gave up) or unexpected crash */
#define NOVA_SVC_STATE_BLOCKED    4u  /* Waiting for dependency to start */

/* Restart Policies */
#define NOVA_SVC_POLICY_NEVER    0u
#define NOVA_SVC_POLICY_ON_CRASH 1u
#define NOVA_SVC_POLICY_ALWAYS   2u

/* Control Actions */
#define NOVA_SVC_ACTION_NONE     0u
#define NOVA_SVC_ACTION_RESTART  1u
#define NOVA_SVC_ACTION_STOP     2u
#define NOVA_SVC_ACTION_START    3u

#define NOVA_SVC_MAX_SERVICES    16
#define NOVA_SVC_MAX_NAME        24
#define NOVA_SVC_MAX_PATH        16
#define NOVA_SVC_MAX_ARG         32

/* Single supervised service status record (136 bytes) */
typedef struct {
    char         name[NOVA_SVC_MAX_NAME];         /* Service identifier name */
    char         exec_path[NOVA_SVC_MAX_PATH];    /* Executable binary path */
    char         arg[NOVA_SVC_MAX_ARG];           /* Startup argument */
    char         depends_on[NOVA_SVC_MAX_NAME];   /* Dependency service name */
    int          pid;                             /* Active PID (-1 if inactive) */
    unsigned int state;                           /* NOVA_SVC_STATE_* */
    unsigned int restart_policy;                  /* NOVA_SVC_POLICY_* */
    unsigned int restart_count;                   /* Consecutive restarts */
    unsigned int uptime_ticks;                    /* Ticks since start / runtime */
    unsigned int trusted;                         /* 1 if trusted (SYS_EXEC_TRUSTED), 0 otherwise */
    int          last_exit_code;                  /* Most recent exit code */
    unsigned int reserved[3];                     /* Reserved for future expansion */
} nova_service_info_t;

/* Complete supervisor status snapshot (2220 bytes) */
typedef struct {
    unsigned int        count;                    /* Number of supervised services */
    unsigned int        supervisor_pid;           /* novainit PID (normally 1) */
    unsigned int        supervisor_uptime_ticks;  /* Timer ticks since supervisor start */
    unsigned int        generation;               /* Increments on every supervision pass */
    unsigned int        pending_action;           /* Current pending control action */
    char                pending_name[NOVA_SVC_MAX_NAME]; /* Target service for pending action */
    nova_service_info_t services[NOVA_SVC_MAX_SERVICES];
} nova_service_status_t;

/* Pending action retrieval record (28 bytes) */
typedef struct {
    unsigned int action;                          /* NOVA_SVC_ACTION_* */
    char         name[NOVA_SVC_MAX_NAME];         /* Target service name */
} nova_service_action_t;

#endif /* NOVA_SVC_ABI_H */
