int main(void) {
    int s = 0;
    for (int i = 0; i < 10; i++) {
        for (int j = 0; j < 10; j++) {
            s += i * j;
        }
    }
    int k = 0;
    while (k < 100) {
        s += k;
        k += 3;
    }
    return s % 256;
}
