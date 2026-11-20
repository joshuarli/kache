#include "config.h"
#include "answer.h"
#include <stdio.h>
int main(void) {
    printf("%d\n", answer());
    return answer() != ANSWER;
}
