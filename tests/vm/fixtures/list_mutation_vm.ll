; Test progressive list mutation: write values one at a time to !mem.
; The trace should capture intermediate states where !mem has partial writes.

define i32 @main() {
entry:
  %a = alloca i32, align 4
  %b = alloca i32, align 4
  %c = alloca i32, align 4

  ; Write first value
  store i32 11, ptr %a, align 4
  ; Write second value
  store i32 22, ptr %b, align 4
  ; Write third value
  store i32 33, ptr %c, align 4

  ; Read all back
  %va = load i32, ptr %a, align 4
  %vb = load i32, ptr %b, align 4
  %vc = load i32, ptr %c, align 4

  %s1 = add i32 %va, %vb
  %s2 = add i32 %s1, %vc
  ret i32 %s2
}
