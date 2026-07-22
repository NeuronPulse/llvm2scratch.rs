; ModuleID = '/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_recursion.c'
source_filename = "/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_recursion.c"
target datalayout = "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-i128:128-f64:32:64-f80:32-n8:16:32-S128"
target triple = "i386-pc-linux-gnu"

; Function Attrs: nofree nosync nounwind memory(none) uwtable
define dso_local i32 @fib(i32 noundef %0) local_unnamed_addr #0 {
  %2 = icmp slt i32 %0, 2
  br i1 %2, label %11, label %3

3:                                                ; preds = %1, %3
  %4 = phi i32 [ %8, %3 ], [ %0, %1 ]
  %5 = phi i32 [ %9, %3 ], [ 0, %1 ]
  %6 = add nsw i32 %4, -1
  %7 = tail call i32 @fib(i32 noundef %6)
  %8 = add nsw i32 %4, -2
  %9 = add nsw i32 %7, %5
  %10 = icmp ult i32 %4, 4
  br i1 %10, label %11, label %3

11:                                               ; preds = %3, %1
  %12 = phi i32 [ 0, %1 ], [ %9, %3 ]
  %13 = phi i32 [ %0, %1 ], [ %8, %3 ]
  %14 = add nsw i32 %13, %12
  ret i32 %14
}

; Function Attrs: nofree norecurse nosync nounwind memory(none) uwtable
define dso_local range(i32 1, -2147483648) i32 @factorial(i32 noundef %0) local_unnamed_addr #1 {
  %2 = icmp slt i32 %0, 2
  br i1 %2, label %45, label %3

3:                                                ; preds = %1
  %4 = add nsw i32 %0, -1
  %5 = add nsw i32 %0, -2
  %6 = and i32 %4, 7
  %7 = icmp ult i32 %5, 7
  br i1 %7, label %32, label %8

8:                                                ; preds = %3
  %9 = and i32 %4, -8
  br label %10

10:                                               ; preds = %10, %8
  %11 = phi i32 [ %0, %8 ], [ %28, %10 ]
  %12 = phi i32 [ 1, %8 ], [ %29, %10 ]
  %13 = phi i32 [ 0, %8 ], [ %30, %10 ]
  %14 = add nsw i32 %11, -1
  %15 = mul nuw nsw i32 %11, %12
  %16 = add nsw i32 %11, -2
  %17 = mul nuw nsw i32 %14, %15
  %18 = add nsw i32 %11, -3
  %19 = mul nuw nsw i32 %16, %17
  %20 = add nsw i32 %11, -4
  %21 = mul nuw nsw i32 %18, %19
  %22 = add nsw i32 %11, -5
  %23 = mul nuw nsw i32 %20, %21
  %24 = add nsw i32 %11, -6
  %25 = mul nuw nsw i32 %22, %23
  %26 = add nsw i32 %11, -7
  %27 = mul nuw nsw i32 %24, %25
  %28 = add nsw i32 %11, -8
  %29 = mul nuw nsw i32 %26, %27
  %30 = add i32 %13, 8
  %31 = icmp eq i32 %30, %9
  br i1 %31, label %32, label %10

32:                                               ; preds = %10, %3
  %33 = phi i32 [ poison, %3 ], [ %29, %10 ]
  %34 = phi i32 [ %0, %3 ], [ %28, %10 ]
  %35 = phi i32 [ 1, %3 ], [ %29, %10 ]
  %36 = icmp eq i32 %6, 0
  br i1 %36, label %45, label %37

37:                                               ; preds = %32, %37
  %38 = phi i32 [ %41, %37 ], [ %34, %32 ]
  %39 = phi i32 [ %42, %37 ], [ %35, %32 ]
  %40 = phi i32 [ %43, %37 ], [ 0, %32 ]
  %41 = add nsw i32 %38, -1
  %42 = mul nuw nsw i32 %38, %39
  %43 = add i32 %40, 1
  %44 = icmp eq i32 %43, %6
  br i1 %44, label %45, label %37, !llvm.loop !6

45:                                               ; preds = %32, %37, %1
  %46 = phi i32 [ 1, %1 ], [ %33, %32 ], [ %42, %37 ]
  ret i32 %46
}

; Function Attrs: nofree nosync nounwind memory(none) uwtable
define dso_local range(i32 -2147483647, -2147483648) i32 @main() local_unnamed_addr #0 {
  %1 = tail call i32 @fib(i32 noundef 10)
  %2 = add nsw i32 %1, 120
  ret i32 %2
}

attributes #0 = { nofree nosync nounwind memory(none) uwtable "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="i686" "target-features"="+cmov,+cx8,+x87" "tune-cpu"="generic" }
attributes #1 = { nofree norecurse nosync nounwind memory(none) uwtable "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="i686" "target-features"="+cmov,+cx8,+x87" "tune-cpu"="generic" }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 1, !"NumRegisterParameters", i32 0}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"PIE Level", i32 2}
!4 = !{i32 7, !"uwtable", i32 2}
!5 = !{!"Debian clang version 19.1.7 (3+b1)"}
!6 = distinct !{!6, !7}
!7 = !{!"llvm.loop.unroll.disable"}
