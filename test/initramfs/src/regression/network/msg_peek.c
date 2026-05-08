// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <arpa/inet.h>
#include <fcntl.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define PAYLOAD "abcdef"
#define PAYLOAD_LEN 6
#define TCP_SETTLE_USEC 100000
#define SHORT_LEN 3

#define SEND_AND_SETTLE(fd, buf, len)                                          \
	do {                                                                   \
		TEST_RES(send((fd), (buf), (len), 0), _ret == (ssize_t)(len)); \
		usleep(TCP_SETTLE_USEC);                                       \
	} while (0)

static ssize_t recvmsg_with_flags(int fd, int flags, char *buf, size_t len,
				  int *msg_flags)
{
	struct iovec iov = { .iov_base = buf, .iov_len = len };
	struct msghdr msg = { .msg_iov = &iov, .msg_iovlen = 1 };
	ssize_t ret = CHECK(recvmsg(fd, &msg, flags));

	*msg_flags = msg.msg_flags;
	return ret;
}

FN_TEST(tcp_msg_peek)
{
	int listener;
	int send_fd;
	int recv_fd;
	int status_flags;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	listener = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);
	send_fd = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(listener, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(listener, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(listen(listener, 1));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));
	recv_fd = TEST_RES(accept(listener, NULL, NULL), _ret >= 0);
	status_flags = TEST_RES(fcntl(recv_fd, F_GETFL, 0), _ret >= 0);
	TEST_SUCC(fcntl(recv_fd, F_SETFL, status_flags | O_NONBLOCK));
	TEST_SUCC(close(listener));

	// A short peek leaves the stream untouched for later reads.
	SEND_AND_SETTLE(send_fd, PAYLOAD, PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, SHORT_LEN, 0),
		 _ret == SHORT_LEN && memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, PAYLOAD_LEN - SHORT_LEN, 0),
		 _ret == PAYLOAD_LEN - SHORT_LEN &&
			 memcmp(buf, PAYLOAD + SHORT_LEN,
				PAYLOAD_LEN - SHORT_LEN) == 0);

	// A later full read still observes the whole message after a short peek.
	SEND_AND_SETTLE(send_fd, PAYLOAD, PAYLOAD_LEN);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	// New bytes appended after peeking stay behind the original prefix.
	SEND_AND_SETTLE(send_fd, PAYLOAD, SHORT_LEN);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	SEND_AND_SETTLE(send_fd, PAYLOAD + SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, SHORT_LEN, 0),
		 _ret == SHORT_LEN && memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, PAYLOAD_LEN - SHORT_LEN, 0),
		 _ret == PAYLOAD_LEN - SHORT_LEN &&
			 memcmp(buf, PAYLOAD + SHORT_LEN,
				PAYLOAD_LEN - SHORT_LEN) == 0);

	// A larger read after new data arrives returns both the peeked prefix and the suffix.
	SEND_AND_SETTLE(send_fd, PAYLOAD, SHORT_LEN);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	SEND_AND_SETTLE(send_fd, PAYLOAD + SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	// A full peek leaves the stream available for multiple later reads.
	SEND_AND_SETTLE(send_fd, PAYLOAD, PAYLOAD_LEN);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, PAYLOAD_LEN,
				    &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, SHORT_LEN, 0),
		 _ret == SHORT_LEN && memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, PAYLOAD_LEN - SHORT_LEN, 0),
		 _ret == PAYLOAD_LEN - SHORT_LEN &&
			 memcmp(buf, PAYLOAD + SHORT_LEN,
				PAYLOAD_LEN - SHORT_LEN) == 0);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()
