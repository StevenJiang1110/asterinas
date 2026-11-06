// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <linux/in_route.h>
#include <linux/rtnetlink.h>
#include <net/if.h>
#include <stdint.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#include "netlink_sockoption.h"

#define BUFFER_SIZE 16384

struct route_request {
	struct nlmsghdr hdr;
	struct rtmsg rtm;
	char attrs[128];
};

static char buffer[BUFFER_SIZE] __attribute__((aligned(4)));
static int fd;
static uint32_t loopback;
static struct sockaddr_nl kernel = { .nl_family = AF_NETLINK };
static unsigned int loopback_index;
static const struct in6_addr ipv6_loopback = IN6ADDR_LOOPBACK_INIT;

FN_SETUP(init)
{
	fd = CHECK(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	// Bound receive waits so a missing response fails instead of hanging.
	struct timeval timeout = { .tv_sec = 1 };
	CHECK(setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			 sizeof(timeout)));
	loopback = htonl(INADDR_LOOPBACK);
	loopback_index = CHECK_WITH(if_nametoindex("lo"), _ret != 0);
}
END_SETUP()

static struct route_request new_request(int family, int flags)
{
	return (struct route_request){
		.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct rtmsg)),
			 .nlmsg_type = RTM_GETROUTE,
			 .nlmsg_flags = NLM_F_REQUEST | flags,
			 .nlmsg_seq = 123 },
		.rtm = { .rtm_family = family },
	};
}

static void add_attr(struct route_request *request, int type, const void *value,
		     size_t size)
{
	struct rtattr *attr =
		(void *)request + NLMSG_ALIGN(request->hdr.nlmsg_len);
	attr->rta_type = type;
	attr->rta_len = RTA_LENGTH(size);
	memcpy(RTA_DATA(attr), value, size);
	request->hdr.nlmsg_len =
		NLMSG_ALIGN(request->hdr.nlmsg_len) + RTA_ALIGN(attr->rta_len);
}

#define ADD_ATTR(request, type, value)                            \
	do {                                                      \
		__auto_type _value = (value);                     \
		add_attr(request, type, &_value, sizeof(_value)); \
	} while (0)

#define TEST_SEND_REQUEST(request)                                   \
	TEST_RES(sendto(fd, &(request), (request).hdr.nlmsg_len, 0,  \
			(struct sockaddr *)&kernel, sizeof(kernel)), \
		 _ret == (request).hdr.nlmsg_len)

static int valid_message(struct nlmsghdr *header, ssize_t size, int type,
			 size_t body_size)
{
	return size >= (ssize_t)NLMSG_LENGTH(body_size) &&
	       NLMSG_OK(header, size) &&
	       header->nlmsg_len >= NLMSG_LENGTH(body_size) &&
	       header->nlmsg_type == type && header->nlmsg_seq == 123;
}

#define TEST_RECV_ERROR(type, expected)                                       \
	do {                                                                  \
		struct nlmsghdr *_hdr = (void *)buffer;                       \
		struct nlmsgerr *_error = NLMSG_DATA(_hdr);                   \
		size_t _size = (type) == NLMSG_DONE ? sizeof(int) :           \
						      sizeof(*_error);        \
		TEST_RES(recv(fd, buffer, sizeof(buffer), 0),                 \
			 valid_message(_hdr, _ret, type, _size) &&            \
				 _error->error == -(expected) &&              \
				 ((type) == NLMSG_DONE ?                      \
					  (_hdr->nlmsg_flags & NLM_F_MULTI) : \
					  (!(_hdr->nlmsg_flags &              \
					     NLM_F_MULTI) &&                  \
					   _error->msg.nlmsg_type ==          \
						   RTM_GETROUTE &&            \
					   _error->msg.nlmsg_seq == 123)));   \
	} while (0)

/*
 * FIXME: Linux reports dump validation errors in DONE, but Asterinas uses
 * the common ERROR reply path. Use DONE on both once that path supports it.
 */
#ifdef __asterinas__
#define DUMP_ERROR_TYPE NLMSG_ERROR
#else
#define DUMP_ERROR_TYPE NLMSG_DONE
#endif

#define TEST_REQUEST_ERROR(request, expected)                           \
	do {                                                            \
		TEST_SEND_REQUEST(request);                             \
		TEST_RECV_ERROR((request).hdr.nlmsg_flags &NLM_F_DUMP ? \
					DUMP_ERROR_TYPE :               \
					NLMSG_ERROR,                    \
				expected);                              \
	} while (0)

