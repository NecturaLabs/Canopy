// wlstub: a minimal Wayland compositor for tools/stall-check.py, which asks one question: does a
// client keep answering xdg_wm_base pings while the compositor withholds its frame callbacks, as
// Hyprland and other compositors do for a window on a hidden workspace?
//
//   SIGUSR1  hide: commits are accepted and buffers released, frame callbacks are held back
//   SIGUSR2  show: held frame callbacks are sent, and every 16 ms from then on
//   SIGHUP   ask every toplevel to resize, which makes a client repaint
//
// It pings every second like Hyprland (which shows "Application Not Responding" after three
// missed pings by default) and prints READY, HIDDEN, SHOWN, RESIZE, MISSED, ANR, PONG and
// STATUS lines on stdout. Only wl_compositor, wl_shm and xdg_wm_base are served; request handlers
// that need no behaviour are one shared no-op, called with more arguments than it declares,
// which is harmless on every ABI this builds for.
//
// Build: see tools/stall-check.py.
#define _GNU_SOURCE
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <wayland-server.h>

#include "xdg-shell-server-protocol.h"

static struct wl_display *display;
static struct wl_event_loop *loop;
static bool visible = true;
static uint32_t commits, frames_sent, serial_counter = 1;

static uint64_t now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static void noop(void) {}

struct frame {
    struct wl_resource *cb;
    struct wl_list link;
};

struct surface {
    struct wl_resource *res, *buffer, *current, *xdg_surface, *toplevel;
    struct wl_list pending, queued, link;
    bool configured;
};

static struct wl_list surfaces;

static void frame_destroyed(struct wl_resource *r) {
    struct frame *f = wl_resource_get_user_data(r);
    wl_list_remove(&f->link);
    free(f);
}

static void surface_destroy_req(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }

static void surface_attach(struct wl_client *c, struct wl_resource *r, struct wl_resource *buffer, int32_t x, int32_t y) {
    struct surface *s = wl_resource_get_user_data(r);
    s->buffer = buffer;
}

static void surface_frame(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct surface *s = wl_resource_get_user_data(r);
    struct frame *f = calloc(1, sizeof *f);
    f->cb = wl_resource_create(c, &wl_callback_interface, 1, id);
    wl_resource_set_implementation(f->cb, NULL, f, frame_destroyed);
    wl_list_insert(s->pending.prev, &f->link);
}

static void surface_commit(struct wl_client *c, struct wl_resource *r) {
    struct surface *s = wl_resource_get_user_data(r);
    if (s->toplevel && !s->configured) {
        struct wl_array states;
        wl_array_init(&states);
        xdg_toplevel_send_configure(s->toplevel, 0, 0, &states);
        xdg_surface_send_configure(s->xdg_surface, serial_counter++);
        wl_array_release(&states);
        s->configured = true;
    }
    if (s->buffer) {
        if (s->current && s->current != s->buffer) wl_buffer_send_release(s->current);
        s->current = s->buffer;
        s->buffer = NULL;
        commits++;
    }
    // Frame callbacks requested before this commit belong to it.
    struct frame *f, *tmp;
    wl_list_for_each_safe(f, tmp, &s->pending, link) {
        wl_list_remove(&f->link);
        wl_list_insert(s->queued.prev, &f->link);
    }
}

static const struct wl_surface_interface surface_impl = {
    .destroy = surface_destroy_req,
    .attach = surface_attach,
    .damage = (void *)noop,
    .frame = surface_frame,
    .set_opaque_region = (void *)noop,
    .set_input_region = (void *)noop,
    .commit = surface_commit,
    .set_buffer_transform = (void *)noop,
    .set_buffer_scale = (void *)noop,
    .damage_buffer = (void *)noop,
};

static void surface_gone(struct wl_resource *r) {
    struct surface *s = wl_resource_get_user_data(r);
    // Each callback's destructor unlinks and frees its node.
    struct frame *f, *tmp;
    wl_list_for_each_safe(f, tmp, &s->pending, link) wl_resource_destroy(f->cb);
    wl_list_for_each_safe(f, tmp, &s->queued, link) wl_resource_destroy(f->cb);
    wl_list_remove(&s->link);
    free(s);
}

