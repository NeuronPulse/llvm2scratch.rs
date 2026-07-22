declare i32 @llvm.setjmp(ptr)
declare void @llvm.longjmp(ptr, i32)

define i32 @main() {
entry:
  %env = alloca [32 x i8], align 8
  %r = call i32 @llvm.setjmp(ptr %env)
  %is_zero = icmp eq i32 %r, 0
  br i1 %is_zero, label %first, label %second

first:
  call void @llvm.longjmp(ptr %env, i32 42)
  unreachable

second:
  ret i32 %r
}
