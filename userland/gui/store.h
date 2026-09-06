#ifndef GUI_STORE_H
#define GUI_STORE_H

/* A GUI front-end for nova-pkg (see kernel/pkg/pkgmgr.h) - the
 * "Software Center" from PROJECT_PLAN.md's original vision, connecting
 * the package manager (Phase 8/10) to the windowing system (Phase 7)
 * for the first time. Lists available packages with an INSTALL/REMOVE
 * button per row; clicking installs or removes it and refreshes the
 * list. ESC returns to the text shell, the same convention the `gui`
 * command's windowing demo already uses. */
void store_run(void);

#endif
