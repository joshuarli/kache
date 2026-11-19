int f(int x) {
  switch (x) {
  case 1:
    x++;
    /* fallthrough */
  case 2:
    return x;
  }
  return 0;
}
