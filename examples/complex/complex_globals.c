static int counter = 0;
int g_arr[5] = {1, 2, 3, 4, 5};

int add_to_counter(int x) {
    counter += x;
    return counter;
}

int main(void) {
    int sum = 0;
    for (int i = 0; i < 5; i++) {
        sum += add_to_counter(g_arr[i]);
    }
    return sum;
}
