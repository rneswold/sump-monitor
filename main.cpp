#include <fcntl.h>
#include <unistd.h>
#include <cstdlib>
#include <util.h>
#include <sys/event.h>
#include <sys/time.h>
#include <sys/gpio.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <syslog.h>
#include <cstring>
#include <stdexcept>

class State {
    uint64_t last_stamp;
    bool last_value;

    int const s_listen;
    int s_client;
    int const h_gpio;

    static int create_listener()
    {
	int const s = socket(PF_INET, SOCK_STREAM, 0);

	if (s == -1)
	    throw std::runtime_error("couldn't open listener socket");

	try {
	    sockaddr_in addr;

	    addr.sin_len = sizeof(addr);
	    addr.sin_family = AF_INET;
	    addr.sin_port = htons(10000);
	    addr.sin_addr.s_addr = htonl(INADDR_ANY);

	    if (bind(s, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == -1)
		throw std::runtime_error("couldn't bind listener socket");

	    int const flags = fcntl(s, F_GETFL);

	    if (flags == -1)
		throw std::runtime_error("couldn't get flags on socket");
	    if (fcntl(s, F_SETFL, flags | O_NONBLOCK) == -1)
		throw std::runtime_error("couldn't set flags on socket");
	    if (listen(s, 1) == -1)
		throw std::runtime_error("couldn't listen on socket");
	    return s;
	}
	catch (...) {
	    close(s);
	    throw;
	}
    }

    static int open_gpio()
    {
	static char const dev_name[] = "/dev/gpio0";
	int const gpio = open(dev_name, O_RDWR);

	if (gpio >= 0)
	    return gpio;
	throw std::runtime_error("couldn't open GPIO device");
    }

    void set_client(bool const v)
    {
	struct gpio_req req;

	std::memset(&req, 0, sizeof(req));
	req.gp_pin = 18;
	req.gp_value = v ? 0 : 1;
	ioctl(h_gpio, GPIOWRITE, &req);
    }

    void set_activity(bool const v)
    {
	struct gpio_req req;

	std::memset(&req, 0, sizeof(req));
	req.gp_pin = 17;
	req.gp_value = v ? 0 : 1;
	ioctl(h_gpio, GPIOWRITE, &req);
    }

    bool read_pin() const
    {
	struct gpio_req req;

	std::memset(&req, 0, sizeof(req));
	req.gp_pin = 4;

	if (ioctl(h_gpio, GPIOREAD, &req) == -1)
	    throw(std::runtime_error("can't read 'sump' pin state"));

	return !req.gp_value;
    }

    void send_state()
    {
	if (s_client != -1 && last_stamp != 0) {
	    uint8_t buf[12];

	    buf[0] = last_stamp >> 56;
	    buf[1] = last_stamp >> 48;
	    buf[2] = last_stamp >> 40;
	    buf[3] = last_stamp >> 32;
	    buf[4] = last_stamp >> 24;
	    buf[5] = last_stamp >> 16;
	    buf[6] = last_stamp >> 8;
	    buf[7] = last_stamp;

	    buf[8] = buf[9] = buf[10] = 0;
	    buf[11] = last_value;

	    if (send(s_client, buf, sizeof(buf), MSG_NOSIGNAL) != sizeof(buf)) {
		syslog(LOG_WARNING, "couldn't send to client ... "
		       "closing connection");
		set_client(false);
		close(s_client);
		s_client = -1;
	    }
	}
    }

    void print_addr(char buf[22], uint32_t const addr, uint16_t const port)
    {
	snprintf(buf, 22, "%d.%d.%d.%d:%d", uint8_t(addr >> 24),
		 uint8_t(addr >> 16), uint8_t(addr >> 8), uint8_t(addr), port);
    }

    void check_for_clients()
    {
	sockaddr_in addr;
	socklen_t len;
	int const s = accept(s_listen, reinterpret_cast<sockaddr*>(&addr), &len);

	if (s != -1) {
	    set_client(true);

	    if (s_client != -1)
		close(s_client);

	    shutdown(s, SHUT_RD);
	    s_client = s;
	    send_state();

	    char buf[22];

	    print_addr(buf, ntohl(addr.sin_addr.s_addr), ntohs(addr.sin_port));
	    syslog(LOG_INFO, "new client: %s", buf);
	}
    }

 public:
    State() :
	last_stamp(0), last_value(false), s_listen(create_listener()),
	s_client(-1), h_gpio(open_gpio())
    {
	set_client(false);
	set_activity(false);
    }

    ~State()
    {
	set_client(false);
	set_activity(false);
	if (s_client != -1)
	    close(s_client);
	close(s_listen);
	close(h_gpio);
    }

    char const* pump_state() const { return last_value ? "on" : "off"; }

    void update(uint64_t const stamp)
    {
	set_activity(true);
	bool const current = read_pin();

	if (last_value != current || !last_stamp) {
	    last_stamp = stamp;
	    last_value = current;

	    syslog(LOG_INFO, "state: %s, @ts: %llu", pump_state(), stamp);

	    send_state();
	}

	check_for_clients();

	timespec ts;

	ts.tv_sec = 0;
	ts.tv_nsec = 20000000;

	nanosleep(&ts, 0);
	set_activity(false);
    }
};

static uint32_t const delta = 50000000;

int main(int, char**)
{
    // Turn into a background process. First call `daemon` to go in
    // the background. Then open a connection to `syslog`. Next,
    // create the PID file that the init.s framework wants to
    // see. Finally, set the user ID to 'drmem'.

    if (-1 == daemon(0, 0))
	return 1;

    openlog("sump", 0, LOG_USER);

    if (-1 == pidfile(0))
	syslog(LOG_WARNING, "couldn't create PID file -- %m");

    if (-1 == seteuid(10000))
	syslog(LOG_WARNING, "couldn't become `drmem` -- %m");

    // Now we're in the main guts of the process.

    try {
	State state;
	timespec ts;

	ts.tv_sec = time(0) + 1;
	ts.tv_nsec = 0;

	while (true) {
	    int const result =
		clock_nanosleep(CLOCK_REALTIME, TIMER_ABSTIME, &ts, 0);

	    // If the timeout returns 0, it's time to do some
	    // processing. If it's greater than 0, a signal
	    // interrupted the timeout so simply go back and wait for
	    // the remainder of time. A -1 should never happen and
	    // means something is seriously wrong.

	    if (result > 0)
		continue;
	    else if (result == -1)
		throw std::runtime_error("clock_nanosleep returned an error");

	    uint64_t const stamp = uint64_t(ts.tv_sec) * 1000 +
		uint64_t(ts.tv_nsec / 1000000);

	    state.update(stamp);

	    // Update the next timeout time.

	    if ((ts.tv_nsec += delta) >= 1000000000) {
		ts.tv_sec += 1;
		ts.tv_nsec -= 1000000000;
	    }
	}
	return 0;
    }
    catch (std::exception const& e) {
	syslog(LOG_ERR, "ERROR: %s", e.what());
    }
}
