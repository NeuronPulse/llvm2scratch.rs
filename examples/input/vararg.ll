; ModuleID = 'examples/input/vararg.c'
source_filename = "examples/input/vararg.c"
target datalayout = "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-i128:128-f64:32:64-f80:32-n8:16:32-S128"
target triple = "i386-pc-linux-gnu"

@.str = private unnamed_addr constant [16 x i8] c"Enter avg (1/3)\00", align 1
@.str.1 = private unnamed_addr constant [16 x i8] c"Enter avg (2/3)\00", align 1
@.str.2 = private unnamed_addr constant [16 x i8] c"Enter avg (3/3)\00", align 1
@.str.3 = private unnamed_addr constant [51 x i8] c"Enter 1 for mean and 2 for median of (1, 2, 3, 10)\00", align 1

; Function Attrs: nofree norecurse nosync nounwind uwtable
define dso_local double @mean(i32 noundef %0, ...) local_unnamed_addr #0 {
  %2 = alloca ptr, align 4
  %3 = alloca ptr, align 4
  call void @llvm.lifetime.start.p0(i64 4, ptr nonnull %2) #6
  call void @llvm.lifetime.start.p0(i64 4, ptr nonnull %3) #6
  call void @llvm.va_start.p0(ptr nonnull %2)
  call void @llvm.va_copy.p0(ptr nonnull %3, ptr nonnull %2)
  %4 = load ptr, ptr %2, align 4
  %5 = icmp eq i32 %0, 0
  br i1 %5, label %10, label %6

6:                                                ; preds = %1
  %7 = shl i32 %0, 3
  br label %14

8:                                                ; preds = %14
  %9 = getelementptr i8, ptr %4, i32 %7
  store ptr %9, ptr %2, align 4
  br label %10

10:                                               ; preds = %8, %1
  %11 = phi double [ %20, %8 ], [ 0.000000e+00, %1 ]
  call void @llvm.va_end.p0(ptr %2)
  call void @llvm.va_end.p0(ptr %3)
  %12 = uitofp i32 %0 to double
  %13 = fdiv double %11, %12
  call void @llvm.lifetime.end.p0(i64 4, ptr nonnull %3) #6
  call void @llvm.lifetime.end.p0(i64 4, ptr nonnull %2) #6
  ret double %13

14:                                               ; preds = %6, %14
  %15 = phi i32 [ %21, %14 ], [ 0, %6 ]
  %16 = phi double [ %20, %14 ], [ 0.000000e+00, %6 ]
  %17 = phi ptr [ %18, %14 ], [ %4, %6 ]
  %18 = getelementptr inbounds i8, ptr %17, i32 8
  %19 = load double, ptr %17, align 4, !tbaa !6
  %20 = fadd double %16, %19
  %21 = add nuw nsw i32 %15, 1
  %22 = icmp eq i32 %21, %0
  br i1 %22, label %8, label %14, !llvm.loop !10
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.start.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn
declare void @llvm.va_start.p0(ptr) #2

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn
declare void @llvm.va_copy.p0(ptr, ptr) #2

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.end.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn
declare void @llvm.va_end.p0(ptr) #2

; Function Attrs: mustprogress nofree norecurse nosync nounwind willreturn uwtable
define dso_local double @median(i32 noundef %0, ...) local_unnamed_addr #3 {
  %2 = alloca ptr, align 4
  %3 = icmp eq i32 %0, 0
  br i1 %3, label %27, label %4

4:                                                ; preds = %1
  call void @llvm.lifetime.start.p0(i64 4, ptr nonnull %2) #6
  call void @llvm.va_start.p0(ptr nonnull %2)
  %5 = add i32 %0, 1
  %6 = and i32 %0, 1
  %7 = icmp eq i32 %6, 0
  %8 = icmp ult i32 %5, 2
  br i1 %8, label %17, label %9

9:                                                ; preds = %4
  %10 = load ptr, ptr %2, align 4
  %11 = shl i32 %5, 2
  %12 = and i32 %11, -8
  %13 = getelementptr i8, ptr %10, i32 %12
  %14 = getelementptr i8, ptr %13, i32 -8
  %15 = getelementptr i8, ptr %10, i32 %12
  store ptr %15, ptr %2, align 4
  %16 = load double, ptr %14, align 4, !tbaa !6
  br label %17

17:                                               ; preds = %9, %4
  %18 = phi double [ %16, %9 ], [ undef, %4 ]
  br i1 %7, label %19, label %25

19:                                               ; preds = %17
  %20 = load ptr, ptr %2, align 4
  %21 = getelementptr inbounds i8, ptr %20, i32 8
  store ptr %21, ptr %2, align 4
  %22 = load double, ptr %20, align 4, !tbaa !6
  %23 = fadd double %18, %22
  %24 = fmul double %23, 5.000000e-01
  br label %25

