#define JOIN_INNER(a,b) a##b
#define JOIN(a,b) JOIN_INNER(a,b)
#define LEFT __TIME
#define RIGHT STAMP__
const char *stamp = JOIN(LEFT, RIGHT);
