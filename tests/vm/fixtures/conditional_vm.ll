; Test conditional branches (if/else) with different paths.
; Verifies that the correct branch is taken and variables are set accordingly.

define i32 @main() {
entry:
  %cmp1 = icmp slt i32 5, 10        ; true
  br i1 %cmp1, label %then1, label %else1

then1:
  %a = add i32 1, 100               ; 101
  br label %merge

else1:
  %a_wrong = add i32 1, 200
  br label %merge

merge:
  %av = phi i32 [ %a, %then1 ], [ %a_wrong, %else1 ]
  %cmp2 = icmp sgt i32 %av, 50      ; true (101 > 50)
  br i1 %cmp2, label %then2, label %else2

then2:
  %b = mul i32 %av, 2               ; 202
  br label %exit

else2:
  %b_wrong = mul i32 %av, 3
  br label %exit

exit:
  %bv = phi i32 [ %b, %then2 ], [ %b_wrong, %else2 ]
  %sum = add i32 %av, %bv           ; 101 + 202 = 303
  ret i32 %sum
}
