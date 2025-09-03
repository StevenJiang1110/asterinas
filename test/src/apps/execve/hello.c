// SPDX-License-Identifier: MPL-2.0

// hello.c
#include <stdio.h>
#include <unistd.h> // For getpid()

int main() {
    printf("Hello from memfd! My PID is %d\n", getpid());
    return 0;
}
