; Rust-only aggregate load/store test (works when accurate-byte-spacing is disabled).

define i32 @test_aggregate() {
entry:
  %arr = alloca [2 x i32], align 4
  %p0 = getelementptr inbounds [2 x i32], ptr %arr, i32 0, i32 0
  %p1 = getelementptr inbounds [2 x i32], ptr %arr, i32 0, i32 1
  store i32 7, ptr %p0, align 4
  store i32 9, ptr %p1, align 4
  %loaded = load [2 x i32], ptr %arr, align 4
  %a = extractvalue [2 x i32] %loaded, 0
  %b = extractvalue [2 x i32] %loaded, 1
  %sum = add i32 %a, %b
  ret i32 %sum
}
