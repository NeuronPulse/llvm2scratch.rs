; Complexity test: iterative factorial.
; Computes 10! = 3628800 and returns it.
; Only the final result is asserted.

define i32 @main() {
entry:
  %n = alloca i32, align 4
  %acc = alloca i32, align 4
  store i32 10, ptr %n, align 4
  store i32 1, ptr %acc, align 4
  br label %loop

loop:
  %i = phi i32 [ 1, %entry ], [ %i.next, %loop ]
  %i.next = add i32 %i, 1
  %n.val = load i32, ptr %n, align 4
  %acc.val = load i32, ptr %acc, align 4
  %prod = mul i32 %acc.val, %i
  store i32 %prod, ptr %acc, align 4
  %cmp = icmp sle i32 %i.next, %n.val
  br i1 %cmp, label %loop, label %exit

exit:
  %result = load i32, ptr %acc, align 4
  ret i32 %result
}
