; crt0.asm - process entry point for NovaOS userland C programs
;
; NovaOS's process_exec() (kernel/task/process.c) leaves the initial
; stack in the real x86 process-entry convention - the same raw
; layout Linux's own execve() leaves - starting from ESP:
;   [esp]            = argc
;   [esp+4]          = argv[0]
;   [esp+8]          = argv[1]
;   ...
;   [esp+4+4*argc]   = NULL              (argv terminator)
;   [esp+4+4*argc+4] = envp[0]           (Phase 73: the kernel now
;                                          places the environment its
;                                          parent passed via SYS_EXEC_ENV
;                                          here - see below)
;   ...              envp[m-1], then a NULL terminator (with no
;                                          environment at all, that NULL
;                                          is simply the first entry)
;   ... argv/envp string data further up ...
;
; This file's only job is translating that raw layout into a proper
; cdecl call to a C `int main(int argc, char** argv, char** envp)`,
; first publishing envp as the global `environ` (defined in stdlib.c)
; so getenv()/setenv() see the same environment main() does, then
; calling exit() with its return value - exactly what a real C
; runtime's _start always does, just against NovaOS's own stack
; convention instead of Linux's.

BITS 32

SECTION .text
global _start
extern main
extern exit
extern environ

_start:
    xor ebp, ebp            ; mark the outermost stack frame (debugger
                             ; convention, matches real crt0 files)

    mov eax, [esp]           ; argc
    lea ecx, [esp+4]         ; argv = &[esp+4]

    ; envp = argv + 4*(argc+1) - skip argc's worth of argv pointers
    ; plus the one NULL terminator right after them.
    mov edx, eax
    inc edx
    lea edx, [ecx + edx*4]   ; envp

    mov [environ], edx        ; Phase 73: publish the environment before
                              ; main() (or any constructor-style code) can
                              ; call getenv()

    push edx                  ; argument 3: envp
    push ecx                  ; argument 2: argv
    push eax                  ; argument 1: argc
    call main
    add esp, 12                ; caller cleans up (cdecl)

    push eax                    ; main()'s return value
    call exit                    ; never returns

.hang:                            ; unreachable - exit() always calls
    jmp .hang                     ; SYS_EXIT, which never returns
