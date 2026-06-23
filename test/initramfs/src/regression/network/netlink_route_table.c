// SPDX-License-Identifier: MPL-2.0

#include <net/if.h>
#include <netinet/in.h>
#include <stdint.h>
#include <errno.h>
#include <linux/rtnetlink.h>
#include <stddef.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define ETHER_NAME "eth0"
#define LOOPBACK_NAME "lo"

#ifndef RTM_F_FIB_MATCH
#define RTM_F_FIB_MATCH 0x2000
#endif

#ifndef RTA_NH_ID
#define RTA_NH_ID 30
#endif

struct route_spec {
	uint32_t dst;
	uint8_t dst_len;
	uint32_t gateway;
	uint32_t oif;
	uint32_t table;
	uint32_t priority;
	uint32_t flags;
	uint8_t protocol;
	uint8_t scope;
	uint8_t type;
};

struct route_request {
	struct nlmsghdr hdr;
	struct rtmsg rtmsg;
	char attrs[256];
};

static uint32_t ipv4_addr(uint8_t a, uint8_t b, uint8_t c, uint8_t d);
static uint32_t iface_index_by_name(const char *name);
static uint32_t iface_ipv4_addr_by_name(const char *name);
static void add_rtattr(struct nlmsghdr *nlh, size_t max_len, uint16_t type,
		       const void *data, size_t data_len);
static void add_rtattr_u32(struct nlmsghdr *nlh, size_t max_len, uint16_t type,
			   uint32_t value);
static void init_route_request(struct route_request *req, uint16_t type,
			       uint16_t flags, uint32_t seq);
static void init_route_request_from_spec(struct route_request *req,
					 uint16_t type, uint16_t flags,
					 uint32_t seq,
					 const struct route_spec *spec);
static void init_route_dump_request_from_spec(struct route_request *req,
					      uint32_t seq,
					      const struct route_spec *spec);
static int route_request_success(int sock_fd, const struct route_request *req,
				 const struct route_spec *spec);
static int route_request_absent(int sock_fd, const struct route_request *req,
				const struct route_spec *spec);
static int route_request_empty_dump(int sock_fd,
				    const struct route_request *req);
static int route_request_ack_errno(int sock_fd, const struct route_request *req,
				   int *ack_errno);
static int route_request_error(int sock_fd, const struct route_request *req,
			       int expected_errno);
static int route_request_cleanup(int sock_fd, struct route_request *req,
				 uint16_t type, uint32_t seq,
				 const struct route_spec *spec);
static int route_lookup_table_info(int sock_fd, const struct route_request *req,
				   uint32_t *header_table, int *attr_present,
				   uint32_t *attr_table);
static int route_lookup_prefsrc(int sock_fd, const struct route_request *req,
				uint32_t *prefsrc);