static const void *find_attr(struct nlmsghdr *header, int type, size_t size)
{
	struct rtmsg *rtm = NLMSG_DATA(header);
	int remaining = RTM_PAYLOAD(header);
	for (struct rtattr *attr = RTM_RTA(rtm); RTA_OK(attr, remaining);
	     attr = RTA_NEXT(attr, remaining)) {
		if (attr->rta_type == type && RTA_PAYLOAD(attr) == size)
			return RTA_DATA(attr);
	}
	return NULL;
}

static int attr_eq_u32(struct nlmsghdr *header, int type, uint32_t expected)
{
	const uint32_t *value = find_attr(header, type, sizeof(*value));
	return value && *value == expected;
}

static const void *loopback_addr(int family)
{
	return family == AF_INET ? (const void *)&loopback :
				   (const void *)&ipv6_loopback;
}

static int attr_eq_loopback(struct nlmsghdr *header, int type, int family)
{
	size_t size = family == AF_INET ? 4 : 16;
	const void *addr = find_attr(header, type, size);
	return addr && !memcmp(addr, loopback_addr(family), size);
}

static int matches_loopback(struct nlmsghdr *header, int family)
{
	struct rtmsg *rtm = NLMSG_DATA(header);
	return rtm->rtm_family == family && rtm->rtm_type == RTN_LOCAL &&
	       rtm->rtm_dst_len == (family == AF_INET ? 32 : 128) &&
	       attr_eq_u32(header, RTA_OIF, loopback_index) &&
	       attr_eq_loopback(header, RTA_DST, family);
}

struct dump_cursor {
	struct nlmsghdr *header;
	ssize_t remaining;
};

// Advance across both messages in one datagram and datagrams in one dump.
static struct nlmsghdr *next_dump_message(struct dump_cursor *cursor)
{
	if (cursor->header)
		cursor->header = NLMSG_NEXT(cursor->header, cursor->remaining);
	if (!cursor->remaining) {
		cursor->remaining = recv(fd, buffer, sizeof(buffer), 0);
		cursor->header = (void *)buffer;
		if (cursor->remaining < 0)
			return NULL;
	}
	struct nlmsghdr *header = cursor->header;
	if (!NLMSG_OK(header, cursor->remaining) ||
	    !valid_message(header, cursor->remaining, header->nlmsg_type,
			   header->nlmsg_type == NLMSG_DONE ?
				   sizeof(int) :
				   sizeof(struct rtmsg)) ||
	    !(header->nlmsg_flags & NLM_F_MULTI) ||
	    (header->nlmsg_type != RTM_NEWROUTE &&
	     header->nlmsg_type != NLMSG_DONE)) {
		errno = EPROTO;
		return NULL;
	}
	return header;
}

// cond checks every route through _hdr and _rtm; DONE must follow a loopback route.
#define TEST_DUMP(request, cond)                                               \
	do {                                                                   \
		TEST_SEND_REQUEST(request);                                    \
		struct dump_cursor _cursor = { 0 };                            \
		int _found_loopback = 0;                                       \
		struct nlmsghdr *_hdr;                                         \
		do {                                                           \
			_hdr = TEST_RES(                                       \
				next_dump_message(&_cursor),                   \
				_ret && (_ret->nlmsg_type != NLMSG_DONE ||     \
					 (*(int *)NLMSG_DATA(_ret) == 0 &&     \
					  _found_loopback)));                  \
			if (!_hdr || _hdr->nlmsg_type == NLMSG_DONE)           \
				break;                                         \
			struct rtmsg *_rtm = NLMSG_DATA(_hdr);                 \
			TEST_RES(find_attr(_hdr, RTA_TABLE, sizeof(uint32_t)), \
				 _ret && (cond));                              \
			_found_loopback |= matches_loopback(_hdr, AF_INET) ||  \
					   matches_loopback(_hdr, AF_INET6);   \
		} while (1);                                                   \
	} while (0)

