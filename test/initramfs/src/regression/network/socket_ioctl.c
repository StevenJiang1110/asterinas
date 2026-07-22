// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <errno.h>
#include <linux/if.h>
#include <linux/if_arp.h>
#include <linux/netlink.h>
#include <linux/sockios.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

#ifdef __asterinas__
#include <linux/vm_sockets.h>
#endif

#include "../common/test.h"

static int fd;

static int expect_unsupported_socket_ioctl(int socket_fd, unsigned long request,
					   struct ifreq *ifreq)
{
	int ret = ioctl(socket_fd, request, ifreq);
	if (ret < 0 && (errno == ENOTTY || errno == EOPNOTSUPP))
		errno = 0;
	return ret;
}

FN_SETUP(general)
{
	fd = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
}
END_SETUP()

FN_TEST(interface_queries)
{
	struct ifreq ifr = { 0 };
	strncpy(ifr.ifr_name, "lo", IFNAMSIZ - 1);

	TEST_SUCC(ioctl(fd, SIOCGIFINDEX, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFINDEX, &ifr), ifr.ifr_ifindex > 0);
	int loopback_index = ifr.ifr_ifindex;

	memset(&ifr, 0, sizeof(ifr));
	ifr.ifr_ifindex = loopback_index;
	TEST_SUCC(ioctl(fd, SIOCGIFNAME, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFNAME, &ifr), strcmp(ifr.ifr_name, "lo") == 0);

	memset(&ifr, 0, sizeof(ifr));
	strncpy(ifr.ifr_name, "lo", IFNAMSIZ - 1);
	TEST_SUCC(ioctl(fd, SIOCGIFFLAGS, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFFLAGS, &ifr),
		 (ifr.ifr_flags & (IFF_UP | IFF_LOOPBACK | IFF_RUNNING)) ==
			 (IFF_UP | IFF_LOOPBACK | IFF_RUNNING));

	TEST_SUCC(ioctl(fd, SIOCGIFADDR, &ifr));
	TEST_RES(
		ioctl(fd, SIOCGIFADDR, &ifr),
		ifr.ifr_addr.sa_family == AF_INET &&
			((struct sockaddr_in *)&ifr.ifr_addr)->sin_addr.s_addr ==
				htonl(INADDR_LOOPBACK));
	TEST_SUCC(ioctl(fd, SIOCGIFDSTADDR, &ifr));
	TEST_SUCC(ioctl(fd, SIOCGIFBRDADDR, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFBRDADDR, &ifr),
		 ((struct sockaddr_in *)&ifr.ifr_broadaddr)->sin_addr.s_addr ==
			 0);
	TEST_SUCC(ioctl(fd, SIOCGIFNETMASK, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFNETMASK, &ifr),
		 ((struct sockaddr_in *)&ifr.ifr_netmask)->sin_addr.s_addr ==
			 htonl(0xff000000));

	TEST_SUCC(ioctl(fd, SIOCGIFMETRIC, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFMETRIC, &ifr), ifr.ifr_metric == 0);
	TEST_SUCC(ioctl(fd, SIOCGIFMTU, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFMTU, &ifr), ifr.ifr_mtu > 0);
	TEST_SUCC(ioctl(fd, SIOCGIFHWADDR, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFHWADDR, &ifr),
		 ifr.ifr_hwaddr.sa_family == ARPHRD_LOOPBACK);
	TEST_SUCC(ioctl(fd, SIOCGIFTXQLEN, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFTXQLEN, &ifr), ifr.ifr_qlen == 1000);
	TEST_SUCC(ioctl(fd, SIOCGIFMAP, &ifr));
	TEST_RES(ioctl(fd, SIOCGIFMAP, &ifr),
		 ifr.ifr_map.mem_start == 0 && ifr.ifr_map.mem_end == 0);

	memset(&ifr, 0, sizeof(ifr));
	strncpy(ifr.ifr_name, "missing", IFNAMSIZ - 1);
	TEST_ERRNO(ioctl(fd, SIOCGIFINDEX, &ifr), ENODEV);
	memset(&ifr, 0, sizeof(ifr));
	ifr.ifr_ifindex = 0;
	TEST_ERRNO(ioctl(fd, SIOCGIFNAME, &ifr), ENODEV);

	memset(&ifr, 'x', sizeof(ifr));
	TEST_ERRNO(ioctl(fd, SIOCGIFINDEX, &ifr), ENODEV);
}
END_TEST()

FN_TEST(interface_conf)
{
	struct ifconf ifc = { .ifc_len = 0, .ifc_buf = NULL };
	TEST_SUCC(ioctl(fd, SIOCGIFCONF, &ifc));
	TEST_RES(ioctl(fd, SIOCGIFCONF, &ifc),
		 ifc.ifc_len >= (int)sizeof(struct ifreq));

	int capacity = ifc.ifc_len;
	struct ifreq *ifreqs = calloc(1, capacity);
	ifc.ifc_len = capacity;
	ifc.ifc_buf = (char *)ifreqs;
	TEST_SUCC(ioctl(fd, SIOCGIFCONF, &ifc));
	TEST_RES(ioctl(fd, SIOCGIFCONF, &ifc),
		 ifc.ifc_len >= (int)sizeof(struct ifreq));
	int found_lo = 0;
	for (int offset = 0; offset < ifc.ifc_len;
	     offset += sizeof(struct ifreq)) {
		if (strcmp(ifreqs[offset / sizeof(struct ifreq)].ifr_name,
			   "lo") == 0)
			found_lo = 1;
	}
	TEST_RES(ioctl(fd, SIOCGIFCONF, &ifc), found_lo);
	free(ifreqs);
}
END_TEST()

FN_TEST(socket_types)
{
	int sockets[] = {
		CHECK(socket(AF_INET, SOCK_STREAM, 0)),
		CHECK(socket(AF_INET, SOCK_DGRAM, 0)),
		CHECK(socket(AF_UNIX, SOCK_STREAM, 0)),
	};
	for (size_t i = 0; i < sizeof(sockets) / sizeof(sockets[0]); i++) {
		struct ifreq ifr = { 0 };
		strncpy(ifr.ifr_name, "lo", IFNAMSIZ - 1);
		TEST_SUCC(ioctl(sockets[i], SIOCGIFINDEX, &ifr));
		TEST_RES(ioctl(sockets[i], SIOCGIFINDEX, &ifr),
			 ifr.ifr_ifindex > 0);
		close(sockets[i]);
	}

	int route = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
	if (route >= 0) {
		struct ifreq ifr = { 0 };
		strncpy(ifr.ifr_name, "lo", IFNAMSIZ - 1);
		TEST_SUCC(ioctl(route, SIOCGIFINDEX, &ifr));
		close(route);
	}

#ifdef __asterinas__
	int vsock = socket(AF_VSOCK, SOCK_STREAM, 0);
	if (vsock >= 0) {
		struct ifreq ifr = { 0 };
		strncpy(ifr.ifr_name, "lo", IFNAMSIZ - 1);
		TEST_SUCC(expect_unsupported_socket_ioctl(vsock, SIOCGIFINDEX,
							  &ifr));
		close(vsock);
	}
#endif
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
}
END_SETUP()
