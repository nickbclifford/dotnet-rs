using System;
using System.Runtime.InteropServices;
using System.Threading;

public class Program {
    private static volatile bool ready;
    private static volatile bool release;
    private static GCHandle strong;
    private static GCHandle pinned;
    private static GCHandle pinnedAgain;
    private static WeakReference weak;
    private static object first;
    private static object second;

    private static void CreateHandles() {
        first = new object();
        strong = GCHandle.Alloc(first);
        weak = new WeakReference(first);
        byte[] bytes = new byte[8];
        pinned = GCHandle.Alloc(bytes, GCHandleType.Pinned);
        pinnedAgain = GCHandle.Alloc(bytes, GCHandleType.Pinned);
        ready = true;
        while (!release) Thread.Yield();
    }

    private static int ExerciseHandles() {
        Thread creator = new Thread(CreateHandles);
        creator.Start();
        try {
            while (!ready) Thread.Yield();
            GC.Collect();
            if (strong.Target != first || weak.Target != first) return 1;
            IntPtr address = pinned.AddrOfPinnedObject();
            if (address == IntPtr.Zero || pinnedAgain.AddrOfPinnedObject() != address) return 6;
            pinned.Free();

            second = new object();
            strong.Target = second;
            weak.Target = second;
            GC.Collect();
            if (strong.Target != second || weak.Target != second) return 2;

            release = true;
            if (!creator.Join(5000)) return 3;
            GC.Collect();
            if (strong.Target != second || weak.Target != second) return 4;
            if (pinnedAgain.AddrOfPinnedObject() != address) return 7;
            pinnedAgain.Free();
            strong.Target = first;
            weak.Target = first;
            GC.Collect();
            if (strong.Target != first || weak.Target != first) return 5;
            first = null;
            second = null;
            GC.Collect();
            if (strong.Target == null || weak.Target != strong.Target) return 8;
            strong.Free();
            return 42;
        } finally {
            release = true;
            creator.Join();
        }
    }

    public static int Main() {
        int result = ExerciseHandles();
        if (result != 42) return result;
        GC.Collect();
        GC.Collect();
        return weak.Target == null ? 42 : 9;
    }
}
