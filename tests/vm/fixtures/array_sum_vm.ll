; Complexity test: sum an array of integers stored on the stack.
; Array values are [3, 7, 2, 9, 4, 1, 8, 6]; sum = 40.
; Uses --no-accurate-byte-spacing so each i32 occupies one !mem cell.
; Only the final return value is asserted.

define i32 @main() {
entry:
  %arr = alloca [8 x i32], align 4

  ; Store array elements.
  %p0 = getelementptr [8 x i32], ptr %arr, i32 0, i32 0
  %p1 = getelementptr [8 x i32], ptr %arr, i32 0, i32 1
  %p2 = getelementptr [8 x i32], ptr %arr, i32 0, i32 2
  %p3 = getelementptr [8 x i32], ptr %arr, i32 0, i32 3
  %p4 = getelementptr [8 x i32], ptr %arr, i32 0, i32 4
  %p5 = getelementptr [8 x i32], ptr %arr, i32 0, i32 5
  %p6 = getelementptr [8 x i32], ptr %arr, i32 0, i32 6
  %p7 = getelementptr [8 x i32], ptr %arr, i32 0, i32 7

  store i32 3, ptr %p0, align 4
  store i32 7, ptr %p1, align 4
  store i32 2, ptr %p2, align 4
  store i32 9, ptr %p3, align 4
  store i32 4, ptr %p4, align 4
  store i32 1, ptr %p5, align 4
  store i32 8, ptr %p6, align 4
  store i32 6, ptr %p7, align 4

  %sum = alloca i32, align 4
  store i32 0, ptr %sum, align 4
  br label %loop

loop:
  %i = phi i32 [ 0, %entry ], [ %i.next, %loop ]
  %i.next = add i32 %i, 1

  ; GEP based on loop index.
  %ptr = getelementptr [8 x i32], ptr %arr, i32 0, i32 %i
  %val = load i32, ptr %ptr, align 4
  %acc = load i32, ptr %sum, align 4
  %new_acc = add i32 %acc, %val
  store i32 %new_acc, ptr %sum, align 4

  %cmp = icmp slt i32 %i.next, 8
  br i1 %cmp, label %loop, label %exit

exit:
  %result = load i32, ptr %sum, align 4
  ret i32 %result
}
