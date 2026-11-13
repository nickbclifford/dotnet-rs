using System;
using System.Runtime.CompilerServices;
using System.Threading;

public class Program {
    class Node { public Node Next; public int Value; }
    class Finalizable {
        public Node Child;
        ~Finalizable() { saved = this; }
    }
    static Finalizable saved;
    static Node foreign;
    static WeakReference longWeak;
    static volatile int command;
    static volatile bool ready;

    [MethodImpl(MethodImplOptions.NoInlining)]
    static void MakeForeign() { foreign = new Node { Value = 42 }; }
    [MethodImpl(MethodImplOptions.NoInlining)]
    static void MakeFinalizable() {
        Finalizable f = new Finalizable { Child = foreign };
        foreign = null;
        longWeak = new WeakReference(f, true);
    }
    [MethodImpl(MethodImplOptions.NoInlining)]
    static void MakeLongList() {
        Node n = null;
        for (int i = 0; i < 10000; i++) n = new Node { Next = n, Value = i };
        Finalizable f = new Finalizable { Child = n };
        longWeak = new WeakReference(f, true);
    }
    static void Worker() {
        MakeForeign(); ready = true;
        while (command != 1) Thread.Yield();
        GC.Collect(); command = 2;
        while (command != 3) Thread.Yield();
        MakeFinalizable(); command = 4;
        while (command != 5) Thread.Yield();
        GC.Collect(); command = 6;
        while (command != 7) Thread.Yield();
        MakeLongList(); GC.Collect(); command = 8;
    }
    static void Wait(int value) { while (command != value) Thread.Yield(); }
    static bool Check() {
        GC.WaitForPendingFinalizers();
        return saved != null && saved.Child != null && saved.Child.Value == 42 && longWeak.Target == saved;
    }
    static bool CheckList() {
        if (saved == null || longWeak.Target != saved) return false;
        int count = 0;
        for (Node n = saved.Child; n != null; n = n.Next) count++;
        return count == 10000;
    }
    public static int Main() {
        Thread t = new Thread(Worker); t.IsBackground = true; t.Start();
        while (!ready) Thread.Yield();
        MakeFinalizable(); command = 1; Wait(2);
        if (!Check()) return 1;
        saved = null;
        MakeForeign(); command = 3; Wait(4);
        GC.Collect();
        if (!Check()) return 2;
        saved = null;
        MakeForeign(); MakeFinalizable(); command = 5; Wait(6);
        if (!Check()) return 3;
        saved = null;
        MakeLongList(); GC.Collect(); GC.WaitForPendingFinalizers();
        if (!CheckList()) return 4;
        saved = null; command = 7; Wait(8);
        GC.WaitForPendingFinalizers();
        bool ok = CheckList(); t.Join();
        return ok ? 42 : 5;
    }
}
