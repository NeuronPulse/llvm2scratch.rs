; Rust-only shufflevector test.
; Expected local stack after store: [1, 4, 5, 8]

@glob = global <4 x i32> zeroinitializer, align 16

define void @test_shuffle() {
entry:
  %shuf = shufflevector <4 x i32> <i32 1, i32 2, i32 3, i32 4>, <4 x i32> <i32 5, i32 6, i32 7, i32 8>, <4 x i32> <i32 0, i32 3, i32 4, i32 7>
  store <4 x i32> %shuf, ptr @glob, align 16
  ret void
}
