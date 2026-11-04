/*
 * LD_PRELOAD shim for the stress bench: adds a fixed delay to every fsync and
 * fdatasync PostgreSQL makes, so a local SSD behaves like network storage
 * where each commit's WAL flush costs a round trip (RDS, EBS). The delay is
 * FSYNC_DELAY_US microseconds; unset or 0 passes calls straight through.
 *
 * Build: cc -shared -fPIC -O2 -o fsync_delay.so fsync_delay.c -ldl
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <time.h>

static long delay_us = -1;

static void pause_for_flush(void) {
    if (delay_us < 0) {
        const char *v = getenv("FSYNC_DELAY_US");
        delay_us = v ? atol(v) : 0;
    }
    if (delay_us > 0) {
        struct timespec ts = {delay_us / 1000000, (delay_us % 1000000) * 1000};
        nanosleep(&ts, NULL);
    }
}

int fsync(int fd) {
    static int (*real)(int);
    if (!real) real = (int (*)(int))dlsym(RTLD_NEXT, "fsync");
    pause_for_flush();
    return real(fd);
}

int fdatasync(int fd) {
    static int (*real)(int);
    if (!real) real = (int (*)(int))dlsym(RTLD_NEXT, "fdatasync");
    pause_for_flush();
    return real(fd);
}
