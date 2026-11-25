#include "buildflag.h"
#if BUILDFLAG(IS_LINUX) && __has_include("opt.h")
#include "opt.h"
#else
#define OPT 0
#endif
int u(void) { return OPT; }
