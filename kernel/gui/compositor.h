#ifndef GUI_COMPOSITOR_H
#define GUI_COMPOSITOR_H

#include "../include/types.h"
#include "../drivers/mouse/ps2mouse.h"

#define COMPOSITOR_MAX_WINDOWS 3

/* A tiny window manager: fixed-count draggable colored rectangle
 * windows with a titlebar (labeled with a single digit - see
 * font5x7.h for why not full text), rendered into an off-screen
 * back buffer and blitted to VGA Mode 13h in one shot per frame to
 * avoid visible tearing/flicker. Click-and-drag by the titlebar is
 * the only interaction; there's no resize, minimize, close, focus
 * order beyond "last dragged," or overlap-aware redraw - see
 * PROGRESS.md for the honest scope of "windowing system" here. */
void compositor_init(void);

/* Applies one mouse_state_t reading (see ps2mouse.h) - moves the
 * cursor and, if the left button is down over a titlebar, drags that
 * window. */
void compositor_handle_mouse(mouse_state_t state);

/* Renders one frame into the back buffer and blits it to 0xA0000. */
void compositor_render(void);

#endif
