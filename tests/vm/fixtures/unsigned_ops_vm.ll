; Test signed-vs-unsigned semantics.
;
; A very easy place for a reimplementation to go wrong is treating everything as
; signed: compare/divide/remainder all change meaning once the operands are read
; as unsigned. `-1` is 4294967289 when read as u32, so it is NOT <u 1, and
; `-8 /u 4` is 1073741822 rather than -2. Each result is weighted differently so
; a single wrong operation changes the expected total.
define i32 @main() {
entry:
  %ult = icmp ult i32 -1, 1         ; false -> 0
  %ugt = icmp ugt i32 -1, 1         ; true  -> 1
  %slt = icmp slt i32 -1, 1         ; true  -> 1
  %q = sdiv i32 -7, 2               ; -3 (truncates toward zero)
  %r = srem i32 -7, 2               ; -1
  %uq = udiv i32 -8, 4              ; 1073741822
  %ur = urem i32 -8, 4              ; 0
  %z1 = zext i1 %ult to i32
  %z2 = zext i1 %ugt to i32
  %z3 = zext i1 %slt to i32
  %acc = add i32 %z1, %z2
  %acc1 = add i32 %acc, %z3
  %acc2 = add i32 %acc1, %uq
  %acc3 = sub i32 %acc2, %r         ; -= -1  -> +1
  %res = sub i32 %acc3, %q          ; -= -3  -> +3
  ret i32 %res
}
