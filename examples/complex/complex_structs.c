struct Point {
    int x;
    int y;
};

struct Rect {
    struct Point a;
    struct Point b;
};

int main(void) {
    struct Rect r;
    r.a.x = 1;
    r.a.y = 2;
    r.b.x = 4;
    r.b.y = 8;
    int area = (r.b.x - r.a.x) * (r.b.y - r.a.y);
    return area;
}
