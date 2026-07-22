; Test store/load to memory (the !mem list) and verify list contents.
; Uses --no-accurate-byte-spacing so each i32 occupies exactly 1 cell.

define i32 @main() {
entry:
  %a = alloca i32, align 4
  %b = alloca i32, align 4
  %c = alloca i32, align 4
  store i32 111, ptr %a, align 4
  store i32 222, ptr %b, align 4
  store i32 333, ptr %c, align 4
  %va = load i32, ptr %a, align 4
  %vb = load i32, ptr %b, align 4
  %vc = load i32, ptr %c, align 4
  %sum = add i32 %va, %vb
  %total = add i32 %sum, %vc
  ret i32 %total
}
