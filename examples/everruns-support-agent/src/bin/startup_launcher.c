/* Prints a CLOCK_MONOTONIC timestamp and execs the benchmark binary, so the
 * exec and dynamic-linking cost before main() is attributable.
 *   cc -O2 -o startup_launcher startup_launcher.c
 *   ./startup_launcher ./target/release/startup_bench
 */
#include <stdio.h>
#include <time.h>
#include <unistd.h>

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s <binary> [args...]\n", argv[0]);
    return 2;
  }
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  printf("exec_monotonic       %.6f\n", ts.tv_sec + ts.tv_nsec / 1e9);
  fflush(stdout);
  execv(argv[1], &argv[1]);
  perror("execv");
  return 127;
}
