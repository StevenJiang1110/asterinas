#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <errno.h>
#include <wait.h> // For waitpid

#define BUFFER_SIZE 256

int main()
{
	int sv[2]; // socketpair 将创建的两个套接字
	pid_t pid;
	char buffer[BUFFER_SIZE];
	ssize_t bytes_read;
	int status;

	// 1. 创建 UNIX stream socketpair
	// AF_UNIX: 本地通信，而不是网络通信
	// SOCK_STREAM: 流式套接字，提供可靠的、顺序的、全双工的连接
	// 0: 协议类型，对于 SOCK_STREAM 而言通常为 0
	if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) == -1) {
		perror("socketpair failed");
		return 1;
	}

	printf("Socketpair created: sv[0]=%d, sv[1]=%d\n", sv[0], sv[1]);

	// 2. Fork 创建子进程
	pid = fork();

	if (pid == -1) {
		perror("fork failed");
		close(sv[0]);
		close(sv[1]);
		return 1;
	}

	if (pid == 0) {
		// 子进程逻辑
		printf("Child process (PID: %d) started.\n", getpid());

		// 子进程关闭它不用的那一端
		close(sv[0]);

		// 模拟一些工作，或者让父进程先尝试读取
		printf("Child: Sleeping for 1 second before shutdown...\n");
		sleep(1);

		// 关闭子进程的写端。这会向父进程的读端发送一个 EOF 信号。
		printf("Child: Shutting down write half of sv[1] (fd: %d)...\n",
		       sv[1]);
		if (shutdown(sv[1], SHUT_WR) == -1) {
			perror("Child: shutdown SHUT_WR failed");
			close(sv[1]);
			exit(1);
		}
		printf("Child: SHUT_WR completed.\n");

		// 再次延迟，让父进程有机会处理 SHUT_WR
		sleep(1);

		// 关闭子进程的读端。这通常不影响父进程的读行为，因为它已经收到 SHUT_WR 的 EOF。
		// 但如果父进程尝试写入，这个 SHUT_RD 可能会影响子进程是否还能接收。
		printf("Child: Shutting down read half of sv[1] (fd: %d)...\n",
		       sv[1]);
		if (shutdown(sv[1], SHUT_RD) == -1) {
			perror("Child: shutdown SHUT_RD failed");
			close(sv[1]);
			exit(1);
		}
		printf("Child: SHUT_RD completed.\n");

		// 关闭子进程的套接字
		close(sv[1]);
		printf("Child: Closed sv[1] (fd: %d).\n", sv[1]);

		printf("Child process exiting.\n");
		exit(0);

	} else {
		// 父进程逻辑
		printf("Parent process (PID: %d, Child PID: %d) started.\n",
		       getpid(), pid);

		// 父进程关闭它不用的那一端
		close(sv[1]);

		printf("Parent: Reading from sv[0] (fd: %d)...\n", sv[0]);

		while (1) {
			memset(buffer, 0, BUFFER_SIZE);
			bytes_read = read(
				sv[0], buffer,
				BUFFER_SIZE - 1); // 留一个字节给 null 终止符

			if (bytes_read == -1) {
				// 读取错误
				if (errno == EINTR) { // 被信号中断
					printf("Parent: read interrupted by signal, retrying.\n");
					continue;
				}
				perror("Parent: read failed");
				break;
			} else if (bytes_read == 0) {
				// 收到 EOF
				printf("Parent: read returned 0 bytes (EOF detected).\n");
				printf("Parent: This indicates the other end (child) has closed its write half or entire socket.\n");
				break; // 退出循环
			} else {
				// 成功读取到数据 (在这个测试中不应该发生，除非子进程在 shutdown 前写入)
				buffer[bytes_read] = '\0';
				printf("Parent: Read %zd bytes: '%s'\n",
				       bytes_read, buffer);
			}
		}

		// 关闭父进程的套接字
		close(sv[0]);
		printf("Parent: Closed sv[0] (fd: %d).\n", sv[0]);

		// 等待子进程结束
		if (waitpid(pid, &status, 0) == -1) {
			perror("Parent: waitpid failed");
		} else {
			if (WIFEXITED(status)) {
				printf("Parent: Child exited with status %d.\n",
				       WEXITSTATUS(status));
			} else if (WIFSIGNALED(status)) {
				printf("Parent: Child terminated by signal %d.\n",
				       WTERMSIG(status));
			}
		}

		printf("Parent process exiting.\n");
	}

	return 0;
}