FN_TEST(get_route_dump_bootstrap)
{
	int sock_fd;
	uint32_t lo_index = iface_index_by_name(LOOPBACK_NAME);
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	struct route_request req;
	struct route_spec loopback_connected = {
		.dst = ipv4_addr(127, 0, 0, 0),
		.dst_len = 8,
		.gateway = 0,
		.oif = lo_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_UNICAST,
	};
	struct route_spec loopback_local = {
		.dst = ipv4_addr(127, 0, 0, 0),
		.dst_len = 8,
		.gateway = 0,
		.oif = lo_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_HOST,
		.type = RTN_LOCAL,
	};
	struct route_spec eth0_connected = {
		.dst = ipv4_addr(10, 0, 2, 0),
		.dst_len = 24,
		.gateway = 0,
		.oif = eth0_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_UNICAST,
	};
	struct route_spec default_route = {
		.dst = 0,
		.dst_len = 0,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_BOOT,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec eth0_broadcast = {
		.dst = ipv4_addr(10, 0, 2, 255),
		.dst_len = 32,
		.gateway = 0,
		.oif = eth0_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_BROADCAST,
	};
	struct route_spec limited_broadcast = {
		.dst = ipv4_addr(255, 255, 255, 255),
		.dst_len = 32,
		.gateway = 0,
		.oif = eth0_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_BROADCAST,
	};

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 10);
	req.rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req.rtmsg.rtm_type = RTN_UNSPEC;
	req.rtmsg.rtm_scope = RT_SCOPE_UNIVERSE;
	TEST_RES(route_request_success(sock_fd, &req, &loopback_connected),
		 _ret == 0);
	TEST_RES(route_request_success(sock_fd, &req, &loopback_local),
		 _ret == 0);
	if (eth0_index != 0) {
		TEST_RES(route_request_success(sock_fd, &req, &eth0_connected),
			 _ret == 0);
		TEST_RES(route_request_success(sock_fd, &req, &default_route),
			 _ret == 0);
		TEST_RES(route_request_success(sock_fd, &req,
					       &limited_broadcast),
			 _ret == 0);
		TEST_RES(route_request_success(sock_fd, &req, &eth0_broadcast),
			 _ret == 0);
	}

	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 11);
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct rtgenmsg));
	((struct rtgenmsg *)NLMSG_DATA(&req.hdr))->rtgen_family = AF_UNSPEC;
	TEST_RES(route_request_success(sock_fd, &req, &loopback_connected),
		 _ret == 0);
	TEST_RES(route_request_success(sock_fd, &req, &loopback_local),
		 _ret == 0);

	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 12);
	req.rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req.rtmsg.rtm_type = RTN_UNSPEC;
	req.rtmsg.rtm_scope = RT_SCOPE_UNIVERSE;
	req.rtmsg.rtm_flags = RTM_F_CLONED;
	TEST_RES(route_request_empty_dump(sock_fd, &req), _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(new_get_replace_delete_route)
{
	int sock_fd;
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	uint32_t lo_index = iface_index_by_name(LOOPBACK_NAME);
	struct route_request req;
	struct route_spec route = {
		.dst = ipv4_addr(192, 0, 2, 0),
		.dst_len = 24,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_STATIC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec duplicate_attr_route = route;
	struct route_spec oif_replacement_route = route;
	struct route_spec oif_replacement = route;
	struct route_spec replacement = route;
	struct route_spec delete_selector = route;
	uint32_t wrong_gateway = ipv4_addr(10, 0, 2, 3);

	SKIP_TEST_IF(eth0_index == 0 || lo_index == 0);

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 30,
				     &route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_dump_request_from_spec(&req, 31, &route);
	TEST_RES(route_request_success(sock_fd, &req, &route), _ret == 0);

	replacement.gateway = ipv4_addr(10, 0, 2, 3);
	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_REPLACE | NLM_F_ACK, 32,
				     &replacement);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_dump_request_from_spec(&req, 33, &replacement);
	TEST_RES(route_request_success(sock_fd, &req, &replacement), _ret == 0);
	TEST_RES(route_request_absent(sock_fd, &req, &route), _ret == 0);

	delete_selector.gateway = 0;
	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 34,
				     &delete_selector);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_dump_request_from_spec(&req, 35, &replacement);
	TEST_RES(route_request_absent(sock_fd, &req, &replacement), _ret == 0);

	oif_replacement_route.dst = ipv4_addr(192, 0, 4, 0);
	oif_replacement_route.priority = 25;
	oif_replacement = oif_replacement_route;
	oif_replacement.gateway = ipv4_addr(127, 0, 0, 2);
	oif_replacement.oif = lo_index;
	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 36,
				     &oif_replacement_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_REPLACE | NLM_F_ACK, 37,
				     &oif_replacement);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	init_route_dump_request_from_spec(&req, 38, &oif_replacement);
	TEST_RES(route_request_success(sock_fd, &req, &oif_replacement),
		 _ret == 0);
	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_REPLACE | NLM_F_EXCL | NLM_F_ACK, 39,
				     &oif_replacement_route);
	TEST_RES(route_request_error(sock_fd, &req, EEXIST), _ret == 0);
	init_route_dump_request_from_spec(&req, 40, &oif_replacement_route);
	TEST_RES(route_request_absent(sock_fd, &req, &oif_replacement_route),
		 _ret == 0);
	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 41,
				     &oif_replacement);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	init_route_dump_request_from_spec(&req, 42, &oif_replacement);
	TEST_RES(route_request_absent(sock_fd, &req, &oif_replacement),
		 _ret == 0);

	duplicate_attr_route.dst = ipv4_addr(192, 0, 3, 0);
	duplicate_attr_route.priority = 10;
	init_route_request(&req, RTM_NEWROUTE,
			   NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 56);
	req.rtmsg.rtm_dst_len = duplicate_attr_route.dst_len;
	req.rtmsg.rtm_table = duplicate_attr_route.table;
	req.rtmsg.rtm_protocol = duplicate_attr_route.protocol;
	req.rtmsg.rtm_scope = duplicate_attr_route.scope;
	req.rtmsg.rtm_type = duplicate_attr_route.type;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &duplicate_attr_route.dst,
		   sizeof(duplicate_attr_route.dst));
	add_rtattr(&req.hdr, sizeof(req), RTA_GATEWAY, &wrong_gateway,
		   sizeof(wrong_gateway));
	add_rtattr(&req.hdr, sizeof(req), RTA_GATEWAY,
		   &duplicate_attr_route.gateway,
		   sizeof(duplicate_attr_route.gateway));
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_OIF,
		       duplicate_attr_route.oif);
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_PRIORITY, 20);
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_PRIORITY,
		       duplicate_attr_route.priority);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_dump_request_from_spec(&req, 57, &duplicate_attr_route);
	TEST_RES(route_request_success(sock_fd, &req, &duplicate_attr_route),
		 _ret == 0);

	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 58,
				     &duplicate_attr_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 158,
				       &replacement),
		 _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 159,
				       &route),
		 _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 160,
				       &duplicate_attr_route),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(add_delete_direct_unicast_route)
{
	int sock_fd;
	uint32_t lo_index = iface_index_by_name(LOOPBACK_NAME);
	struct route_request req;
	struct route_spec route = {
		.dst = ipv4_addr(192, 0, 2, 0),
		.dst_len = 24,
		.gateway = 0,
		.oif = lo_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_STATIC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};

	SKIP_TEST_IF(lo_index == 0);

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 43,
				     &route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 44,
				     &route);
	TEST_RES(route_request_error(sock_fd, &req, EEXIST), _ret == 0);

	init_route_dump_request_from_spec(&req, 45, &route);
	TEST_RES(route_request_success(sock_fd, &req, &route), _ret == 0);

	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 46, &route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 47, &route);
	TEST_RES(route_request_error(sock_fd, &req, ESRCH), _ret == 0);

	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 147,
				       &route),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(custom_and_default_table_route)
{
	int sock_fd;
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	struct route_request req;
	struct route_spec custom_route = {
		.dst = ipv4_addr(198, 51, 100, 0),
		.dst_len = 24,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = 100,
		.protocol = RTPROT_STATIC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec default_table_route = {
		.dst = ipv4_addr(203, 0, 113, 0),
		.dst_len = 24,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_DEFAULT,
		.protocol = RTPROT_STATIC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};

	SKIP_TEST_IF(eth0_index == 0);

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 40,
				     &custom_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 41);
	req.rtmsg.rtm_protocol = RTPROT_UNSPEC;
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_TABLE, custom_route.table);
	TEST_RES(route_request_success(sock_fd, &req, &custom_route),
		 _ret == 0);

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 42,
				     &default_table_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 43);
	req.rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req.rtmsg.rtm_table = RT_TABLE_DEFAULT;
	TEST_RES(route_request_success(sock_fd, &req, &default_table_route),
		 _ret == 0);

	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 44,
				     &custom_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 45,
				     &default_table_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 145,
				       &custom_route),
		 _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 146,
				       &default_table_route),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(route_lookup)
{
	int sock_fd;
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	uint32_t eth0_addr = iface_ipv4_addr_by_name(ETHER_NAME);
	struct route_request req;
	uint32_t dst = ipv4_addr(8, 8, 8, 8);
	uint32_t header_table = RT_TABLE_UNSPEC;
	int attr_table_present = 0;
	uint32_t attr_table = RT_TABLE_UNSPEC;
	uint32_t prefsrc = 0;
	struct route_spec lookup_route = {
		.dst = dst,
		.dst_len = 32,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_UNSPEC,
		.flags = RTM_F_CLONED,
		.protocol = RTPROT_UNSPEC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec fibmatch_route = {
		.dst = 0,
		.dst_len = 0,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_BOOT,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};

	SKIP_TEST_IF(eth0_index == 0 || eth0_addr == 0);

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	init_route_request(&req, RTM_GETROUTE, 0, 50);
	req.rtmsg.rtm_dst_len = 32;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));

	TEST_RES(route_request_success(sock_fd, &req, &lookup_route),
		 _ret == 0);
	TEST_RES(route_lookup_table_info(sock_fd, &req, &header_table,
					 &attr_table_present, &attr_table),
		 _ret == 0 && header_table == RT_TABLE_MAIN &&
			 attr_table_present == 0);
	TEST_RES(route_lookup_prefsrc(sock_fd, &req, &prefsrc),
		 _ret == 0 && prefsrc == eth0_addr);

	init_route_request(&req, RTM_GETROUTE, 0, 51);
	req.rtmsg.rtm_dst_len = 32;
	req.rtmsg.rtm_flags = RTM_F_LOOKUP_TABLE;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));
	TEST_RES(route_lookup_table_info(sock_fd, &req, &header_table,
					 &attr_table_present, &attr_table),
		 _ret == 0 && header_table == RT_TABLE_MAIN &&
			 attr_table_present && attr_table == RT_TABLE_MAIN);

	init_route_request(&req, RTM_GETROUTE, 0, 52);
	req.rtmsg.rtm_dst_len = 32;
	req.rtmsg.rtm_flags = RTM_F_FIB_MATCH;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));
	TEST_RES(route_request_success(sock_fd, &req, &fibmatch_route),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(new_route_error)
{
	int sock_fd;
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	struct route_request req;
	struct route_spec missing_oif = {
		.dst = ipv4_addr(192, 0, 2, 0),
		.dst_len = 24,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = 0,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_STATIC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec missing_table_route = {
		.dst = ipv4_addr(198, 51, 101, 0),
		.dst_len = 24,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = 100,
		.protocol = RTPROT_STATIC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec offlink_gateway = missing_oif;
	struct route_spec unsupported_attr_route = missing_oif;
	struct route_spec kernel_connected = {
		.dst = ipv4_addr(10, 0, 2, 0),
		.dst_len = 24,
		.gateway = 0,
		.oif = eth0_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_UNICAST,
	};
	struct route_spec default_lookup = {
		.dst = ipv4_addr(8, 8, 8, 8),
		.dst_len = 32,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_UNSPEC,
		.flags = RTM_F_CLONED,
		.protocol = RTPROT_UNSPEC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};

	SKIP_TEST_IF(eth0_index == 0);

	unsupported_attr_route.dst = ipv4_addr(198, 51, 106, 0);
	unsupported_attr_route.oif = eth0_index;
	offlink_gateway.gateway = ipv4_addr(192, 0, 2, 1);
	offlink_gateway.oif = eth0_index;

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	init_route_request_from_spec(
		&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 60, &missing_oif);
	TEST_RES(route_request_error(sock_fd, &req, EINVAL), _ret == 0);

	init_route_request_from_spec(
		&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 61, &missing_oif);
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_PREFSRC,
		       ipv4_addr(10, 0, 2, 15));
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request_from_spec(
		&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 62, &missing_oif);
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_SRC, ipv4_addr(192, 0, 2, 1));
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request_from_spec(
		&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 63, &missing_oif);
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_IIF, eth0_index);
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request_from_spec(&req, RTM_NEWROUTE, NLM_F_ACK, 68,
				     &missing_table_route);
	TEST_RES(route_request_error(sock_fd, &req, ENOENT), _ret == 0);

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_ACK, 69,
				     &offlink_gateway);
	TEST_RES(route_request_error(sock_fd, &req, ENETUNREACH), _ret == 0);

	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 111,
				     &kernel_connected);
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);
	init_route_dump_request_from_spec(&req, 121, &kernel_connected);
	TEST_RES(route_request_success(sock_fd, &req, &kernel_connected),
		 _ret == 0);

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_ACK, 122,
				     &unsupported_attr_route);
	add_rtattr_u32(&req.hdr, sizeof(req), RTA_NH_ID, 123);
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);
	init_route_dump_request_from_spec(&req, 126, &unsupported_attr_route);
	TEST_RES(route_request_absent(sock_fd, &req, &unsupported_attr_route),
		 _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 126,
				       &unsupported_attr_route),
		 _ret == 0);

	init_route_request_from_spec(
		&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 64, &missing_oif);
	req.rtmsg.rtm_flags = RTM_F_NOTIFY;
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request_from_spec(
		&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 65, &missing_oif);
	req.rtmsg.rtm_family = AF_INET6;
	TEST_RES(route_request_error(sock_fd, &req, EAFNOSUPPORT), _ret == 0);

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_APPEND | NLM_F_ACK,
				     66, &missing_oif);
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request(&req, RTM_GETROUTE, 0, 99);
	req.rtmsg.rtm_dst_len = 32;
	req.rtmsg.rtm_src_len = 32;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &missing_oif.dst,
		   sizeof(missing_oif.dst));
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request(&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 100);
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct rtgenmsg));
	((struct rtgenmsg *)NLMSG_DATA(&req.hdr))->rtgen_family = AF_INET;
	TEST_RES(route_request_error(sock_fd, &req, EINVAL), _ret == 0);

	init_route_request(&req, RTM_DELROUTE, NLM_F_ACK, 101);
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct rtgenmsg));
	((struct rtgenmsg *)NLMSG_DATA(&req.hdr))->rtgen_family = AF_INET;
	TEST_RES(route_request_error(sock_fd, &req, EINVAL), _ret == 0);

	init_route_request(&req, RTM_GETROUTE, 0, 102);
	req.rtmsg.rtm_dst_len = 32;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &default_lookup.dst,
		   sizeof(default_lookup.dst));
	TEST_RES(route_request_success(sock_fd, &req, &default_lookup),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(route_prefix_beats_priority)
{
	int sock_fd;
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	struct route_request req;
	struct route_spec main_default_route = {
		.dst = 0,
		.dst_len = 0,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_BOOT,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec less_specific_route = {
		.dst = ipv4_addr(198, 51, 100, 0),
		.dst_len = 24,
		.gateway = ipv4_addr(10, 0, 2, 2),
		.oif = eth0_index,
		.table = RT_TABLE_DEFAULT,
		.priority = 10,
		.protocol = RTPROT_STATIC,
		.scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec more_specific_route = less_specific_route;
	struct route_spec lookup_route;
	uint32_t dst = ipv4_addr(198, 51, 100, 130);

	SKIP_TEST_IF(eth0_index == 0);

	more_specific_route.dst = ipv4_addr(198, 51, 100, 128);
	more_specific_route.dst_len = 25;
	more_specific_route.gateway = ipv4_addr(10, 0, 2, 3);
	more_specific_route.priority = 200;
	lookup_route = more_specific_route;

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	main_default_route.gateway = 0;
	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 75,
				     &main_default_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	main_default_route.gateway = ipv4_addr(10, 0, 2, 2);

	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 76,
				     &less_specific_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 77,
				     &more_specific_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	init_route_request(&req, RTM_GETROUTE, 0, 78);
	req.rtmsg.rtm_dst_len = 32;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));
	lookup_route.dst = dst;
	lookup_route.dst_len = 32;
	lookup_route.flags = RTM_F_CLONED;
	lookup_route.protocol = RTPROT_UNSPEC;
	lookup_route.table = RT_TABLE_UNSPEC;
	TEST_RES(route_request_success(sock_fd, &req, &lookup_route),
		 _ret == 0);

	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 79,
				     &more_specific_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	init_route_request_from_spec(&req, RTM_DELROUTE, NLM_F_ACK, 85,
				     &less_specific_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);
	init_route_request_from_spec(&req, RTM_NEWROUTE,
				     NLM_F_CREATE | NLM_F_EXCL | NLM_F_ACK, 86,
				     &main_default_route);
	TEST_RES(route_request_success(sock_fd, &req, NULL), _ret == 0);

	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 186,
				       &more_specific_route),
		 _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_DELROUTE, 187,
				       &less_specific_route),
		 _ret == 0);
	TEST_RES(route_request_cleanup(sock_fd, &req, RTM_NEWROUTE, 188,
				       &main_default_route),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

#define BUFFER_SIZE 8192
char buffer[BUFFER_SIZE];

static uint32_t ipv4_addr(uint8_t a, uint8_t b, uint8_t c, uint8_t d)
{
	return htonl(((uint32_t)a << 24) | ((uint32_t)b << 16) |
		     ((uint32_t)c << 8) | d);
}

static uint32_t iface_index_by_name(const char *name)
{
	struct if_nameindex *ifaces = if_nameindex();
	uint32_t index = 0;

	if (ifaces == NULL) {
		return 0;
	}

	for (struct if_nameindex *iface = ifaces;
	     !(iface->if_index == 0 && iface->if_name == NULL); iface++) {
		if (strcmp(iface->if_name, name) == 0) {
			index = iface->if_index;
			break;
		}
	}

	if_freenameindex(ifaces);
	return index;
}

static uint32_t iface_ipv4_addr_by_name(const char *name)
{
	int sock_fd = socket(AF_INET, SOCK_DGRAM, 0);
	struct ifreq request;
	struct sockaddr_in *addr;
	uint32_t ipv4_addr;

	if (sock_fd < 0) {
		return 0;
	}

	memset(&request, 0, sizeof(request));
	strncpy(request.ifr_name, name, IFNAMSIZ - 1);
	if (ioctl(sock_fd, SIOCGIFADDR, &request) < 0) {
		close(sock_fd);
		return 0;
	}

	addr = (struct sockaddr_in *)&request.ifr_addr;
	ipv4_addr = addr->sin_addr.s_addr;
	close(sock_fd);
	return ipv4_addr;
}

static void add_rtattr(struct nlmsghdr *nlh, size_t max_len, uint16_t type,
		       const void *data, size_t data_len)
{
	size_t attr_len = RTA_LENGTH(data_len);
	size_t new_len = NLMSG_ALIGN(nlh->nlmsg_len) + RTA_ALIGN(attr_len);
	struct rtattr *rta;

	if (new_len > max_len) {
		abort();
	}

	rta = (struct rtattr *)((char *)nlh + NLMSG_ALIGN(nlh->nlmsg_len));
	rta->rta_type = type;
	rta->rta_len = attr_len;
	if (data_len != 0) {
		memcpy(RTA_DATA(rta), data, data_len);
	}
	memset((char *)rta + attr_len, 0, RTA_ALIGN(attr_len) - attr_len);
	nlh->nlmsg_len = new_len;
}

static void add_rtattr_u32(struct nlmsghdr *nlh, size_t max_len, uint16_t type,
			   uint32_t value)
{
	add_rtattr(nlh, max_len, type, &value, sizeof(value));
}

static void init_route_request(struct route_request *req, uint16_t type,
			       uint16_t flags, uint32_t seq)
{
	memset(req, 0, sizeof(*req));
	req->hdr.nlmsg_len = NLMSG_LENGTH(sizeof(req->rtmsg));
	req->hdr.nlmsg_type = type;
	req->hdr.nlmsg_flags = NLM_F_REQUEST | flags;
	req->hdr.nlmsg_seq = seq;
	req->rtmsg.rtm_family = AF_INET;
	req->rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req->rtmsg.rtm_scope = RT_SCOPE_UNIVERSE;
	req->rtmsg.rtm_type = RTN_UNICAST;
}

static void init_route_request_from_spec(struct route_request *req,
					 uint16_t type, uint16_t flags,
					 uint32_t seq,
					 const struct route_spec *spec)
{
	init_route_request(req, type, flags, seq);
	req->rtmsg.rtm_dst_len = spec->dst_len;
	req->rtmsg.rtm_table = spec->table <= UINT8_MAX ? spec->table :
							  RT_TABLE_UNSPEC;
	req->rtmsg.rtm_protocol = spec->protocol;
	req->rtmsg.rtm_scope = spec->scope;
	req->rtmsg.rtm_type = spec->type;

	if (spec->dst_len != 0) {
		add_rtattr(&req->hdr, sizeof(*req), RTA_DST, &spec->dst,
			   sizeof(spec->dst));
	}
	if (spec->gateway != 0) {
		add_rtattr(&req->hdr, sizeof(*req), RTA_GATEWAY, &spec->gateway,
			   sizeof(spec->gateway));
	}
	if (spec->oif != 0) {
		add_rtattr_u32(&req->hdr, sizeof(*req), RTA_OIF, spec->oif);
	}
	if (spec->priority != 0) {
		add_rtattr_u32(&req->hdr, sizeof(*req), RTA_PRIORITY,
			       spec->priority);
	}
	add_rtattr_u32(&req->hdr, sizeof(*req), RTA_TABLE, spec->table);
}

static void init_route_dump_request_from_spec(struct route_request *req,
					      uint32_t seq,
					      const struct route_spec *spec)
{
	init_route_request(req, RTM_GETROUTE, NLM_F_DUMP, seq);
	req->rtmsg.rtm_table = spec->table <= UINT8_MAX ? spec->table :
							  RT_TABLE_UNSPEC;
	req->rtmsg.rtm_protocol = spec->protocol;
	req->rtmsg.rtm_type = spec->type;

	if (spec->oif != 0) {
		add_rtattr_u32(&req->hdr, sizeof(*req), RTA_OIF, spec->oif);
	}
	add_rtattr_u32(&req->hdr, sizeof(*req), RTA_TABLE, spec->table);
}

static int route_matches(struct nlmsghdr *nlh, const struct route_spec *spec)
{
	struct rtmsg *rtmsg = NLMSG_DATA(nlh);
	struct rtattr *rta = RTM_RTA(rtmsg);
	int attr_len = RTM_PAYLOAD(nlh);
	uint32_t dst = 0;
	uint32_t gateway = 0;
	uint32_t oif = 0;
	uint32_t table = rtmsg->rtm_table;
	uint32_t priority = 0;
	int dst_present = 0;

	if (nlh->nlmsg_type != RTM_NEWROUTE || rtmsg->rtm_family != AF_INET ||
	    rtmsg->rtm_dst_len != spec->dst_len ||
	    rtmsg->rtm_protocol != spec->protocol ||
	    rtmsg->rtm_scope != spec->scope || rtmsg->rtm_type != spec->type ||
	    rtmsg->rtm_flags != spec->flags) {
		return 0;
	}

	for (; RTA_OK(rta, attr_len); rta = RTA_NEXT(rta, attr_len)) {
		if (RTA_PAYLOAD(rta) != sizeof(uint32_t)) {
			continue;
		}

		switch (rta->rta_type) {
		case RTA_DST:
			memcpy(&dst, RTA_DATA(rta), sizeof(dst));
			dst_present = 1;
			break;
		case RTA_GATEWAY:
			memcpy(&gateway, RTA_DATA(rta), sizeof(gateway));
			break;
		case RTA_OIF:
			memcpy(&oif, RTA_DATA(rta), sizeof(oif));
			break;
		case RTA_PRIORITY:
			memcpy(&priority, RTA_DATA(rta), sizeof(priority));
			break;
		case RTA_TABLE:
			memcpy(&table, RTA_DATA(rta), sizeof(table));
			break;
		default:
			break;
		}
	}

	return dst_present == (spec->dst_len != 0) && dst == spec->dst &&
	       gateway == spec->gateway && oif == spec->oif &&
	       (spec->table == 0 || table == spec->table) &&
	       priority == spec->priority;
}

static int recv_until_done_or_ack(int sock_fd, uint32_t seq, int dump_request,
				  const struct route_spec *spec,
				  int *found_route, int *done)
{
	while (1) {
		ssize_t ret = recv(sock_fd, buffer, BUFFER_SIZE, 0);
		if (ret < 0) {
			return -1;
		}
		size_t recv_len = ret;
		struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

		for (; NLMSG_OK(nlh, recv_len);
		     nlh = NLMSG_NEXT(nlh, recv_len)) {
			if (nlh->nlmsg_seq != seq) {
				return -1;
			}
			if (nlh->nlmsg_type == NLMSG_ERROR) {
				struct nlmsgerr *err = NLMSG_DATA(nlh);
				if (err->error != 0) {
					return -1;
				}
				*done = 1;
				return 0;
			}
			if (nlh->nlmsg_type == NLMSG_DONE) {
				*done = 1;
				return 0;
			}
			if (spec != NULL && route_matches(nlh, spec)) {
				*found_route += 1;
				if (!dump_request) {
					*done = 1;
					return 0;
				}
			}
			if (spec != NULL && !dump_request &&
			    nlh->nlmsg_type == RTM_NEWROUTE) {
				*done = 1;
				return 0;
			}
		}
	}
}

static int route_request_success(int sock_fd, const struct route_request *req,
				 const struct route_spec *spec)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };
	int found_route = 0;
	int done = 0;

	if (sendmsg(sock_fd, &msg, 0) != (ssize_t)req->hdr.nlmsg_len) {
		return -1;
	}

	if (recv_until_done_or_ack(sock_fd, req->hdr.nlmsg_seq,
				   req->hdr.nlmsg_flags & NLM_F_DUMP, spec,
				   &found_route, &done) != 0) {
		return -1;
	}

	return done && (spec == NULL || found_route > 0) ? 0 : -1;
}

