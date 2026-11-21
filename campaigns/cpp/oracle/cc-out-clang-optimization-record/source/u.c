extern int helper(int);
int u(int *p, int n) {
    int sum = 0; /* before */
    for (int i = 0; i < n; ++i) sum += p[i];
    return helper(sum);
}
