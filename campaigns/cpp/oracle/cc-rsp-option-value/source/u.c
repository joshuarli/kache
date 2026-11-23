#define S(x) #x
#define T(x) S(x)
int main(void) { return sizeof(T(TAG)); }
