; ModuleID = '/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_aggregate_return.c'
source_filename = "/home/neuronpulse/llvm2scratch.rs/scripts/../examples/complex/complex_aggregate_return.c"
target datalayout = "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-i128:128-f64:32:64-f80:32-n8:16:32-S128"
target triple = "i386-pc-linux-gnu"

%struct.Pair = type { i32, i32 }

; Function Attrs: noinline nounwind optnone uwtable
define dso_local void @make_pair(ptr dead_on_unwind noalias writable sret(%struct.Pair) align 4 %0, i32 noundef %1, i32 noundef %2) #0 {
  %4 = alloca ptr, align 4
  %5 = alloca i32, align 4
  %6 = alloca i32, align 4
  store ptr %0, ptr %4, align 4
  store i32 %1, ptr %5, align 4
  store i32 %2, ptr %6, align 4
  %7 = load i32, ptr %5, align 4
  %8 = getelementptr inbounds %struct.Pair, ptr %0, i32 0, i32 0
  store i32 %7, ptr %8, align 4
  %9 = load i32, ptr %6, align 4
  %10 = getelementptr inbounds %struct.Pair, ptr %0, i32 0, i32 1
  store i32 %9, ptr %10, align 4
  ret void
}

; Function Attrs: noinline nounwind optnone uwtable
define dso_local i32 @main() #0 {
  %1 = alloca i32, align 4
  %2 = alloca %struct.Pair, align 4
  store i32 0, ptr %1, align 4
  call void @make_pair(ptr dead_on_unwind writable sret(%struct.Pair) align 4 %2, i32 noundef 3, i32 noundef 5)
  %3 = getelementptr inbounds %struct.Pair, ptr %2, i32 0, i32 0
  %4 = load i32, ptr %3, align 4
  %5 = getelementptr inbounds %struct.Pair, ptr %2, i32 0, i32 1
  %6 = load i32, ptr %5, align 4
  %7 = add nsw i32 %4, %6
  ret i32 %7
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
