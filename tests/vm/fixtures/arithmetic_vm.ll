; Test arithmetic operations on local variables.
; Verifies intermediate variable values and the final return value.

define i32 @main() {
entry:
  %a = add i32 10, 5        ; 15
  %b = sub i32 %a, 3        ; 12
  %c = mul i32 %b, 2        ; 24
  %d = sdiv i32 %c, 4       ; 6
  %e = srem i32 %c, 5       ; 4
  %f = and i32 %c, 16       ; 16
  %g = or i32 %e, 1         ; 5
  %h = xor i32 %g, 5        ; 0
  %i = shl i32 %d, 3        ; 48
  %sum = add i32 %i, %h     ; 48
  ret i32 %sum
}
