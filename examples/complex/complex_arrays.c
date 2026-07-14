int main(void) {
    int arr[16];
    for (int i = 0; i < 16; i++) {
        arr[i] = i * i;
    }
    int sum = 0;
    for (int i = 0; i < 16; i++) {
        sum += arr[i];
    }
    return sum % 256;
}
