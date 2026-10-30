; Test floating point arithmetic, float comparisons and int<->float conversion.
;
; The generated project uses Scratch doubles, so this checks that float ops
; round-trip through them and that fptosi truncates toward zero while frem
; keeps the fractional part. No other fixture covers floats.
define i32 @main() {
entry:
  %a = fadd double 1.5, 2.25        ; 3.75
  %b = fmul double %a, 4.0          ; 15.0
  %c = fdiv double %b, 2.0          ; 7.5
  %ipart = fptosi double %c to i32  ; 7 (truncates toward zero)
  %frac = frem double %c, 2.0       ; 1.5
  %neg = fsub double 0.0, %frac     ; -1.5
  %abs1 = fadd double %neg, 3.0     ; 1.5
  %cmp = fcmp ogt double %abs1, 1.0 ; true -> 1
  %z = zext i1 %cmp to i32
  %res = add i32 %ipart, %z         ; 8
  ret i32 %res
}
