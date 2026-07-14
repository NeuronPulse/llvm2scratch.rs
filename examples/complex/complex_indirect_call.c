int add(int a, int b) { return a + b; }
int sub(int a, int b) { return a - b; }
int mul(int a, int b) { return a * b; }

typedef int (*binop_t)(int, int);

int main(void) {
    binop_t ops[3] = {add, sub, mul};
    int sum = 0;
    for (int i = 0; i < 3; i++) {
        sum += ops[i](i + 1, i + 2);
    }
    return sum;
}