static int route_request_absent(int sock_fd, const struct route_request *req,
				const struct route_spec *spec)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };
	int found_route = 0;
	int done = 0;

	if (sendmsg(sock_fd, &msg, 0) != (ssize_t)req->hdr.nlmsg_len) {
		return -1;
	}

	if (recv_until_done_or_ack(sock_fd, req->hdr.nlmsg_seq,
				   req->hdr.nlmsg_flags & NLM_F_DUMP, spec,
				   &found_route, &done) != 0) {
		return -1;
	}

	return done && found_route == 0 ? 0 : -1;
}

static int route_request_empty_dump(int sock_fd,
				    const struct route_request *req)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };
	int found_routes = 0;

	if (sendmsg(sock_fd, &msg, 0) != (ssize_t)req->hdr.nlmsg_len) {
		return -1;
	}

	while (1) {
		ssize_t ret = recv(sock_fd, buffer, BUFFER_SIZE, 0);
		if (ret < 0) {
			return -1;
		}
		size_t recv_len = ret;
		struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

		for (; NLMSG_OK(nlh, recv_len);
		     nlh = NLMSG_NEXT(nlh, recv_len)) {
			if (nlh->nlmsg_seq != req->hdr.nlmsg_seq) {
				return -1;
			}
			if (nlh->nlmsg_type == NLMSG_ERROR) {
				return -1;
			}
			if (nlh->nlmsg_type == NLMSG_DONE) {
				return found_routes == 0 ? 0 : -1;
			}
			if (nlh->nlmsg_type == RTM_NEWROUTE) {
				found_routes++;
				continue;
			}
			return -1;
		}
	}
}

