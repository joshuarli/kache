extern int g(int);
int f(int a) { return g(a) + 1; }
int main(void) { return 0; }
int g(int a) { return a; }
