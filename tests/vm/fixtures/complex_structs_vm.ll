; ModuleID = '/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_structs.c'
source_filename = "/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_structs.c"
target datalayout = "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-i128:128-f64:32:64-f80:32-n8:16:32-S128"
target triple = "i386-pc-linux-gnu"

%struct.Rect = type { %struct.Point, %struct.Point }
%struct.Point = type { i32, i32 }

; Function Attrs: noinline nounwind optnone uwtable
define dso_local i32 @main() #0 {
  %1 = alloca i32, align 4
  %2 = alloca %struct.Rect, align 4
  %3 = alloca i32, align 4
  store i32 0, ptr %1, align 4
  %4 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 0
  %5 = getelementptr inbounds %struct.Point, ptr %4, i32 0, i32 0
  store i32 1, ptr %5, align 4
  %6 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 0
  %7 = getelementptr inbounds %struct.Point, ptr %6, i32 0, i32 1
  store i32 2, ptr %7, align 4
  %8 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 1
  %9 = getelementptr inbounds %struct.Point, ptr %8, i32 0, i32 0
  store i32 4, ptr %9, align 4
  %10 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 1
  %11 = getelementptr inbounds %struct.Point, ptr %10, i32 0, i32 1
  store i32 8, ptr %11, align 4
  %12 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 1
  %13 = getelementptr inbounds %struct.Point, ptr %12, i32 0, i32 0
  %14 = load i32, ptr %13, align 4
  %15 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 0
  %16 = getelementptr inbounds %struct.Point, ptr %15, i32 0, i32 0
  %17 = load i32, ptr %16, align 4
  %18 = sub nsw i32 %14, %17
  %19 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 1
  %20 = getelementptr inbounds %struct.Point, ptr %19, i32 0, i32 1
  %21 = load i32, ptr %20, align 4
  %22 = getelementptr inbounds %struct.Rect, ptr %2, i32 0, i32 0
  %23 = getelementptr inbounds %struct.Point, ptr %22, i32 0, i32 1
  %24 = load i32, ptr %23, align 4
  %25 = sub nsw i32 %21, %24
  %26 = mul nsw i32 %18, %25
  store i32 %26, ptr %3, align 4
  %27 = load i32, ptr %3, align 4
  ret i32 %27
}

attributes #0 = { noinline nounwind optnone uwtable "frame-pointer"="all" "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="i686" "target-features"="+cmov,+cx8,+x87" "tune-cpu"="generic" }

!llvm.module.flags = !{!0, !1, !2, !3, !4, !5}
!llvm.ident = !{!6}

!0 = !{i32 1, !"NumRegisterParameters", i32 0}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"PIE Level", i32 2}
!4 = !{i32 7, !"uwtable", i32 2}
!5 = !{i32 7, !"frame-pointer", i32 2}
!6 = !{!"Debian clang version 19.1.7 (3+b1)"}