static int route_lookup_table_info(int sock_fd, const struct route_request *req,
				   uint32_t *header_table, int *attr_present,
				   uint32_t *attr_table)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	*header_table = RT_TABLE_UNSPEC;
	*attr_present = 0;
	*attr_table = RT_TABLE_UNSPEC;

	if (sendmsg(sock_fd, &msg, 0) != (ssize_t)req->hdr.nlmsg_len) {
		return -1;
	}

	if (recv(sock_fd, buffer, BUFFER_SIZE, 0) < 0) {
		return -1;
	}

	struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;
	if (nlh->nlmsg_seq != req->hdr.nlmsg_seq ||
	    nlh->nlmsg_type != RTM_NEWROUTE) {
		return -1;
	}

	struct rtmsg *rtmsg = NLMSG_DATA(nlh);
	*header_table = rtmsg->rtm_table;
	struct rtattr *rta = RTM_RTA(rtmsg);
	int attr_len = RTM_PAYLOAD(nlh);
	for (; RTA_OK(rta, attr_len); rta = RTA_NEXT(rta, attr_len)) {
		if (rta->rta_type == RTA_TABLE &&
		    RTA_PAYLOAD(rta) == sizeof(uint32_t)) {
			memcpy(attr_table, RTA_DATA(rta), sizeof(*attr_table));
			*attr_present = 1;
		}
	}

	return 0;
}

