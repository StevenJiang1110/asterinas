#define _GNU_SOURCE // For memfd_secret and execveat

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <errno.h>

// 假设编译好的 "Hello, World!" 程序路径
#define EXECUTABLE_PATH "./hello"
// memfd 的名称，仅用于调试或procfs可见性
#define MFD_NAME "my_hello_exe"

int main() {
    int hello_fd = -1;
    int memfd = -1;
    char buffer[4096];
    ssize_t bytes_read;
    ssize_t bytes_written;
    struct stat st;

    printf("--- memfd_secret + execveat Test ---\n");

    // 1. 打开要执行的源文件 (hello)
    hello_fd = open(EXECUTABLE_PATH, O_RDONLY);
    if (hello_fd < 0) {
        perror("Error opening source executable " EXECUTABLE_PATH);
        return 1;
    }
    printf("Opened source executable '%s'.\n", EXECUTABLE_PATH);

    // 获取源文件大小，用于设置memfd大小
    if (fstat(hello_fd, &st) < 0) {
        perror("Error getting source executable stats");
        close(hello_fd);
        return 1;
    }
    off_t hello_size = st.st_size;
    printf("Source executable size: %ld bytes.\n", hello_size);

    // 2. 创建一个 memfd_secret 文件描述符
    // MFD_CLOEXEC: close-on-exec，防止子进程继承
    // MFD_ALLOW_SEALING: 允许使用 fcntl(F_ADD_SEALS) 来锁定memfd
    // MFD_EXEC: 标记为可执行文件 (关键!)
    // MFD_SECRET: (Linux 5.14+) 创建一个不可被其他进程映射或访问的匿名文件
    //             如果系统不支持 memfd_secret，会退化为 memfd_create
    memfd = memfd_create(MFD_NAME, MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (memfd < 0) {
        if (errno == ENOSYS) {
            printf("memfd_secret not supported, trying memfd_create without MFD_SECRET.\n");
            memfd = memfd_create(MFD_NAME, MFD_CLOEXEC | MFD_ALLOW_SEALING);
        }
        if (memfd < 0) {
            perror("Error creating memfd");
            close(hello_fd);
            return 1;
        }
    }
    printf("Created memfd_secret (FD: %d) with name '%s'.\n", memfd, MFD_NAME);

    // 3. 设置 memfd 的大小
    if (ftruncate(memfd, hello_size) < 0) {
        perror("Error setting memfd size");
        close(hello_fd);
        close(memfd);
        return 1;
    }
    printf("Set memfd size to %ld bytes.\n", hello_size);

    // 4. 将源文件内容拷贝到 memfd
    off_t current_offset = 0;
    while ((bytes_read = read(hello_fd, buffer, sizeof(buffer))) > 0) {
        bytes_written = write(memfd, buffer, bytes_read);
        if (bytes_written != bytes_read) {
            perror("Error writing to memfd");
            close(hello_fd);
            close(memfd);
            return 1;
        }
        current_offset += bytes_written;
    }
    if (bytes_read < 0) {
        perror("Error reading from source executable");
        close(hello_fd);
        close(memfd);
        return 1;
    }
    printf("Successfully copied %ld bytes from '%s' to memfd.\n", current_offset, EXECUTABLE_PATH);

    // 关闭源文件，memfd 现在包含了可执行文件
    close(hello_fd);
    hello_fd = -1; // 标记为已关闭

    // 5. (可选) 对 memfd 进行密封，防止进一步修改
    // 这使得 memfd 不可写入，不可缩小，不可增加大小等
    if (fcntl(memfd, F_ADD_SEALS, F_SEAL_WRITE | F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL) < 0) {
        perror("Warning: Could not seal memfd (this is not critical for execution)");
    } else {
        printf("Memfd sealed successfully.\n");
    }

    printf("Attempting to execute memfd content via execveat...\n");

    // 6. 使用 execveat 执行 memfd 文件
    // fd: memfd 文件描述符
    // pathname: AT_EMPTY_PATH (需要 Linux 5.3+)，表示fd是文件本身，不是目录
    // argv: 参数数组
    // envp: 环境变量数组
    // flags: AT_EMPTY_PATH (结合 AT_FDCWD，表示执行 fd)
    char *const argv[] = { "memfd_hello", NULL }; // execveat 第一个参数是程序名
    char *const envp[] = { "PATH=/bin:/usr/bin", NULL }; // 示例环境变量

    // execveat 成功则不会返回
    execveat(memfd, "", argv, envp, AT_EMPTY_PATH);

    // 如果 execveat 返回，说明执行失败
    perror("Error execveat");

    // 清理（如果 execveat 失败）
    if (memfd != -1) {
        close(memfd);
    }

    return 1; // 失败退出
}
