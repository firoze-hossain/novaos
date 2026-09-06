#ifndef SHELL_FIRSTRUN_H
#define SHELL_FIRSTRUN_H

/* NovaOS boots from a live CD/USB image (see PROJECT_PLAN.md) rather
 * than being installed to a hard disk in the traditional sense - a
 * real installer would mean writing a bootloader to the disk's boot
 * sector, which is a substantial separate undertaking tracked as
 * future work (see PROGRESS.md). What IS implemented, and is the
 * realistic "installer" for a live-boot design like this one (the
 * same pattern many live-CD Linux distributions use for
 * "persistence"): a first-run setup wizard that asks for a hostname
 * and username once, then saves them to the attached disk so every
 * later boot recognizes the machine and greets the user by name
 * instead of re-asking.
 *
 * Called once from kernel_main(), after the boot banner and before
 * any tasks are created - deliberately not part of the shell's
 * command loop, so the shell task never needs "is this the first run"
 * logic of its own. */
void firstrun_check_and_run(void);

/* Whatever was loaded (returning user) or entered (first run) -
 * "novaos"/"user" if no disk is attached to persist anything on. Used
 * by the shell prompt and the whoami/hostname commands. */
const char* firstrun_get_hostname(void);
const char* firstrun_get_username(void);

#endif
