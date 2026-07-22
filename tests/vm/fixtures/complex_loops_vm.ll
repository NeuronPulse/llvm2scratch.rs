; ModuleID = '/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_loops.c'
source_filename = "/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_loops.c"
target datalayout = "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-i128:128-f64:32:64-f80:32-n8:16:32-S128"
target triple = "i386-pc-linux-gnu"

; Function Attrs: noinline nounwind optnone uwtable
define dso_local i32 @main() #0 {
  %1 = alloca i32, align 4
  %2 = alloca i32, align 4
  %3 = alloca i32, align 4
  %4 = alloca i32, align 4
  %5 = alloca i32, align 4
  store i32 0, ptr %1, align 4
  store i32 0, ptr %2, align 4
  store i32 0, ptr %3, align 4
  br label %6

6:                                                ; preds = %23, %0
  %7 = load i32, ptr %3, align 4
  %8 = icmp slt i32 %7, 10
  br i1 %8, label %9, label %26

9:                                                ; preds = %6
  store i32 0, ptr %4, align 4
  br label %10

10:                                               ; preds = %19, %9
  %11 = load i32, ptr %4, align 4
  %12 = icmp slt i32 %11, 10
  br i1 %12, label %13, label %22

13:                                               ; preds = %10
  %14 = load i32, ptr %3, align 4
  %15 = load i32, ptr %4, align 4
  %16 = mul nsw i32 %14, %15
  %17 = load i32, ptr %2, align 4
  %18 = add nsw i32 %17, %16
  store i32 %18, ptr %2, align 4
  br label %19

19:                                               ; preds = %13
  %20 = load i32, ptr %4, align 4
  %21 = add nsw i32 %20, 1
  store i32 %21, ptr %4, align 4
  br label %10, !llvm.loop !7

22:                                               ; preds = %10
  br label %23

23:                                               ; preds = %22
  %24 = load i32, ptr %3, align 4
  %25 = add nsw i32 %24, 1
  store i32 %25, ptr %3, align 4
  br label %6, !llvm.loop !9

26:                                               ; preds = %6
  store i32 0, ptr %5, align 4
  br label %27

27:                                               ; preds = %30, %26
  %28 = load i32, ptr %5, align 4
  %29 = icmp slt i32 %28, 100
  br i1 %29, label %30, label %36

30:                                               ; preds = %27
  %31 = load i32, ptr %5, align 4
  %32 = load i32, ptr %2, align 4
  %33 = add nsw i32 %32, %31
  store i32 %33, ptr %2, align 4
  %34 = load i32, ptr %5, align 4
  %35 = add nsw i32 %34, 3
  store i32 %35, ptr %5, align 4
  br label %27, !llvm.loop !10

36:                                               ; preds = %27
  %37 = load i32, ptr %2, align 4
  %38 = srem i32 %37, 256
  ret i32 %38
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
!7 = distinct !{!7, !8}
!8 = !{!"llvm.loop.mustprogress"}
!9 = distinct !{!9, !8}
!10 = distinct !{!10, !8}
