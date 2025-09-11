#include <stdio.h>   // For printf, perror
#include <stdlib.h>  // For exit
#include <fcntl.h>   // For open
#include <unistd.h>  // For read, close
#include <string.h>  // For memset

#define DEVICE_PATH "/dev/full"
#define READ_SIZE 100

int main() {
    int fd;
    char buffer[READ_SIZE];
    ssize_t bytes_read;

    // 1. 打开 /dev/full 设备文件
    // O_RDONLY: 以只读方式打开
    fd = open(DEVICE_PATH, O_RDONLY);
    if (fd == -1) {
        perror("Failed to open /dev/full");
        exit(EXIT_FAILURE);
    }

    printf("Successfully opened %s (fd: %d)\n", DEVICE_PATH, fd);

    // 清空缓冲区，确保读取前是干净的
    memset(buffer, 0, sizeof(buffer));

    // 2. 从 /dev/full 读取指定数量的字节
    printf("Attempting to read %d bytes from %s...\n", READ_SIZE, DEVICE_PATH);
    bytes_read = read(fd, buffer, READ_SIZE);

    if (bytes_read == -1) {
        perror("Failed to read from /dev/full");
        close(fd);
        exit(EXIT_FAILURE);
    } else if (bytes_read == 0) {
        printf("Read 0 bytes. This is expected as /dev/full immediately returns EOF on read.\n");
    } else {
        // 实际上，这部分代码通常不会被执行，因为 /dev/full 读取会返回 0 字节
        printf("Read %zd bytes. Content: '%.*s'\n", bytes_read, (int)bytes_read, buffer);
    }

    // 3. 关闭文件描述符
    if (close(fd) == -1) {
        perror("Failed to close /dev/full");
        exit(EXIT_FAILURE);
    }

    printf("Successfully closed %s.\n", DEVICE_PATH);

    return EXIT_SUCCESS;
}

