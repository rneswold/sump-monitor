CXXFLAGS+=-Wextra -Wall -Werror

ifdef DEBUG
CXXFLAGS+=-O2 -g
else
CXXFLAGS+=-DNDEBUG -O2
endif

sumpd : main.o
	c++ -g -lrt -lutil -o $@ $^

clean :
	rm -f sump *.o
