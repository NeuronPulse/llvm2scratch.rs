; Test i64 (wide integer) arithmetic and bitwise operations.
;
; Exercises the wide-integer lowering paths: values above 2^32 must be split
; across limbs, and and/or/xor may be lowered through lookup tables rather than
; emitted as single Scratch ops. The returned value is kept small (< 2^53) so
; the expectation stays exactly representable as a Scratch double.
;
; Note: `ashr` on i64 is not supported by either implementation (the Python
; reference fails an assert on it), so this fixture stays unsigned. Signed
; division/remainder of negative operands is covered by unsigned_ops_vm.
define i32 @main() {
entry:
  %a = shl i64 1, 40                ; 2^40 = 1099511627776  (needs >32 bits)
  %b = lshr i64 %a, 33              ; 2^7  = 128
  %c = and i64 %b, 255              ; 128
  %d = mul i64 %c, 3                ; 384
  %f = xor i64 %d, 128              ; 256
  %g = sub i64 %f, 128              ; 128
  %r = trunc i64 %g to i32
  ret i32 %r
}
