const char s[] = "é";
const __WCHAR_TYPE__ w[] = L"é";
int main(void) { return sizeof(w) + sizeof(s); }
