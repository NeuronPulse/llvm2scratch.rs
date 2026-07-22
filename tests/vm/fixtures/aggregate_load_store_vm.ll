%MyStruct = type { i32, i32 }

define i32 @main() {
entry:
  %ptr = alloca %MyStruct, align 8
  %s0 = insertvalue %MyStruct undef, i32 7, 0
  %s1 = insertvalue %MyStruct %s0, i32 9, 1
  store %MyStruct %s1, ptr %ptr, align 8
  %loaded = load %MyStruct, ptr %ptr, align 8
  %a = extractvalue %MyStruct %loaded, 0
  %b = extractvalue %MyStruct %loaded, 1
  %sum = add i32 %a, %b
  ret i32 %sum
}
