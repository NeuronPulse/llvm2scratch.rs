typedef struct __jmp_buf_tag {
    int __jb[8];
} jmp_buf[1];

int setjmp(jmp_buf env);
void longjmp(jmp_buf env, int val);

int main(void) {
    jmp_buf env;
    int r = setjmp(env);
    if (r == 0) {
        longjmp(env, 42);
    }
    return r;
}
