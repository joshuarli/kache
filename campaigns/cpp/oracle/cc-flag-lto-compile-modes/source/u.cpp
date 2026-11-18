struct B { virtual int f(); };
int B::f() { return 1; }
int use(B *b) { return b->f(); }
