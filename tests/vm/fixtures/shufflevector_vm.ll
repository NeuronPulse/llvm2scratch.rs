define i32 @main() {
entry:
  %a = insertelement <4 x i32> <i32 1, i32 2, i32 3, i32 4>, i32 5, i32 0
  %b = insertelement <4 x i32> <i32 6, i32 7, i32 8, i32 9>, i32 10, i32 0
  %shuf = shufflevector <4 x i32> %a, <4 x i32> %b, <4 x i32> <i32 0, i32 3, i32 4, i32 7>
  %r = extractelement <4 x i32> %shuf, i32 2
  ret i32 %r
}
