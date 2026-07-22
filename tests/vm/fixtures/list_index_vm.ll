; Test writing multiple values to memory (the !mem list) via alloca pointers,
; then reading them back and computing with them.
; Uses --no-accurate-byte-spacing so each i32 = 1 cell.

define i32 @main() {
entry:
  ; Allocate an array of 4 i32s on the stack
  %arr = alloca [4 x i32], align 4

  ; Get pointers to each element
  %p0 = getelementptr [4 x i32], ptr %arr, i32 0, i32 0
  %p1 = getelementptr [4 x i32], ptr %arr, i32 0, i32 1
  %p2 = getelementptr [4 x i32], ptr %arr, i32 0, i32 2
  %p3 = getelementptr [4 x i32], ptr %arr, i32 0, i32 3

  ; Write values
  store i32 10, ptr %p0, align 4
  store i32 20, ptr %p1, align 4
  store i32 30, ptr %p2, align 4
  store i32 40, ptr %p3, align 4

  ; Read them back
  %v0 = load i32, ptr %p0, align 4
  %v1 = load i32, ptr %p1, align 4
  %v2 = load i32, ptr %p2, align 4
  %v3 = load i32, ptr %p3, align 4

  ; Sum: 10 + 20 + 30 + 40 = 100
  %s1 = add i32 %v0, %v1
  %s2 = add i32 %s1, %v2
  %s3 = add i32 %s2, %v3

  ret i32 %s3
}
