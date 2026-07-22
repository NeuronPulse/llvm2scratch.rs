; Rust-only setjmp/longjmp local-stack snapshot test.
; Compile with -T scratch3 (longjmp is not supported with branch jump tables in Phase 1).

@jmpbuf = global [4 x i32] zeroinitializer, align 4

define void @callee(ptr %env) {
entry:
  call void @longjmp(ptr %env, i32 42)
  ret void
}

define i32 @main() {
entry:
  %env = alloca [4 x i32], align 4
  %env_ptr = getelementptr inbounds [4 x i32], ptr %env, i32 0, i32 0
  %r = call i32 @setjmp(ptr %env_ptr)
  %is_zero = icmp eq i32 %r, 0
  br i1 %is_zero, label %first, label %second

first:
  %local_var = add i32 123, 0
  call void @callee(ptr %env_ptr)
  ret i32 %local_var

second:
  ret i32 %r
}

declare i32 @setjmp(ptr)
declare void @longjmp(ptr, i32)
