#include <cstdio>
extern "C" int answer(void);
extern "C" int adjust(void);
extern "C" int extra(void);
int main() {
    std::printf("%d\n", answer() + adjust() + extra());
    return 0;
}
