CXXFLAGS+=-W -Wall -Werror

sump : main.o
	c++ -lrt -lutil -o ${.TARGET} ${.ALLSRC}
