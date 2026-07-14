float global_f = 1.5f;

double compute(double a, double b) {
    double sum = a + b;
    double diff = a - b;
    double prod = a * b;
    double quot = a / b;
    double rem = (float)(a - (int)(a / b) * b);
    int eq = (a == b);
    int ne = (a != b);
    int lt = (a < b);
    int le = (a <= b);
    int gt = (a > b);
    int ge = (a >= b);
    return sum + diff + prod + quot + rem + eq + ne + lt + le + gt + ge + global_f;
}

int main(void) {
    double x = 3.14;
    double y = 2.0;
    double r = compute(x, y);
    return (int)r;
}
