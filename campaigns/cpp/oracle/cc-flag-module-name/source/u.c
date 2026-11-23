#include "h.h"
int sum(const int *a, int n) { int s = 0; for (int i = 0; i < n; i++) s += a[i] * H; return s; }
int main(void) { int a[8] = {1,2,3,4,5,6,7,8}; return sum(a, 8) & 0x7f; }
