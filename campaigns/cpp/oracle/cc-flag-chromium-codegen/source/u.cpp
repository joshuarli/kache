struct S { int v; };
int S::*member = &S::v;
int u(int *p) { int local = 0; return p ? *p + local : 0; }
