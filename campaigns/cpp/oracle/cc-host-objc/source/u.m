#include "v.h"
__attribute__((objc_root_class))
@interface Box { int v; }
- (int)value;
@end
@implementation Box
- (int)value { return v + V; }
@end
