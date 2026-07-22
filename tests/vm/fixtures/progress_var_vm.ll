define i32 @main() {
entry:
  br label %loop

loop:
  %i = phi i32 [ 0, %entry ], [ %next, %loop ]
  %next = add i32 %i, 1
  %cmp = icmp slt i32 %next, 10
  br i1 %cmp, label %loop, label %exit

exit:
  ret i32 %i
}
