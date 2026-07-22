; Complexity test: nested loops computing a triangular-like sum.
; Computes sum_{i=1..5} sum_{j=1..i} j = 1 + 3 + 6 + 10 + 15 = 35.
; Only the final result is asserted.

define i32 @main() {
entry:
  %total = alloca i32, align 4
  %inner.acc = alloca i32, align 4
  store i32 0, ptr %total, align 4
  br label %outer.loop

outer.loop:
  %i = phi i32 [ 1, %entry ], [ %i.next, %outer.next ]
  %i.next = add i32 %i, 1

  store i32 0, ptr %inner.acc, align 4
  br label %inner.loop

inner.loop:
  %j = phi i32 [ 1, %outer.loop ], [ %j.next, %inner.loop ]
  %j.next = add i32 %j, 1
  %acc = load i32, ptr %inner.acc, align 4
  %acc.next = add i32 %acc, %j
  store i32 %acc.next, ptr %inner.acc, align 4
  %inner.cmp = icmp sle i32 %j.next, %i
  br i1 %inner.cmp, label %inner.loop, label %inner.exit

inner.exit:
  %inner.sum = load i32, ptr %inner.acc, align 4
  %total.val = load i32, ptr %total, align 4
  %total.next = add i32 %total.val, %inner.sum
  store i32 %total.next, ptr %total, align 4
  br label %outer.next

outer.next:
  %outer.cmp = icmp sle i32 %i.next, 5
  br i1 %outer.cmp, label %outer.loop, label %exit

exit:
  %result = load i32, ptr %total, align 4
  ret i32 %result
}
