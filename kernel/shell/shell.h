#ifndef SHELL_H
#define SHELL_H

/* Runs forever, reading lines from the keyboard driver and dispatching
 * built-in commands. This is intentionally minimal - a real line editor,
 * command history, and the Phase 3/4 filesystem-backed commands (ls,
 * cat) are tracked separately in PROGRESS.md. Its purpose right now is
 * to exercise the full Phase 2 input pipeline (IDT -> IRQ1 -> keyboard
 * buffer -> shell) end to end. */
void shell_run(void);

#endif
