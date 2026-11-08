using System;
using System.Runtime.CompilerServices;

public delegate void RefAction(ref int value);
public class Finalizable {
    public static int Count;
    ~Finalizable() { Count++; }
}
public class Program {
    static int staticValue;
    [MethodImpl(MethodImplOptions.NoInlining)]
    static void Create() { new Finalizable(); }
    static void First(ref int value) {
        Create();
        for (int i = 0; i < 5 && Finalizable.Count == value; ++i) {
            GC.Collect();
            GC.WaitForPendingFinalizers();
        }
        value = Finalizable.Count;
    }
    static void Second(ref int value) { value += 20; }
    public static int Main() {
        RefAction chain = First;
        chain += Second;
        int local = 0;
        chain(ref local);
        staticValue = Finalizable.Count;
        chain(ref staticValue);
        Console.WriteLine(local);
        Console.WriteLine(staticValue);
        return local == 21 && staticValue == 22 ? 42 : 1;
    }
}
