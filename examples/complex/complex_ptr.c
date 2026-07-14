void swap(int *a, int *b) {
    int t = *a;
    *a = *b;
    *b = t;
}

int main(void) {
    int x = 5, y = 10;
    swap(&x, &y);
    return x + y;
}
