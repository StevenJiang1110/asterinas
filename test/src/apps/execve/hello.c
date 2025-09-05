// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <unistd.h> // For getpid()
#include <sys/prctl.h>
#include <string.h>

#define THREAD_NAME_LEN 256

int main()
{
	printf("Hello from memfd! My PID is %d\n", getpid());

	// 2. 使用 prctl 获取当前线程的名称
	char current_name[THREAD_NAME_LEN];

	// 初始化缓冲区，这是一个好习惯
	memset(current_name, 0, THREAD_NAME_LEN);

	if (prctl(PR_GET_NAME, (unsigned long)current_name, 0, 0, 0) == -1) {
		perror("prctl(PR_GET_NAME) failed");
		return 0;
	}

	printf("Thread: prctl successfully got my name: '%s'\n", current_name);

	return 0;
}