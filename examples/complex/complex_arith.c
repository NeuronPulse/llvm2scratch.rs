int main(void) {
    int a = 100;
    int b = 7;
    int r = (a + b) * (a - b) + (a / b) + (a % b);
    r = (r << 2) | (r >> 1);
    r = r ^ 0x1234;
    return r & 0xFF;
}
