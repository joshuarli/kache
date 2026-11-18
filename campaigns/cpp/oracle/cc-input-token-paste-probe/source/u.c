#define CAT(a, b) a##b
#define PROBE(n) __has_include(CAT(<feat_, n>))
#if PROBE(x.h)
#include <feat_x.h>
#else
#define FEAT 0
#endif
int u(void) { return FEAT; }