FN_TEST(route_dumps)
{
	struct route_request request;
	int families[] = { AF_INET, AF_INET6, AF_UNSPEC };
	for (int strict = 0; strict <= 1; strict++) {
		TEST_SETSOCKOPT_INT(fd, NETLINK_GET_STRICT_CHK, strict);
		for (size_t i = 0; i < sizeof(families) / sizeof(families[0]);
		     i++) {
			request = new_request(families[i], NLM_F_DUMP);
			TEST_DUMP(request,
				  families[i] == AF_UNSPEC ||
					  _rtm->rtm_family == families[i]);
		}
	}

	// RTA_TABLE overrides the header table; all strict filters apply together.
	request = new_request(AF_INET, NLM_F_DUMP);
	request.rtm.rtm_table = RT_TABLE_MAIN;
	request.rtm.rtm_protocol = RTPROT_KERNEL;
	request.rtm.rtm_type = RTN_LOCAL;
	ADD_ATTR(&request, RTA_TABLE, (uint32_t)RT_TABLE_LOCAL);
	ADD_ATTR(&request, RTA_OIF, loopback_index);
	TEST_DUMP(request,
		  _rtm->rtm_family == AF_INET &&
			  _rtm->rtm_protocol == RTPROT_KERNEL &&
			  _rtm->rtm_type == RTN_LOCAL &&
			  (_hdr->nlmsg_flags & NLM_F_DUMP_FILTERED) &&
			  attr_eq_u32(_hdr, RTA_TABLE, RT_TABLE_LOCAL) &&
			  attr_eq_u32(_hdr, RTA_OIF, loopback_index));

	// Legacy dumps ignore table and output interface selectors.
	TEST_SETSOCKOPT_INT(fd, NETLINK_GET_STRICT_CHK, 0);
	request = new_request(AF_INET, NLM_F_DUMP);
	request.rtm.rtm_table = 123;
	ADD_ATTR(&request, RTA_OIF, UINT32_MAX);
	TEST_DUMP(request, _rtm->rtm_family == AF_INET);

	// Old iproute2 versions use a one-byte rtgenmsg body.
	request = new_request(AF_INET, NLM_F_DUMP);
	request.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct rtgenmsg));
	TEST_DUMP(request, _rtm->rtm_family == AF_INET);
}
END_TEST()

FN_TEST(single_dump_flags)
{
	struct route_request request;
	int flags[] = { NLM_F_ROOT, NLM_F_MATCH };
	for (int strict = 0; strict <= 1; strict++) {
		TEST_SETSOCKOPT_INT(fd, NETLINK_GET_STRICT_CHK, strict);
		for (size_t i = 0; i < sizeof(flags) / sizeof(flags[0]); i++) {
			request = new_request(AF_INET, flags[i]);
			if (!strict) {
				// Legacy dumps ignore malformed attributes; queries reject them.
				ADD_ATTR(&request, RTA_OIF, (uint8_t)0);
			}
			TEST_DUMP(request, _rtm->rtm_family == AF_INET);
		}
	}
}
END_TEST()

FN_TEST(strict_route_validation)
{
	TEST_SETSOCKOPT_INT(fd, NETLINK_GET_STRICT_CHK, 1);
	struct route_request request;

	request = new_request(AF_INET, NLM_F_DUMP);
	request.rtm.rtm_dst_len = 32;
	TEST_REQUEST_ERROR(request, EINVAL);

	request.rtm.rtm_dst_len = 0;
	request.rtm.rtm_scope = RT_SCOPE_HOST;
	TEST_REQUEST_ERROR(request, EINVAL);

	request.rtm.rtm_scope = 0;
	request.rtm.rtm_flags = RTM_F_FIB_MATCH;
	TEST_REQUEST_ERROR(request, EINVAL);

	request.rtm.rtm_flags = 0;
	ADD_ATTR(&request, RTA_DST, loopback);
	TEST_REQUEST_ERROR(request, EINVAL);

	request = new_request(AF_INET, NLM_F_DUMP);
	ADD_ATTR(&request, RTA_OIF, UINT32_MAX);
	TEST_REQUEST_ERROR(request, ENODEV);

	request = new_request(AF_INET, NLM_F_DUMP);
	request.rtm.rtm_table = 123;
	TEST_REQUEST_ERROR(request, ENOENT);

	// A rejected request must not corrupt the next request on the same socket.
	request = new_request(AF_INET, NLM_F_DUMP);
	TEST_DUMP(request, _rtm->rtm_family == AF_INET);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
}
END_SETUP()
