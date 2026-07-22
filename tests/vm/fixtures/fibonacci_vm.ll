; Complexity test: iterative Fibonacci.
; Computes fib(10) = 55 and returns it.
; Only the final result is asserted, since loop bodies execute without yields.

define i32 @main() {
entry:
  %a = alloca i32, align 4
  %b = alloca i32, align 4
  store i32 0, ptr %a, align 4
  store i32 1, ptr %b, align 4
  br label %loop

loop:
  %i = phi i32 [ 0, %entry ], [ %i.next, %loop ]
  %i.next = add i32 %i, 1
  %a.val = load i32, ptr %a, align 4
  %b.val = load i32, ptr %b, align 4
  %c = add i32 %a.val, %b.val
  store i32 %b.val, ptr %a, align 4
  store i32 %c, ptr %b, align 4
  %cmp = icmp slt i32 %i.next, 10
  br i1 %cmp, label %loop, label %exit

exit:
  %result = load i32, ptr %a, align 4
  ret i32 %result
}
