; Complexity test: Euclidean algorithm for GCD.
; Computes gcd(48, 18) = 6.
; Uses a loop with remainder operations (subtraction-based to avoid unsupported srem).
; Only the final result is asserted.

define i32 @main() {
entry:
  %a = alloca i32, align 4
  %b = alloca i32, align 4
  store i32 48, ptr %a, align 4
  store i32 18, ptr %b, align 4
  br label %loop

loop:
  %av = phi i32 [ 48, %entry ], [ %a.next, %next ]
  %bv = phi i32 [ 18, %entry ], [ %bv.next, %next ]

  ; Compare a and b.
  %cmp = icmp eq i32 %av, %bv
  br i1 %cmp, label %exit, label %body

body:
  %gt = icmp sgt i32 %av, %bv
  br i1 %gt, label %a_gt_b, label %b_gt_a

a_gt_b:
  %diff1 = sub i32 %av, %bv
  br label %next

b_gt_a:
  %diff2 = sub i32 %bv, %av
  br label %next

next:
  %a.next = phi i32 [ %diff1, %a_gt_b ], [ %av, %b_gt_a ]
  %bv.next = phi i32 [ %bv, %a_gt_b ], [ %diff2, %b_gt_a ]
  br label %loop

exit:
  ret i32 %av
}