static void compositor_create_surface(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct surface *s = calloc(1, sizeof *s);
    wl_list_init(&s->pending);
    wl_list_init(&s->queued);
    s->res = wl_resource_create(c, &wl_surface_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(s->res, &surface_impl, s, surface_gone);
    wl_list_insert(&surfaces, &s->link);
}

static void region_destroy(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }
static const struct wl_region_interface region_impl = {
    .destroy = region_destroy, .add = (void *)noop, .subtract = (void *)noop,
};

static void compositor_create_region(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct wl_resource *region = wl_resource_create(c, &wl_region_interface, 1, id);
    wl_resource_set_implementation(region, &region_impl, NULL, NULL);
}

static const struct wl_compositor_interface compositor_impl = {
    .create_surface = compositor_create_surface, .create_region = compositor_create_region,
};

static void bind_compositor(struct wl_client *c, void *d, uint32_t version, uint32_t id) {
    struct wl_resource *r = wl_resource_create(c, &wl_compositor_interface, version, id);
    wl_resource_set_implementation(r, &compositor_impl, NULL, NULL);
}

// xdg-shell
static void xdg_destroy_req(struct wl_client *c, struct wl_resource *r) { wl_resource_destroy(r); }

static const struct xdg_toplevel_interface toplevel_impl = {
    .destroy = xdg_destroy_req,
    .set_parent = (void *)noop, .set_title = (void *)noop, .set_app_id = (void *)noop,
    .show_window_menu = (void *)noop, .move = (void *)noop, .resize = (void *)noop,
    .set_max_size = (void *)noop, .set_min_size = (void *)noop, .set_maximized = (void *)noop,
    .unset_maximized = (void *)noop, .set_fullscreen = (void *)noop,
    .unset_fullscreen = (void *)noop, .set_minimized = (void *)noop,
};

static void xs_get_toplevel(struct wl_client *c, struct wl_resource *r, uint32_t id) {
    struct surface *s = wl_resource_get_user_data(r);
    s->toplevel = wl_resource_create(c, &xdg_toplevel_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(s->toplevel, &toplevel_impl, s, NULL);
}

static const struct xdg_surface_interface xdg_surface_impl = {
    .destroy = xdg_destroy_req, .get_toplevel = xs_get_toplevel, .get_popup = (void *)noop,
    .set_window_geometry = (void *)noop, .ack_configure = (void *)noop,
};

static struct wl_resource *wm_bases[8];
static uint32_t ping_serial;
static bool ping_outstanding;
static uint64_t ping_sent_at;
static int missed;

static void wm_get_xdg_surface(struct wl_client *c, struct wl_resource *r, uint32_t id, struct wl_resource *surface) {
    struct surface *s = wl_resource_get_user_data(surface);
    s->xdg_surface = wl_resource_create(c, &xdg_surface_interface, wl_resource_get_version(r), id);
    wl_resource_set_implementation(s->xdg_surface, &xdg_surface_impl, s, NULL);
}

static void wm_pong(struct wl_client *c, struct wl_resource *r, uint32_t serial) {
    if (!ping_outstanding || serial != ping_serial) return;
    uint64_t latency = now_ms() - ping_sent_at;
    if (latency > 500 || missed > 0)
        printf("PONG latency_ms=%llu missed=%d\n", (unsigned long long)latency, missed);
    ping_outstanding = false;
    missed = 0;
    fflush(stdout);
}

static const struct xdg_wm_base_interface wm_impl = {
    .destroy = xdg_destroy_req, .create_positioner = (void *)noop,
    .get_xdg_surface = wm_get_xdg_surface, .pong = wm_pong,
};

static void wm_gone(struct wl_resource *r) {
    for (int i = 0; i < 8; i++)
        if (wm_bases[i] == r) wm_bases[i] = NULL;
}

static void bind_wm_base(struct wl_client *c, void *d, uint32_t version, uint32_t id) {
    struct wl_resource *r = wl_resource_create(c, &xdg_wm_base_interface, version, id);
    wl_resource_set_implementation(r, &wm_impl, NULL, wm_gone);
    for (int i = 0; i < 8; i++)
        if (!wm_bases[i]) { wm_bases[i] = r; break; }
}

// timers and signals
static struct wl_event_source *frame_timer, *ping_timer, *status_timer;

static int on_frame_tick(void *d) {
    if (visible) {
        struct surface *s;
        wl_list_for_each(s, &surfaces, link) {
            struct frame *f, *tmp;
            wl_list_for_each_safe(f, tmp, &s->queued, link) {
                wl_callback_send_done(f->cb, (uint32_t)now_ms());
                frames_sent++;
                wl_resource_destroy(f->cb);
            }
        }
    }
    wl_event_source_timer_update(frame_timer, 16);
    return 0;
}

static int on_ping_tick(void *d) {
    if (ping_outstanding) {
        missed++;
        printf("MISSED %d (unanswered for %llu ms)\n", missed, (unsigned long long)(now_ms() - ping_sent_at));
        if (missed == 3) printf("ANR: the client stopped answering pings\n");
    } else {
        ping_serial = serial_counter++;
        ping_sent_at = now_ms();
        ping_outstanding = true;
        for (int i = 0; i < 8; i++)
            if (wm_bases[i]) xdg_wm_base_send_ping(wm_bases[i], ping_serial);
        // nobody to ping yet: nothing outstanding
        bool any = false;
        for (int i = 0; i < 8; i++) any |= wm_bases[i] != NULL;
        if (!any) ping_outstanding = false;
    }
    fflush(stdout);
    wl_event_source_timer_update(ping_timer, 1000);
    return 0;
}

static int on_status_tick(void *d) {
    printf("STATUS visible=%d commits=%u frames_sent=%u missed=%d\n", visible, commits, frames_sent, missed);
    fflush(stdout);
    wl_event_source_timer_update(status_timer, 2000);
    return 0;
}

static int on_sigusr(int sig, void *d) {
    visible = sig == SIGUSR2;
    printf("%s\n", visible ? "SHOWN" : "HIDDEN");
    fflush(stdout);
    return 0;
}

// SIGHUP: ask every toplevel to resize, which makes a client repaint (as a real compositor does
// on a scale or geometry change).
static int on_sighup(int sig, void *d) {
    static int width = 1000;
    width = width == 1000 ? 1001 : 1000;
    struct surface *s;
    wl_list_for_each(s, &surfaces, link) {
        if (!s->toplevel || !s->configured) continue;
        struct wl_array states;
        wl_array_init(&states);
        xdg_toplevel_send_configure(s->toplevel, width, 700, &states);
        xdg_surface_send_configure(s->xdg_surface, serial_counter++);
        wl_array_release(&states);
    }
    printf("RESIZE %d\n", width);
    fflush(stdout);
    return 0;
}

static int on_term(int sig, void *d) {
    wl_display_terminate(display);
    return 0;
}

int main(int argc, char **argv) {
    const char *socket = argc > 1 ? argv[1] : "wl-stall-test";
    display = wl_display_create();
    loop = wl_display_get_event_loop(display);
    wl_list_init(&surfaces);
    wl_display_init_shm(display);
    wl_global_create(display, &wl_compositor_interface, 4, NULL, bind_compositor);
    wl_global_create(display, &xdg_wm_base_interface, 2, NULL, bind_wm_base);
    if (wl_display_add_socket(display, socket)) {
        fprintf(stderr, "cannot add socket %s\n", socket);
        return 1;
    }
    sigset_t mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGUSR1);
    sigaddset(&mask, SIGUSR2);
    sigaddset(&mask, SIGTERM);
    sigaddset(&mask, SIGINT);
    sigaddset(&mask, SIGHUP);
    sigprocmask(SIG_BLOCK, &mask, NULL);
    wl_event_loop_add_signal(loop, SIGHUP, on_sighup, NULL);
    wl_event_loop_add_signal(loop, SIGUSR1, on_sigusr, NULL);
    wl_event_loop_add_signal(loop, SIGUSR2, on_sigusr, NULL);
    wl_event_loop_add_signal(loop, SIGTERM, on_term, NULL);
    wl_event_loop_add_signal(loop, SIGINT, on_term, NULL);
    frame_timer = wl_event_loop_add_timer(loop, on_frame_tick, NULL);
    ping_timer = wl_event_loop_add_timer(loop, on_ping_tick, NULL);
    status_timer = wl_event_loop_add_timer(loop, on_status_tick, NULL);
    wl_event_source_timer_update(frame_timer, 16);
    wl_event_source_timer_update(ping_timer, 1000);
    wl_event_source_timer_update(status_timer, 2000);
    printf("READY %s\n", socket);
    fflush(stdout);
    wl_display_run(display);
    wl_display_destroy(display);
    return 0;
}