25:                                               ; preds = %19, %17
  %26 = phi double [ %24, %19 ], [ %18, %17 ]
  call void @llvm.lifetime.end.p0(i64 4, ptr nonnull %2) #6
  br label %27

27:                                               ; preds = %1, %25
  %28 = phi double [ %26, %25 ], [ 0x7FF8000000000000, %1 ]
  ret double %28
}

; Function Attrs: nounwind uwtable
define dso_local noundef i32 @main() local_unnamed_addr #4 {
  %1 = alloca double, align 8
  %2 = alloca double, align 8
  %3 = alloca double, align 8
  call void @llvm.lifetime.start.p0(i64 8, ptr nonnull %1) #6
  call void @llvm.lifetime.start.p0(i64 8, ptr nonnull %2) #6
  call void @llvm.lifetime.start.p0(i64 8, ptr nonnull %3) #6
  %4 = call i32 @SB3_ask_dbl(ptr noundef nonnull %1, ptr noundef nonnull @.str) #6
  %5 = call i32 @SB3_ask_dbl(ptr noundef nonnull %2, ptr noundef nonnull @.str.1) #6
  %6 = call i32 @SB3_ask_dbl(ptr noundef nonnull %3, ptr noundef nonnull @.str.2) #6
  %7 = load double, ptr %1, align 8, !tbaa !6
  %8 = load double, ptr %2, align 8, !tbaa !6
  %9 = load double, ptr %3, align 8, !tbaa !6
  %10 = call double (i32, ...) @mean(i32 noundef 3, double noundef %7, double noundef %8, double noundef %9)
  call void @SB3_say_dbl(double noundef %10) #6
  call void @SB3_wait(double noundef 1.000000e+00) #6
  %11 = call i32 @SB3_ask_dbl(ptr noundef nonnull %1, ptr noundef nonnull @.str.3) #6
  %12 = load double, ptr %1, align 8, !tbaa !6
  %13 = fcmp oeq double %12, 1.000000e+00
  %14 = select i1 %13, ptr @mean, ptr @median
  %15 = call double (i32, ...) %14(i32 noundef 4, double noundef 1.000000e+00, double noundef 2.000000e+00, double noundef 3.000000e+00, double noundef 1.000000e+01) #6, !callees !13
  call void @SB3_say_dbl(double noundef %15) #6
  call void @SB3_wait(double noundef 1.000000e+00) #6
  %16 = call double (i32, ...) %14(i32 noundef 0) #6, !callees !13
  call void @SB3_say_dbl(double noundef %16) #6
  call void @llvm.lifetime.end.p0(i64 8, ptr nonnull %3) #6
  call void @llvm.lifetime.end.p0(i64 8, ptr nonnull %2) #6
  call void @llvm.lifetime.end.p0(i64 8, ptr nonnull %1) #6
  ret i32 0
}

declare i32 @SB3_ask_dbl(ptr noundef, ptr noundef) local_unnamed_addr #5

declare void @SB3_say_dbl(double noundef) local_unnamed_addr #5

declare void @SB3_wait(double noundef) local_unnamed_addr #5

attributes #0 = { nofree norecurse nosync nounwind uwtable "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="i686" "target-features"="+cmov,+cx8,+x87" "tune-cpu"="generic" }
attributes #1 = { mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }
attributes #2 = { mustprogress nocallback nofree nosync nounwind willreturn }
attributes #3 = { mustprogress nofree norecurse nosync nounwind willreturn uwtable "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="i686" "target-features"="+cmov,+cx8,+x87" "tune-cpu"="generic" }
attributes #4 = { nounwind uwtable "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="i686" "target-features"="+cmov,+cx8,+x87" "tune-cpu"="generic" }
attributes #5 = { "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="i686" "target-features"="+cmov,+cx8,+x87" "tune-cpu"="generic" }
attributes #6 = { nounwind }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 1, !"NumRegisterParameters", i32 0}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"PIE Level", i32 2}
!4 = !{i32 7, !"uwtable", i32 2}
!5 = !{!"Debian clang version 19.1.7 (3+b1)"}
!6 = !{!7, !7, i64 0}
!7 = !{!"double", !8, i64 0}
!8 = !{!"omnipotent char", !9, i64 0}
!9 = !{!"Simple C/C++ TBAA"}
!10 = distinct !{!10, !11, !12}
!11 = !{!"llvm.loop.mustprogress"}
!12 = !{!"llvm.loop.unroll.disable"}
!13 = !{ptr @mean, ptr @median}
