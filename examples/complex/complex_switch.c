int classify_char(char c) {
    switch (c) {
        case 'a': return 1;
        case 'b': return 2;
        case 'c': return 3;
        case 'd': return 4;
        case 'e': return 5;
        case 'f': return 6;
        default: return 0;
    }
}

int main(void) {
    char str[] = "abcdeffedcba";
    int sum = 0;
    for (int i = 0; str[i]; i++) {
        sum += classify_char(str[i]);
    }
    return sum % 256;
}