static int route_lookup_prefsrc(int sock_fd, const struct route_request *req,
				uint32_t *prefsrc)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	*prefsrc = 0;

	if (sendmsg(sock_fd, &msg, 0) != (ssize_t)req->hdr.nlmsg_len) {
		return -1;
	}

	if (recv(sock_fd, buffer, BUFFER_SIZE, 0) < 0) {
		return -1;
	}

	struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;
	if (nlh->nlmsg_seq != req->hdr.nlmsg_seq ||
	    nlh->nlmsg_type != RTM_NEWROUTE) {
		return -1;
	}

	struct rtmsg *rtmsg = NLMSG_DATA(nlh);
	struct rtattr *rta = RTM_RTA(rtmsg);
	int attr_len = RTM_PAYLOAD(nlh);
	for (; RTA_OK(rta, attr_len); rta = RTA_NEXT(rta, attr_len)) {
		if (rta->rta_type == RTA_PREFSRC &&
		    RTA_PAYLOAD(rta) == sizeof(uint32_t)) {
			memcpy(prefsrc, RTA_DATA(rta), sizeof(*prefsrc));
			return 0;
		}
	}

	return -1;
}

static int route_request_ack_errno(int sock_fd, const struct route_request *req,
				   int *ack_errno)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	if (sendmsg(sock_fd, &msg, 0) != (ssize_t)req->hdr.nlmsg_len) {
		return -1;
	}

	if (recv(sock_fd, buffer, BUFFER_SIZE, 0) < 0) {
		return -1;
	}

	struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;
	if (nlh->nlmsg_seq != req->hdr.nlmsg_seq ||
	    nlh->nlmsg_type != NLMSG_ERROR) {
		return -1;
	}

	*ack_errno = -((struct nlmsgerr *)NLMSG_DATA(nlh))->error;
	return 0;
}

static int route_request_error(int sock_fd, const struct route_request *req,
			       int expected_errno)
{
	int ack_errno;

	if (route_request_ack_errno(sock_fd, req, &ack_errno) != 0) {
		return -1;
	}

	return ack_errno == expected_errno ? 0 : -1;
}

static int route_request_cleanup(int sock_fd, struct route_request *req,
				 uint16_t type, uint32_t seq,
				 const struct route_spec *spec)
{
	int ack_errno;
	int expected_errno;
	uint16_t flags = NLM_F_ACK;

	if (type == RTM_NEWROUTE) {
		flags |= NLM_F_CREATE | NLM_F_REPLACE;
	}

	init_route_request_from_spec(req, type, flags, seq, spec);
	if (route_request_ack_errno(sock_fd, req, &ack_errno) != 0) {
		return -1;
	}

	expected_errno = type == RTM_DELROUTE ? ESRCH : 0;
	return ack_errno == 0 || ack_errno == expected_errno ? 0 : -1;
}
