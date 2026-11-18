#include "v.h"
template <int N> int k() { return N; }
__attribute__((objc_root_class))
@interface Box { int v; }
- (int)value;
@end
@implementation Box
- (int)value { return v + k<V>(); }
@end
