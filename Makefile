CXXFLAGS+=-W -Wall -Werror -O2

sump : main.o
	c++ -o ${.TARGET} ${.ALLSRC}
