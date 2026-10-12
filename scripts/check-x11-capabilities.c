/*
 * Exit status contract for the AppImage launcher:
 *   0  the current X11 display accepts XInput 2.0
 *  10  XInput 2.0 is absent or unsupported (use the nested X11 fallback)
 *  20  the display could not be queried (do not guess; report an error)
 *
 * X2Go/nxagent may advertise the XInput extension but respond to
 * XIQueryVersion with BadRequest.  Xlib's normal error handler terminates the
 * process for that asynchronous error, so temporarily replace it and force a
 * round trip before restoring the caller's handler.
 */
#include <X11/Xlib.h>
#include <X11/extensions/XInput2.h>

#include <stdio.h>

enum {
    XI2_UNSUPPORTED = 10,
    PROBE_ERROR = 20,
};

static int x_error_code;

static int capture_x_error(Display *display, XErrorEvent *event) {
    (void)display;
    x_error_code = event->error_code;
    return 0;
}

int main(void) {
    Display *display;
    int extension_opcode;
    int first_event;
    int first_error;
    int major = 2;
    int minor = 0;
    int status;
    XErrorHandler previous_handler;

    display = XOpenDisplay(NULL);
    if (display == NULL) {
        fprintf(stderr, "cannot connect to the current X11 display\n");
        return PROBE_ERROR;
    }

    if (!XQueryExtension(display, "XInputExtension", &extension_opcode,
                         &first_event, &first_error)) {
        XCloseDisplay(display);
        return XI2_UNSUPPORTED;
    }

    /* Flush any earlier asynchronous errors before handling the XI2 request. */
    XSync(display, False);
    x_error_code = 0;
    previous_handler = XSetErrorHandler(capture_x_error);
    status = XIQueryVersion(display, &major, &minor);
    XSync(display, False);
    XSetErrorHandler(previous_handler);
    XCloseDisplay(display);

    if (x_error_code == BadRequest || status == BadRequest) {
        return XI2_UNSUPPORTED;
    }
    if (x_error_code != 0 || status != Success) {
        fprintf(stderr, "XInput2 version query failed (status %d, X11 error %d)\n",
                status, x_error_code);
        return PROBE_ERROR;
    }
    if (major < 2) {
        return XI2_UNSUPPORTED;
    }
    return 0;
}
