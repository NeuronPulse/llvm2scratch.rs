int classify(int x) {
    if (x < 0) return -1;
    if (x == 0) return 0;
    if (x < 10) return 1;
    if (x < 100) return 2;
    return 3;
}

int main(void) {
    int sum = 0;
    for (int i = -5; i < 150; i++) {
        sum += classify(i);
    }
    return sum;
}
