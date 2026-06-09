// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <arpa/inet.h>
#include <fcntl.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define PAYLOAD "abcdef"
#define PAYLOAD_LEN 6
#define TCP_POLL_TIMEOUT_MS 1000
#define TCP_SETTLE_USEC 100000
#define SHORT_LEN 3

static int tcp_listener;
static struct sockaddr_in tcp_addr = { .sin_family = AF_INET };
static socklen_t tcp_addr_len = sizeof(tcp_addr);
static struct pollfd pfd = { .events = POLLIN };

#define TCP_CONNECT()                                   \
	do {                                            \
		refresh_connection(&send_fd, &recv_fd); \
		pfd.fd = recv_fd;                       \
	} while (0)

#define TCP_CLOSE()                        \
	do {                               \
		TEST_SUCC(close(send_fd)); \
		TEST_SUCC(close(recv_fd)); \
	} while (0)

#define TCP_SEND(offset, len)                                 \
	TEST_RES(send(send_fd, PAYLOAD + (offset), (len), 0), \
		 _ret == (ssize_t)(len))

#define TCP_WAIT_READABLE()                          \
	TEST_RES(poll(&pfd, 1, TCP_POLL_TIMEOUT_MS), \
		 _ret == 1 && (pfd.revents & POLLIN))

#define TCP_PEEK(offset, len)                                                  \
	do {                                                                   \
		memset(buf, 0, sizeof(buf));                                   \
		msg_flags = 0;                                                 \
		TEST_RES(peek_message(recv_fd, buf, (len), &msg_flags),        \
			 _ret == (ssize_t)(len) &&                             \
				 (msg_flags & MSG_TRUNC) == 0 &&               \
				 memcmp(buf, PAYLOAD + (offset), (len)) == 0); \
	} while (0)

#define TCP_RECV(offset, len)                                                  \
	do {                                                                   \
		memset(buf, 0, sizeof(buf));                                   \
		TEST_RES(recv(recv_fd, buf, (len), 0),                         \
			 _ret == (ssize_t)(len) &&                             \
				 memcmp(buf, PAYLOAD + (offset), (len)) == 0); \
	} while (0)

#define TCP_WAIT_APPENDED_READABLE()                        \
	do {                                                \
		/*                                        \
		 * `poll` cannot wait for the suffix here \
		 * because the peeked prefix keeps the    \
		 * receive side readable.                 \
		 */ \
		TEST_SUCC(usleep(TCP_SETTLE_USEC));         \
	} while (0)

static ssize_t peek_message(int fd, char *buf, size_t len, int *msg_flags)
{
	struct iovec iov = { .iov_base = buf, .iov_len = len };
	struct msghdr msg = { .msg_iov = &iov, .msg_iovlen = 1 };
	ssize_t ret = recvmsg(fd, &msg, MSG_PEEK);

	if (ret >= 0)
		*msg_flags = msg.msg_flags;
	return ret;
}

FN_SETUP(create_tcp_listener)
{
	tcp_listener = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	tcp_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

	CHECK(bind(tcp_listener, (struct sockaddr *)&tcp_addr, tcp_addr_len));
	CHECK(getsockname(tcp_listener, (struct sockaddr *)&tcp_addr,
			  &tcp_addr_len));
	CHECK(listen(tcp_listener, 1));
}
END_SETUP()

static void refresh_connection(int *send_fd, int *recv_fd)
{
	int status_flags;
	int connected_fd = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	int accepted_fd;

	CHECK(connect(connected_fd, (struct sockaddr *)&tcp_addr,
		      tcp_addr_len));
	accepted_fd = CHECK(accept(tcp_listener, NULL, NULL));
	status_flags = CHECK(fcntl(accepted_fd, F_GETFL, 0));
	CHECK(fcntl(accepted_fd, F_SETFL, status_flags | O_NONBLOCK));

	*send_fd = connected_fd;
	*recv_fd = accepted_fd;
}

#define STREAMLIKE_PREFIX tcp_
#define STREAMLIKE_FDS() \
	int send_fd;     \
	int recv_fd
#define STREAMLIKE_CONNECT() TCP_CONNECT()
#define STREAMLIKE_SEND(offset, len) TCP_SEND(offset, len)
#define STREAMLIKE_WAIT_READABLE() TCP_WAIT_READABLE()
#define STREAMLIKE_WAIT_APPENDED_READABLE() TCP_WAIT_APPENDED_READABLE()
#define STREAMLIKE_PEEK(offset, len) TCP_PEEK(offset, len)
#define STREAMLIKE_RECV(offset, len) TCP_RECV(offset, len)
#define STREAMLIKE_CLOSE() TCP_CLOSE()
#include "msg_peek_streamlike.h"
#undef STREAMLIKE_PREFIX
#undef STREAMLIKE_FDS
#undef STREAMLIKE_CONNECT
#undef STREAMLIKE_SEND
#undef STREAMLIKE_WAIT_READABLE
#undef STREAMLIKE_WAIT_APPENDED_READABLE
#undef STREAMLIKE_PEEK
#undef STREAMLIKE_RECV
#undef STREAMLIKE_CLOSE

FN_SETUP(close_tcp_listener)
{
	CHECK(close(tcp_listener));
}
END_SETUP()

FN_TEST(udp_msg_peek)
{
	int send_fd;
	int recv_fd;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	send_fd = TEST_SUCC(socket(AF_INET, SOCK_DGRAM, 0));
	recv_fd = TEST_SUCC(socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(recv_fd, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(recv_fd, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);

	// Peeking a datagram must not consume the datagram.
	TEST_RES(peek_message(recv_fd, buf, PAYLOAD_LEN, &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()
