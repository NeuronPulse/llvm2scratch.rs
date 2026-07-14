struct Pair {
    int first;
    int second;
};

struct Pair make_pair(int a, int b) {
    struct Pair p;
    p.first = a;
    p.second = b;
    return p;
}

int main(void) {
    struct Pair p = make_pair(3, 5);
    return p.first + p.second;
}
