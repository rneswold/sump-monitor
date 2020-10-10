CXXFLAGS+=-W -Wall -Werror

sump : main.o
	c++ -lutil -o ${.TARGET} ${.ALLSRC}
